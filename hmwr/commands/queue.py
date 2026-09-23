"""実験のキュー。GitHubのIssueを順に取り、specを無人で実行する（ADR-0209）。

キューと状態はIssueのラベルが持つ。`experiment` と `queued` の付いたIssueを
古い順に取り、`running`、`done` か `failed` へ付け替える。順番の入れ替えと
取り消しは、オーナーがIssueから行える。

開発機の資源は1つなので、同時に走るのは1実験だけにする。専用のworktreeから
走らせる（`hmwr queue install`）。開発中の作業ツリーで走らせると、ブランチの
切り替えが実験のコードを変えてしまう。
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import plistlib
import re
import shutil
import subprocess
import sys
from pathlib import Path

from .. import config, paths, proc
from . import exp

EXPERIMENT, QUEUED, RUNNING, DONE, FAILED = "experiment", "queued", "running", "done", "failed"
LABELS = {
    EXPERIMENT: "実験キューの対象（ADR-0209）",
    QUEUED: "実行待ち",
    RUNNING: "開発機で実行中",
    DONE: "全ステップが完了",
    FAILED: "失敗して止まった",
}
SPEC_RE = re.compile(r"experiments/([A-Za-z0-9][A-Za-z0-9._-]*)\.toml")
AGENT = "com.kaz9120.himawari-queue"
INTERVAL = 300
LOG_TAIL = 30
# 結果の記録に使うclaude。PATHのshimは端末の環境に依存するので、実体を先に探す
CLAUDE_CANDIDATES = (Path.home() / ".local" / "bin" / "claude",)
RECORD_TIMEOUT = 40 * 60


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("queue", help="実験のキューを回す")
    ss = p.add_subparsers(dest="sub", metavar="<操作>")

    t = ss.add_parser(
        "run",
        help="コードをorigin/mainへ揃えてから、キューを1回見る",
        description="launchdが定期的に呼ぶ入口。専用のworktreeをorigin/mainへ揃えてから "
        "tick を呼ぶ。開発中の作業ツリーでは走らない。",
    )
    t.set_defaults(func=run)

    t = ss.add_parser(
        "tick",
        help="キューを1回見て、実験を1件実行する",
        description="実行中のIssueがあれば続きから、無ければ待ちの先頭を取る。"
        "専用のworktreeでは、コミットが進んでいたらツールを作り直してから走らせる。"
        "終わるまで戻らない。一時停止中と、別の実行が走っている間は何もしない。",
    )
    t.set_defaults(func=tick)

    t = ss.add_parser("status", help="キューと一時停止の状態を出す")
    t.set_defaults(func=status)

    t = ss.add_parser(
        "pause",
        help="次のステップを始めさせない",
        description="開発機を空けたいときに使う（NPSの計測、対話セッションでのSPRT）。"
        "走っているステップは止めない。ステップの切れ目で止まる。",
    )
    t.set_defaults(func=pause)

    t = ss.add_parser("resume", help="一時停止を解く")
    t.set_defaults(func=resume)

    t = ss.add_parser(
        "install",
        help="専用のworktreeとlaunchdの常駐を用意する",
        description="<リポジトリ>-runner にworktreeを作り、data/ と学習の成果物と"
        "開始局面集をリンクでつなぐ。もう一度実行すると、足りないリンクを足して"
        "常駐を入れ直す。launchdが5分おきに hmwr queue run を呼ぶ。",
    )
    t.set_defaults(func=install)

    t = ss.add_parser("uninstall", help="launchdの常駐を外す（worktreeは残す）")
    t.set_defaults(func=uninstall)


# --- GitHub ------------------------------------------------------------


def gh(*args: str) -> str:
    """ghを呼ぶ。届かないときはFailを投げ、次の回でやり直す。"""
    result = subprocess.run(
        ["gh", *args], cwd=str(paths.REPO), capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        raise proc.Fail(f"ghが失敗した: gh {' '.join(args)}\n{result.stderr.strip()}")
    return result.stdout


def issues(state: str) -> list[dict]:
    """状態のラベルが付いた実験のIssueを、古い順に返す。"""
    out = gh(
        "issue", "list", "--state", "open", "--limit", "50",
        "--label", EXPERIMENT, "--label", state,
        "--json", "number,title,body,author,createdAt",
    )  # fmt: skip
    return sorted(json.loads(out), key=lambda i: i["createdAt"])


def owner() -> str:
    return json.loads(gh("repo", "view", "--json", "owner"))["owner"]["login"]


def relabel(number: int, *, add: str, remove: str) -> None:
    gh("issue", "edit", str(number), "--add-label", add, "--remove-label", remove)


def comment(number: int, body: str) -> None:
    gh("issue", "comment", str(number), "--body", body)


def spec_name(body: str) -> str | None:
    """Issueの本文から、最初に現れるspecのパスを取る。"""
    m = SPEC_RE.search(body or "")
    return m.group(1) if m else None


# --- 1回ぶんの処理 -----------------------------------------------------


def tick(args: argparse.Namespace) -> int:
    if exp.pause_file().exists():
        print("一時停止中（hmwr queue resume で解除）")
        return proc.OK

    paths.QUEUE.mkdir(parents=True, exist_ok=True)
    with open(paths.QUEUE / "LOCK", "w") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            print("別の実行が走っている。何もしない")
            return proc.OK
        return _tick(dry_run=args.dry_run)


def _tick(*, dry_run: bool) -> int:
    running = issues(RUNNING)
    issue = (running or issues(QUEUED) or [None])[0]
    if issue is None:
        print("待ちの実験はない")
        return proc.OK
    number, resumed = issue["number"], bool(running)
    print(f"#{number} {issue['title']}（{'続きから' if resumed else '新規'}）")
    if dry_run:
        print(f"[dry-run] hmwr exp run {spec_name(issue['body'])}")
        return proc.OK

    state = QUEUED if not resumed else RUNNING
    if issue["author"]["login"] != owner():
        # Issueフォームは誰が出してもラベルが付く。公開リポジトリなので作者を見る
        return _fail(number, state, "作者がリポジトリのオーナーではないので実行しない。")
    name = spec_name(issue["body"])
    if name is None:
        return _fail(number, state, "本文に `experiments/<名前>.toml` のパスがない。")

    if not resumed:
        # 失敗から戻したIssueには failed が残っていることがある
        relabel(number, add=RUNNING, remove=f"{QUEUED},{FAILED}")
        comment(number, f"開発機で実行を始めた。spec: `experiments/{name}.toml`")
    # ツールを作り直すのは、走らせる実験があるときだけにする。待ちが無いのに
    # ビルドが走ると、対話セッションの計測を乱す
    _build_if_moved()

    result = subprocess.run(
        [sys.executable, str(paths.REPO / "bin" / "hmwr"), "exp", "run", name],
        cwd=str(paths.REPO),
        check=False,
    )
    if result.returncode == proc.JUDGE and exp.pause_file().exists():
        print("一時停止で止まった。ラベルは running のまま残す")
        return proc.OK
    if result.returncode != proc.OK:
        return _fail(number, RUNNING, f"終了コード {result.returncode} で止まった。", name)

    relabel(number, add=DONE, remove=RUNNING)
    comment(number, f"全ステップが完了した。\n\n```\n{_progress(name)}\n```")
    _record(name, number)
    return proc.OK


def _claude() -> str | None:
    for path in CLAUDE_CANDIDATES:
        if path.is_file():
            return str(path)
    return shutil.which("claude")


def _record(name: str, number: int) -> None:
    """結果の記録をclaude -pに任せる。出すのはdocsのPRまでで、マージはしない。

    失敗しても実験の完了は変わらないので、Issueへ書き戻して次へ進む。
    """
    claude = _claude()
    if claude is None:
        comment(number, "結果の記録: claude が見つからないので、対話セッションで記録する。")
        return
    prompt = (
        f"recording-experimentスキルを使って、実験 {name}（Issue #{number}）の結果を"
        "ADRへ記録するPRを出してください。スキルの「しないこと」を守ってください。"
    )
    log = paths.log("record", name)
    print(f"結果の記録を始める（ログ: {paths.rel(log)}）", flush=True)
    with open(log, "ab") as fh:
        try:
            result = subprocess.run(
                [
                    claude, "-p", prompt,
                    "--permission-mode", "acceptEdits",
                    "--allowedTools",
                    "Read", "Edit", "Write", "Glob", "Grep",
                    "Bash(./bin/hmwr:*)", "Bash(hmwr:*)", "Bash(git:*)", "Bash(gh:*)", "Bash(cat:*)",
                ],  # fmt: skip
                cwd=str(paths.REPO),
                stdout=fh,
                stderr=subprocess.STDOUT,
                stdin=subprocess.DEVNULL,
                timeout=RECORD_TIMEOUT,
                check=False,
            )
            code = result.returncode
        except subprocess.TimeoutExpired:
            code = -1
    # 記録の途中で止まっても、次の実験のためにworktreeをorigin/mainへ戻す
    proc.succeeds(["git", "switch", "--quiet", "--detach", "origin/main"])
    proc.succeeds(["git", "checkout", "--quiet", "--", "."])
    if code != proc.OK:
        comment(
            number,
            f"結果の記録が終了コード {code} で止まった。対話セッションで記録する。\n\n"
            f"ログ: `{paths.rel(log)}`",
        )


def _fail(number: int, state: str, reason: str, name: str | None = None) -> int:
    """失敗を書き戻す。次の回は次の実験へ進む。"""
    body = reason
    if name:
        body += f"\n\n```\n{_progress(name)}\n```\n\nログの末尾:\n\n```\n{_log_tail(name)}\n```"
        body += "\n\n原因を直したら、ラベルを `queued` へ戻すと続きから走る。"
    relabel(number, add=FAILED, remove=state)
    comment(number, body)
    print(f"#{number}: {reason}", file=sys.stderr)
    return proc.RUNTIME


def _progress(name: str) -> str:
    out, _ = proc.capture_both(
        [sys.executable, str(paths.REPO / "bin" / "hmwr"), "exp", "show", name]
    )
    return out.strip()


def _log_tail(name: str) -> str:
    log = paths.LOGS / f"exp-{name}.log"
    if not log.is_file():
        return "（ログがない）"
    lines = log.read_text(encoding="utf-8", errors="replace").splitlines()
    return "\n".join(lines[-LOG_TAIL:])


# --- launchdからの入口 -------------------------------------------------


def _detached() -> bool:
    return proc.git("rev-parse", "--abbrev-ref", "HEAD") == "HEAD"


def run(args: argparse.Namespace) -> int:
    """コードをorigin/mainへ揃えてからtickを呼ぶ。"""
    if not _detached():
        raise proc.Fail(
            "ブランチ上の作業ツリーでは走らせない。ブランチの切り替えが実験のコードを変える。\n"
            "専用のworktreeを hmwr queue install で用意する"
        )
    if exp.pause_file().exists():
        print("一時停止中（hmwr queue resume で解除）")
        return proc.OK
    if args.dry_run:
        print("[dry-run] git fetch origin main && git checkout --detach origin/main")
        print("[dry-run] hmwr queue tick")
        return proc.OK

    proc.run(["git", "fetch", "--quiet", "origin", "main"])
    proc.run(["git", "checkout", "--quiet", "--detach", "origin/main"])
    # 揃えた後のコードで続きをやる。この処理自体は古いコードで動いている
    return subprocess.run(
        [sys.executable, str(paths.REPO / "bin" / "hmwr"), "queue", "tick"],
        cwd=str(paths.REPO),
        check=False,
    ).returncode


def _build_if_moved() -> None:
    """コミットが進んでいたらツールを作り直す。計測と同じフラグで作る。

    専用のworktreeでだけ行う。開発用の作業ツリーのビルドは、開発者が管理する。
    """
    if not _detached():
        return
    head = proc.git("rev-parse", "HEAD")
    stamp = paths.REPO / "target" / ".queue-built"
    if stamp.is_file() and stamp.read_text().strip() == head:
        return
    proc.run(
        ["cargo", "build", "--release", "--quiet"],
        env={"RUSTFLAGS": config.rustflags()},
        log=paths.log("queue", "build"),
    )
    stamp.write_text(head + "\n")


# --- 状態と一時停止 ----------------------------------------------------


def status(args: argparse.Namespace) -> int:
    print("一時停止: " + ("している" if exp.pause_file().exists() else "していない"))
    for state in (RUNNING, QUEUED, FAILED):
        for issue in issues(state):
            name = spec_name(issue["body"]) or "（specのパスなし）"
            print(f"{paths.pad(state, 9)}#{issue['number']} {name}  {issue['title']}")
    return proc.OK


def pause(args: argparse.Namespace) -> int:
    if args.dry_run:
        print(f"[dry-run] touch {paths.rel(exp.pause_file())}")
        return proc.OK
    paths.QUEUE.mkdir(parents=True, exist_ok=True)
    exp.pause_file().touch()
    print("一時停止した。走っているステップは最後まで走り、次のステップは始まらない")
    return proc.OK


def resume(args: argparse.Namespace) -> int:
    if args.dry_run:
        print(f"[dry-run] rm {paths.rel(exp.pause_file())}")
        return proc.OK
    exp.pause_file().unlink(missing_ok=True)
    print("一時停止を解いた。次の回（5分以内）から続きが走る")
    return proc.OK


# --- 常駐の用意 --------------------------------------------------------

# worktreeには無く、開発用の作業ツリーと共有するもの。どれもgitignoreの対象
SHARED = ("data", "training/checkpoints", "training/runs")
# 一部のファイルだけがgit管理のディレクトリ。管理外のファイルを1つずつつなぐ
PARTLY_TRACKED = ("openings",)


def runner_dir() -> Path:
    return paths.REPO.parent / f"{paths.REPO.name}-runner"


def plist_path() -> Path:
    return Path.home() / "Library" / "LaunchAgents" / f"{AGENT}.plist"


def plist(runner: Path) -> dict:
    return {
        "Label": AGENT,
        # 実験は数時間かかる。走っている間はスリープさせない
        "ProgramArguments": [
            "/usr/bin/caffeinate", "-i",
            sys.executable, str(runner / "bin" / "hmwr"), "queue", "run",
        ],  # fmt: skip
        "WorkingDirectory": str(runner),
        "StartInterval": INTERVAL,
        "RunAtLoad": True,
        # 対局の持ち時間を乱さないよう、バックグラウンドの絞りを受けない種別にする
        "ProcessType": "Standard",
        "EnvironmentVariables": {
            "HOME": str(Path.home()),
            "LANG": "ja_JP.UTF-8",
            "PATH": os.environ.get("PATH", ""),
        },
        "StandardOutPath": str(paths.LOGS / "queue-launchd.log"),
        "StandardErrorPath": str(paths.LOGS / "queue-launchd.log"),
    }


def install(args: argparse.Namespace) -> int:
    if _detached():
        raise proc.Fail("開発用の作業ツリーから実行する（ここは専用のworktreeに見える）")
    runner = runner_dir()
    dry = args.dry_run

    if not runner.is_dir():
        proc.run(["git", "fetch", "--quiet", "origin", "main"], dry_run=dry)
        proc.run(["git", "worktree", "add", "--detach", str(runner), "origin/main"], dry_run=dry)
    for rel in SHARED:
        source, link = paths.REPO / rel, runner / rel
        if dry:
            print(f"[dry-run] ln -s {source} {link}")
            continue
        source.mkdir(parents=True, exist_ok=True)
        if link.is_symlink():
            continue
        if link.exists():
            raise proc.Fail(f"リンクを張る場所に実体がある: {link}")
        link.parent.mkdir(parents=True, exist_ok=True)
        link.symlink_to(source)

    for rel in PARTLY_TRACKED:
        for source in sorted((paths.REPO / rel).iterdir()):
            if source.is_symlink() or proc.succeeds(
                ["git", "ls-files", "--error-unmatch", str(source.relative_to(paths.REPO))]
            ):
                continue
            link = runner / rel / source.name
            if dry:
                print(f"[dry-run] ln -s {source} {link}")
            elif not link.is_symlink() and not link.exists():
                link.parent.mkdir(parents=True, exist_ok=True)
                link.symlink_to(source)

    target = plist_path()
    if dry:
        print(f"[dry-run] {target} を書き、launchctl bootstrap で常駐させる")
        return proc.OK
    paths.LOGS.mkdir(parents=True, exist_ok=True)
    with open(target, "wb") as fh:
        plistlib.dump(plist(runner), fh)
    domain = f"gui/{os.getuid()}"
    proc.succeeds(["launchctl", "bootout", f"{domain}/{AGENT}"])
    proc.run(["launchctl", "bootstrap", domain, str(target)])
    print(f"常駐させた: {AGENT}（{INTERVAL}秒おき）")
    print(f"worktree : {runner}")
    print(f"ログ     : {paths.rel(paths.LOGS / 'queue-launchd.log')}")
    print("止めるには hmwr queue pause、外すには hmwr queue uninstall")
    return proc.OK


def uninstall(args: argparse.Namespace) -> int:
    target = plist_path()
    if args.dry_run:
        print(f"[dry-run] launchctl bootout gui/{os.getuid()}/{AGENT} && rm {target}")
        return proc.OK
    proc.succeeds(["launchctl", "bootout", f"gui/{os.getuid()}/{AGENT}"])
    target.unlink(missing_ok=True)
    print(f"常駐を外した: {AGENT}（worktreeは残っている: {runner_dir()}）")
    return proc.OK
