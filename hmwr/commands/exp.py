"""実験のspecを実行する（ADR-0209）。

specのステップを上から順に走らせ、終わるたびに完了印を書く。途中で止まっても、
同じコマンドで続きから走る。学習と対局の途中からの再開は、各コマンドの
再開の口に任せる。

**実行するのはorigin/mainにあるspecだけである。** 事前登録のPRをマージしてから
走らせる運用を、機械で強制する。
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

from .. import heartbeat, paths, proc, report as report_mod, spec


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("exp", help="実験のspecを実行する")
    ss = p.add_subparsers(dest="sub", metavar="<操作>")

    t = ss.add_parser(
        "run",
        help="specのステップを順に実行する",
        description="experiments/<名前>.toml のステップを上から順に走らせる。"
        "終わったステップは飛ばすので、止まっても同じコマンドで続きから走る。"
        "読むのはorigin/mainのspecで、マージ前のspecは実行しない。",
    )
    t.add_argument("name", help="specの名前（拡張子なし）")
    t.add_argument(
        "--worktree",
        action="store_true",
        help="origin/mainではなく作業ツリーのspecを読む。実行器そのものを試すとき用",
    )
    t.set_defaults(func=run)

    t = ss.add_parser("show", help="ステップごとの進み具合を出す")
    t.add_argument("name", help="specの名前")
    t.add_argument("--worktree", action="store_true", help="作業ツリーのspecを読む")
    t.set_defaults(func=show)

    t = ss.add_parser(
        "report",
        help="実験の数値をMarkdownの表にする",
        description="対局は結果ファイル、学習は実験台帳、データは完了印から読む。"
        "結果の記録には、この表をそのまま貼る。数値を手で書き写さない。"
        "specの無い実験は --match と --net で名前を渡す。",
    )
    t.add_argument("name", nargs="?", help="specの名前。省いたら --match と --net だけを表にする")
    t.add_argument("--match", action="append", default=[], metavar="名前", help="対局の名前")
    t.add_argument("--net", action="append", default=[], metavar="名前", help="学習したネットの名前")
    t.add_argument("--worktree", action="store_true", help="作業ツリーのspecを読む")
    t.set_defaults(func=report)

    t = ss.add_parser(
        "reset",
        help="完了印を消し、最初から走り直せるようにする",
        description="消すのはステップの完了印だけで、成果物には触らない。"
        "各コマンドの完了印が残っていれば、走り直しても作り直しは起きない。",
    )
    t.add_argument("name", help="specの名前")
    t.set_defaults(func=reset)

    t = ss.add_parser(
        "check",
        help="作業ツリーのspecを検査する",
        description="書き方の規約と、全ステップが予行演習を通ることを確かめる。"
        "引数の誤りと、廃止されたコマンドはここで落ちる。",
    )
    t.add_argument("names", nargs="*", metavar="名前", help="省くと全部")
    t.set_defaults(func=check)


def pause_file() -> Path:
    """一時停止の印。置いてある間は、次のステップを始めない（hmwr queue pause）。"""
    return paths.QUEUE / "PAUSE"


def state_dir(name: str) -> Path:
    return paths.QUEUE / paths.check_name(name)


def _load(args: argparse.Namespace) -> spec.Spec:
    if args.worktree:
        return spec.load(args.name, ref=None)
    if not args.dry_run:
        proc.succeeds(["git", "fetch", "--quiet", "origin", "main"])
    return spec.load(args.name)


def _mark(name: str, step: spec.Step) -> Path:
    return state_dir(name) / f"{step.id}.done"


def _is_done(name: str, step: spec.Step) -> bool:
    """完了印があればTrue。印と今のコマンドが違えば、specが途中で変わっている。"""
    mark = _mark(name, step)
    if not mark.is_file():
        return False
    before = json.loads(mark.read_text(encoding="utf-8"))["run"]
    if before != step.run:
        raise proc.Fail(
            f"完了したステップのコマンドがspecと違う: {name}/{step.id}\n"
            f"実行済み: {before}\nspec    : {step.run}\n"
            "走行後に手順を変えない。変えるなら別の名前のspecにする"
        )
    return True


# 対局のステップは判定が成果である。H0（1）と見送り（2）も完了として受け、
# 読み方はADRの事前登録に任せる（ADR-0217）
MATCH_STEPS = (("match", "run"), ("sprt", "run"), ("sprt", "net"))


def _is_match(step) -> bool:
    return tuple(step.argv[:2]) in MATCH_STEPS


def _allowed_codes(step) -> tuple[int, ...]:
    return (proc.OK, 1, 2) if _is_match(step) else (proc.OK,)


def _has_result(step) -> bool:
    """対局の名前で決まる結果ファイルがあるか。見送りは結果ファイルを持つ。"""
    if not _is_match(step) or len(step.argv) < 3:
        return False
    from . import match

    return match.files(step.argv[2])["result"].is_file()


def run(args: argparse.Namespace) -> int:
    s = _load(args)
    log = paths.log("exp", s.name)
    print(f"=== 実験: {s.name}（ADR-{s.adr}、{len(s.steps)}ステップ） ===")
    # ステップの所要は数分から数日まで揃わないので、残り時間は見積もらない
    with heartbeat.running(
        "exp", s.name, dry_run=args.dry_run, total=len(s.steps), unit="ステップ",
        log=log, estimate=False, detail={"adr": s.adr},
    ) as beat:  # fmt: skip
        return _run_steps(args, s, log, beat)


def _run_steps(args: argparse.Namespace, s: spec.Spec, log: Path, beat) -> int:
    for i, step in enumerate(s.steps, 1):
        head = f"[{i}/{len(s.steps)}] {step.id}"
        if not args.dry_run and _is_done(s.name, step):
            print(f"{head}: 済み")
            continue
        if not args.dry_run and pause_file().exists():
            # 走っているステップは止めない。切れ目で止まり、再開は同じコマンドで行う
            print(f"{head}: 一時停止中なので始めない（hmwr queue resume で解除）")
            beat.finish("stopped", reason="paused")
            return proc.JUDGE
        print(f"{head}: {step.run}", flush=True)
        beat.update(i - 1, detail={"step": step.id, "run": step.run}, force=True)
        if args.dry_run:
            code = _dry(step)
            if code != proc.OK:
                raise proc.Fail(f"予行演習が通らない: {s.name}/{step.id}", code)
            continue

        started = time.time()
        argv = [sys.executable, str(paths.REPO / "bin" / "hmwr"), *step.argv]
        try:
            code = proc.run(argv, log=log, allowed=_allowed_codes(step))
        except proc.Fail as e:
            raise proc.Fail(f"ステップが失敗した: {s.name}/{step.id}\n{e}", e.code) from e
        if code == 2 and not _has_result(step):
            # 対局が判定に至らず、見送りの結果も書けていない。中断なので次回に続ける
            raise proc.Fail(f"対局が途中で止まった: {s.name}/{step.id}", proc.RUNTIME)
        mark = _mark(s.name, step)
        mark.parent.mkdir(parents=True, exist_ok=True)
        mark.write_text(
            json.dumps(
                {
                    "run": step.run,
                    "seconds": round(time.time() - started),
                    "finished": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
                },
                ensure_ascii=False,
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )

    if not args.dry_run:
        print(f"全ステップが完了した: {s.name}")
    return proc.OK


def _dry(step: spec.Step) -> int:
    """ステップを予行演習で通す。引数の誤りはSystemExitで返る。"""
    from .. import cli  # 循環を避けるため、使う場所で読む

    try:
        return cli.main(["--dry-run", *step.argv])
    except SystemExit as e:
        return int(e.code or 0)


def show(args: argparse.Namespace) -> int:
    s = _load(args)
    print(f"=== 実験: {s.name}（ADR-{s.adr}） ===")
    pending = 0
    for step in s.steps:
        mark = _mark(s.name, step)
        if mark.is_file():
            info = json.loads(mark.read_text(encoding="utf-8"))
            state = f"済み（{info.get('seconds', '?')}秒、{info.get('finished', '')}）"
        else:
            state = "未了"
            pending += 1
        print(f"{paths.pad(step.id, 20)}{state}")
    return proc.OK if pending == 0 else proc.JUDGE


def report(args: argparse.Namespace) -> int:
    matches, nets, data = [], [], []
    if args.name:
        s = _load(args)
        matches, nets, data = report_mod.from_steps([step.argv for step in s.steps])
    elif not (args.match or args.net):
        raise proc.Fail("specの名前か、--match・--net を渡す", proc.USAGE)
    matches += [(name, "") for name in args.match]
    nets += args.net
    print(report_mod.render(matches, nets, data))
    return proc.OK


def reset(args: argparse.Namespace) -> int:
    marks = sorted(state_dir(args.name).glob("*.done"))
    for mark in marks:
        if args.dry_run:
            print(f"[dry-run] rm {paths.rel(mark)}")
        else:
            mark.unlink()
    if not args.dry_run:
        print(f"完了印を{len(marks)}個消した: {args.name}")
    return proc.OK


def check(args: argparse.Namespace) -> int:
    targets = args.names or spec.names()
    if not targets:
        print("specはまだない")
        return proc.OK
    bad = 0
    for name in targets:
        try:
            s = spec.load(name, ref=None)
            if spec.adr_file(s.adr) is None:
                raise spec.Invalid(f"{name}: docs/adr/{s.adr}-*.md がない")
            for step in s.steps:
                code = _dry(step)
                if code != proc.OK:
                    raise spec.Invalid(f"{name}/{step.id}: 予行演習が通らない（終了コード {code}）")
        except proc.Fail as e:
            print(f"NG {name}: {e}", file=sys.stderr)
            bad += 1
            continue
        print(f"OK {name}（{len(s.steps)}ステップ）")
    return proc.OK if bad == 0 else proc.RUNTIME
