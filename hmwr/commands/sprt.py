"""対局ゲート（SPRT）の起動・確認・待機。

判定が出るまで走らせ、完了は結果ファイルの有無で決まる。プロセスの生死や
セッションの継続に依存しない設計は ADR-0175 にある。

起動したプロセスは新しいセッションへ移す。エージェントがツールの
バックグラウンド実行のまま走らせると、数十分で回収されて止まるためである
（2026-08-18に2回。標準出力を捨てても起きたので出力量とは無関係）。
**切り離してよいのは、状態がファイルにあるからである。**
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
import time
from pathlib import Path

from .. import config, paths, proc, sprt_log

# 異常終了からの再開を数える上限。判定に至らないまま無限に試し続けない
MAX_RETRY = 20
RETRY_WAIT = 5


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("sprt", help="対局で棋力を検定する")
    ss = p.add_subparsers(dest="sub", metavar="<操作>")

    t = ss.add_parser(
        "run",
        help="ペアを作り、機能検証を通してから起動する",
        description="ビルド・機能検証・起動を順に行う。判定が出るまで走り、"
        "落ちても棋譜から再開する。すでに判定済みなら結果を返して終わる。",
    )
    t.add_argument("name", help="実験名")
    t.add_argument("--baseline", metavar="REF", help="比較元のref（既定 origin/main）")
    t.add_argument(
        "--noninferiority",
        action="store_true",
        help="非劣性で測る（elo0=-5、elo1=0）",
    )
    t.add_argument("--tc", metavar="持ち時間", help="例 60+0.6（既定 10+0.1）")
    t.add_argument(
        "--set",
        action="append",
        metavar="KEY=VALUE",
        help="測定条件を直接渡す（繰り返し可）",
    )
    t.add_argument(
        "--no-verify",
        dest="verify",
        action="store_false",
        help="機能検証を飛ばす。終盤にしか出ない機能を測るときだけ使う",
    )
    t.add_argument(
        "--foreground",
        action="store_true",
        help="切り離さずその場で走らせる。判定まで戻らない",
    )
    # 切り離した子プロセスが自分を呼び直すための内部フラグ
    t.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    t.set_defaults(func=run, verify=True)

    t = ss.add_parser(
        "net",
        help="ビルドを固定し、評価関数だけを差し替えて測る",
        description="同じビルドで対局し、評価関数を片側ずつ指定する。"
        "ネットとビルドの次元は揃える。",
    )
    t.add_argument("base", metavar="baselineネット")
    t.add_argument("cand", metavar="candidateネット")
    t.add_argument("name", help="実験名")
    t.add_argument("--bin", metavar="パス", help="対局に使うビルド")
    t.add_argument("--noninferiority", action="store_true", help="非劣性で測る")
    t.add_argument("--tc", metavar="持ち時間", help="例 60+0.6")
    t.add_argument("--set", action="append", metavar="KEY=VALUE", help="測定条件")
    t.add_argument("--foreground", action="store_true", help="切り離さず走らせる")
    t.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    t.set_defaults(func=run_net)

    t = ss.add_parser("show", help="途中経過や結果を出す。名前を省くと一覧")
    t.add_argument("name", nargs="?", help="実験名")
    t.add_argument("--all", action="store_true", help="完了した走行も並べる")
    t.set_defaults(func=show)

    t = ss.add_parser("wait", help="判定が出るまで待つ")
    t.add_argument("name", help="実験名")
    t.add_argument("--interval", type=int, default=60, metavar="秒", help="確認の間隔")
    t.set_defaults(func=wait)


def files(name: str) -> dict[str, Path]:
    """この名前で決まる置き場をまとめて返す。"""
    paths.check_name(name)
    return {
        "base": paths.BIN / f"base-{name}",
        "cand": paths.BIN / f"cand-{name}",
        "jsonl": paths.SPRT / f"{name}.jsonl",
        "result": paths.SPRT / f"{name}.result",
        "log": paths.log("sprt", name),
    }


# --- 起動 --------------------------------------------------------------


def run(args: argparse.Namespace) -> int:
    """ビルド・機能検証・起動を順に行う。

    3つを別々に叩けると順番を飛ばせてしまう。機能検証を飛ばすと、探索に
    影響のない変更へ対局リソースを払うことになる（ADR-0074）。ここで
    順番を固定し、飛ばすには明示を求める。
    """
    f = files(args.name)

    if args.worker:
        # 切り離された子。判定が出るまで回す
        return until_decision(args.name, settings(args), dry_run=False)

    if f["result"].is_file():
        print(f"判定済み: {paths.rel(f['result'])}")
        print(f["result"].read_text(encoding="utf-8"), end="")
        return proc.OK

    build = [proc.script("build-pair.sh"), args.name]
    if args.baseline:
        build.append(args.baseline)
    proc.run(build, dry_run=args.dry_run)

    if args.verify and not _verified(args):
        return proc.JUDGE

    return _start(args, args.name, ["sprt", "run", args.name])


def run_net(args: argparse.Namespace) -> int:
    """評価関数だけを差し替えて測る。ビルドは同じものを両側に使う。"""
    f = files(args.name)

    if args.worker:
        return until_decision(
            args.name, settings(args), dry_run=False, nets=_net_options(args)
        )

    if f["result"].is_file():
        print(f"判定済み: {paths.rel(f['result'])}")
        print(f["result"].read_text(encoding="utf-8"), end="")
        return proc.OK

    for net in (args.base, args.cand):
        if not Path(net).is_file() and not args.dry_run:
            raise proc.Fail(f"ネットがない: {net}")

    rest = ["sprt", "net", args.base, args.cand, args.name]
    if args.bin:
        rest += ["--bin", args.bin]
    return _start(args, args.name, rest, nets=_net_options(args))


def _verified(args: argparse.Namespace) -> bool:
    """機能検証を通す。全局面で一致したらFalseを返し、起動を止める。"""
    f = files(args.name)
    code = proc.run(
        proc.cargo_tool("verify", [str(f["base"]), str(f["cand"])]),
        dry_run=args.dry_run,
        env=config.measure_env(),
        log=paths.log("verify", args.name),
        allowed=(proc.OK, proc.JUDGE),
    )
    if code != proc.JUDGE:
        return True
    print()
    print("全局面でノード数が一致した。この変更は探索に影響していない。")
    print("対局にかけても中立にしかならないので起動しない。")
    print("終盤にしか出ない機能なら、終盤局面を別に用意して測り直す。")
    print("それでも走らせるなら --no-verify を付ける。")
    return False


def settings(args: argparse.Namespace) -> dict[str, str]:
    """フラグを測定条件の環境変数へ畳む。"""
    env: dict[str, str] = {}
    if getattr(args, "noninferiority", False):
        # 参照追従で「害がなければ入れたい」変更（ADR-0163）
        env["SPRT_ELO0"] = "-5"
        env["SPRT_ELO1"] = "0"
    if getattr(args, "tc", None):
        env["SPRT_TC"] = args.tc
    for item in getattr(args, "set", None) or []:
        if "=" not in item:
            raise proc.Fail(f"--set はKEY=VALUEで書く: {item}", proc.USAGE)
        key, value = item.split("=", 1)
        env[key] = value
    return env


def _net_options(args: argparse.Namespace) -> dict[str, str]:
    """評価関数を片側ずつ指定するための情報。"""
    return {"base": args.base, "cand": args.cand, "bin": args.bin or ""}


def _start(
    args: argparse.Namespace,
    name: str,
    rest: list[str],
    nets: dict[str, str] | None = None,
) -> int:
    """判定まで回す処理を、前面か切り離しかで起動する。"""
    f = files(name)
    env = settings(args)

    if args.foreground:
        return until_decision(name, env, dry_run=args.dry_run, nets=nets)

    argv = [sys.executable, str(paths.REPO / "bin" / "hmwr"), *rest, "--worker"]
    for key, value in env.items():
        argv += ["--set", f"{key}={value}"]

    if args.dry_run:
        print(f"[dry-run] （新しいセッションで）{proc.show(argv)}")
        return proc.OK

    # 親から切り離す。状態はファイルにあるので、掴んでおく必要がない
    with open(os.devnull, "wb") as devnull:
        child = subprocess.Popen(
            argv,
            cwd=str(paths.REPO),
            stdout=devnull,
            stderr=devnull,
            stdin=devnull,
            start_new_session=True,
        )
    print(f"起動した pid={child.pid}（新しいセッション）")
    print(f"経過: hmwr sprt show {name}")
    print(f"完了: {paths.rel(f['result'])} の出現を見る")
    return proc.OK


# --- 判定まで回す ------------------------------------------------------


def until_decision(
    name: str,
    env: dict[str, str],
    *,
    dry_run: bool,
    nets: dict[str, str] | None = None,
) -> int:
    """判定が出るまで走らせる。

    2つを自動化する。落ちたら棋譜から拾い直すことと、上限に達しても
    判定が出ていなければそのまま走り続けることである（ADR-0087・0175）。

    上限は収束の判定基準ではなく暴走を止める安全弁である。真のEloが
    対立仮説の中点ちょうどだと理論上収束しないため、無制限にはしない。

    効く範囲と効かない範囲がある。対局プロセスだけが落ちた場合はここが
    拾い直す。この処理自体が止められた場合はループごと消えるが、棋譜は
    残るので次に呼べば続きから走る。**「止まらない」ではなく「止まっても
    失わない」のが本質である。**
    """
    f = files(name)
    if f["result"].is_file():
        print(f["result"].read_text(encoding="utf-8"), end="")
        return _exit_code(f["result"])

    hard_max = env.get("SPRT_MAX_PAIRS") or config.get("SPRT_HARD_MAX_PAIRS", "60000")
    env = {**env, "SPRT_MAX_PAIRS": hard_max}

    for attempt in range(1, MAX_RETRY + 1):
        before = _games(f["jsonl"])
        code = _selfplay(name, env, dry_run=dry_run, nets=nets, attempt=attempt)
        if dry_run:
            return proc.OK
        if code in (0, 1):
            return _finish(name)
        if code == 2:
            print(f"安全弁（{hard_max} ペア）まで走って判定に至らなかった。")
            print("局数を積むより対立仮説の立て方を見直す。")
            return 2

        # **1局も進まなかった再試行は繰り返さない。** 設定の誤りやバイナリの
        # 欠落なら、何度試しても同じところで落ちる。棋譜が増えているときだけ
        # 「途中で落ちた」とみなして拾い直す
        if _games(f["jsonl"]) == before:
            raise proc.Fail(
                f"1局も進まずに終了した（コード {code}）。設定かバイナリを確かめる。\n"
                f"ログ: {paths.rel(f['log'])}",
                code,
            )
        print(f"試行 {attempt} が異常終了（コード {code}）。再開する", file=sys.stderr)
        time.sleep(RETRY_WAIT)

    raise proc.Fail(f"{MAX_RETRY}回試しても判定に至らなかった")


def _games(jsonl: Path) -> int:
    """棋譜に記録された局数。ファイルが無ければ0。"""
    if not jsonl.is_file():
        return 0
    return sum(1 for _ in jsonl.open("rb"))


def _selfplay(
    name: str,
    env: dict[str, str],
    *,
    dry_run: bool,
    nets: dict[str, str] | None,
    attempt: int,
) -> int:
    """対局を1回走らせる。棋譜があれば続きから測る。"""
    f = files(name)
    binary = paths.REPO / "target" / "release" / "selfplay"
    if not binary.is_file() and not dry_run:
        raise proc.Fail(f"{paths.rel(binary)} がない。cargo build --release を実行する")

    def setting(key: str, fallback: str = "") -> str:
        return env.get(key) or config.get(key, fallback)

    argv = [str(binary)]
    if nets:
        # 評価関数だけを差し替える。--option と併用すると、どちらが効くかが
        # 実装依存になるため、片側指定だけで完結させる
        engine = nets["bin"] or _default_bin(name)
        argv += ["--baseline", engine, "--candidate", engine]
        argv += ["--bopt", f"EvalFile={_abs(nets['base'])}"]
        argv += ["--copt", f"EvalFile={_abs(nets['cand'])}"]
    else:
        argv += ["--baseline", str(f["base"]), "--candidate", str(f["cand"])]
        eval_file = setting("EVAL_FILE")
        if eval_file:
            argv += ["--option", f"EvalFile={eval_file}"]

    argv += [
        "--openings", setting("OPENINGS"),
        "--tc", setting("SPRT_TC", "10+0.1"),
        "--concurrency", setting("SPRT_CONCURRENCY", "8"),
        "--adjudicate", setting("SPRT_ADJUDICATE", "2000,8"),
        "--elo0", setting("SPRT_ELO0", "0"),
        "--elo1", setting("SPRT_ELO1", "5"),
        "--alpha", setting("SPRT_ALPHA", "0.05"),
        "--beta", setting("SPRT_BETA", "0.05"),
        "--max-pairs", setting("SPRT_MAX_PAIRS", "60000"),
        "--out", str(f["jsonl"]),
    ]

    f["jsonl"].parent.mkdir(parents=True, exist_ok=True)
    games = _games(f["jsonl"])
    if games:
        print(f"試行 {attempt}: 既存の棋譜から再開する（{games} 局）")
        argv += ["--resume", str(f["jsonl"])]
    else:
        print(f"試行 {attempt}: 新規に開始する")

    return proc.run(argv, dry_run=dry_run, log=f["log"], allowed=(0, 1, 2, 3))


def _default_bin(name: str) -> str:
    """対局に使うビルド。実験ごとに固定したいので data/bin を先に見る。"""
    fixed = paths.BIN / f"base-{name}"
    if fixed.is_file():
        return str(fixed)
    return str(paths.REPO / "target" / "release" / "himawari")


def _abs(path: str) -> str:
    p = Path(path)
    return str(p if p.is_absolute() else paths.REPO / p)


def _finish(name: str) -> int:
    """判定が出た。結果ファイルを書き、判定を終了コードで返す。"""
    f = files(name)
    try:
        text, verdict = sprt_log.report(f["log"], name, result=f["result"])
    except sprt_log.Unreadable as e:
        raise proc.Fail(f"判定は出たが結果を読めない: {e}") from e
    print(text)
    return sprt_log.EXIT_BY_VERDICT[verdict]


def _exit_code(result: Path) -> int:
    for line in result.read_text(encoding="utf-8").splitlines():
        if line.startswith("decision="):
            return sprt_log.EXIT_BY_VERDICT.get(line.split("=", 1)[1], proc.RUNTIME)
    raise proc.Fail(f"結果ファイルの判定を読めない: {paths.rel(result)}")


# --- 確認 --------------------------------------------------------------


def show(args: argparse.Namespace) -> int:
    """途中経過を出す。名前を省くと走行を新しい順に並べる。"""
    if not args.name:
        return _list(args.all)

    f = files(args.name)
    try:
        text, verdict = sprt_log.report(f["log"], args.name)
    except sprt_log.Unreadable as e:
        raise proc.Fail(str(e)) from e
    print(text)
    return sprt_log.EXIT_BY_VERDICT[verdict]


def _list(show_all: bool) -> int:
    """走行の一覧。完了は結果ファイルの有無で決まる（ADR-0175）。"""
    if not paths.SPRT.is_dir():
        print("走行はまだない")
        return proc.OK

    rows = []
    for jsonl in paths.SPRT.glob("*.jsonl"):
        name = jsonl.stem
        done = (paths.SPRT / f"{name}.result").is_file()
        games = sum(1 for _ in jsonl.open("rb"))
        rows.append((jsonl.stat().st_mtime, name, "完了" if done else "未完了", games))
    if not rows:
        print("走行はまだない")
        return proc.OK

    rows.sort(reverse=True)
    shown = rows if show_all else rows[:10]
    width = max(paths.display_width(r[1]) for r in shown) + 2
    print(paths.pad("名前", width) + paths.pad("状態", 8) + "局数")
    for _, name, state, games in shown:
        print(paths.pad(name, width) + paths.pad(state, 8) + str(games))
    if not show_all and len(rows) > len(shown):
        print(f"\n新しい順に{len(shown)}件。全{len(rows)}件を見るには --all")
    print("\n「未完了」は結果ファイルがない状態を指す。走っているとは限らない。")
    return proc.OK


def wait(args: argparse.Namespace) -> int:
    """判定が出るまで待つ。

    結果ファイルの出現を見る。対局プロセスが消えていて結果も無ければ、
    判定前に止まったとみなす。
    """
    f = files(args.name)
    if args.dry_run:
        print(f"[dry-run] {paths.rel(f['result'])} の出現を待つ")
        return proc.OK

    while True:
        if f["result"].is_file():
            print(f["result"].read_text(encoding="utf-8"), end="")
            return _exit_code(f["result"])
        if not _running(args.name):
            print("判定前に止まった（中断・停止・失敗のいずれか）", file=sys.stderr)
            if f["log"].is_file():
                tail = f["log"].read_text(encoding="utf-8").splitlines()[-2:]
                print("\n".join(tail), file=sys.stderr)
            return proc.JUDGE
        time.sleep(args.interval)


def _running(name: str) -> bool:
    """この実験の対局プロセスが生きているか。"""
    out = proc.capture(["pgrep", "-f", f"selfplay .*{name}"])
    return bool(out.strip())
