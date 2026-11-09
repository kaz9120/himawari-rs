"""単発の診断。1つの実験のために作った測定の置き場。

**ここは互換を保証するインターフェースではない**。引数の互換は保たず、使わなくなったら
消してよい。消したものはgitの履歴から掘り起こせる。診断を足すときはここに置き、
2つ目の実験で使われたら正規の領域への昇格を検討する。
"""

from __future__ import annotations

import argparse
from pathlib import Path

from .. import config, paths, proc
from ..tools import depth_reach
from ..tools import replay as replay_tool


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser(
        "diag",
        help="単発の診断（互換は保たない）",
        description="1つの実験のために作った測定を置く。引数の互換は保たず、"
        "使わなくなったものは消す。",
    )
    ss = p.add_subparsers(dest="sub", metavar="<診断>")

    t = ss.add_parser(
        "replay",
        help="CSAの棋譜を、実戦の持ち時間のまま指し直す",
        description="単発の局面では出ないが対局の流れの中で出る問題を再現する。"
        "置換表・時間管理・ponderの状態は前の手から持ち越されるので、"
        "局面だけを渡す検討では見えない。自分の手番ごとに残り時間で go を送り、"
        "返るまでの時間と最終の反復を出す。時間制の対局と重ねて走らせない。",
    )
    t.add_argument("csa", metavar="CSA", help="棋譜")
    t.add_argument("--engine", metavar="パス", help="エンジン（既定 target/release/himawari）")
    t.add_argument("--eval-file", metavar="パス", help="評価関数（既定は EVAL_FILE）")
    t.add_argument("--player", default="Himawari", metavar="名前", help="自分の側の名前")
    t.add_argument("--threads", type=int, default=4, metavar="N")
    t.add_argument("--from", dest="start_ply", type=int, default=1, metavar="手数", help="この手から探索する")
    t.add_argument("--initial", type=int, default=300, metavar="秒", help="持ち時間")
    t.add_argument("--inc", type=int, default=10, metavar="秒", help="1手ごとの加算")
    t.set_defaults(func=replay)

    t = ss.add_parser(
        "reach",
        help="局面ごとに、固定深さの探索が打ち切りの秒数内に終わるかを測る",
        description="Issue #653の測定。局面ごとに go depth N を送り、"
        "打ち切りの秒数を過ぎたら stop を送る。所要時間と、確定した最後の反復の"
        "深さ・評価値・ノード数を出す。終わらなかった局面と5秒を超えた局面だけを"
        "画面へ出し、全行はログに残す。時間制の対局と重ねて走らせない。",
    )
    t.add_argument("name", metavar="名前", help="測定の名前。ログの名前になる")
    t.add_argument(
        "--engine", metavar="パス", help="エンジン（既定 target/release/himawari）"
    )
    t.add_argument("--eval-file", metavar="パス", help="評価関数（既定は EVAL_FILE）")
    t.add_argument(
        "--positions",
        default="openings/actdump_positions.txt",
        metavar="パス",
        help="局面リスト（既定 openings/actdump_positions.txt）",
    )
    t.add_argument(
        "--only",
        type=int,
        action="append",
        metavar="番号",
        help="この番号の局面だけ測る（繰り返し可。1始まり）",
    )
    t.add_argument(
        "--depth", type=int, default=16, metavar="N", help="探索の深さ（既定16）"
    )
    t.add_argument(
        "--limit",
        type=float,
        default=20.0,
        metavar="秒",
        help="打ち切りの秒数（既定20）",
    )
    t.add_argument("--threads", type=int, default=1, metavar="N")
    t.add_argument("--hash", type=int, default=256, metavar="MB")
    t.set_defaults(func=reach)


def replay(args: argparse.Namespace) -> int:
    """対局の流れを再現する。"""
    engine = args.engine or str(paths.release_bin("himawari"))
    eval_file = args.eval_file or config.get("EVAL_FILE")
    if args.dry_run:
        print(f"[dry-run] {paths.rel(engine)} で {args.csa} を{args.start_ply}手目から指し直す")
        return proc.OK
    for path, what in ((engine, "エンジン"), (eval_file, "評価関数"), (args.csa, "棋譜")):
        if not Path(path).is_file():
            raise proc.Fail(f"{what}がない: {path}")
    return replay_tool.replay(
        engine, eval_file, Path(args.csa),
        player=args.player, threads=args.threads, start_ply=args.start_ply, hash_mb=256,
        initial_ms=args.initial * 1000, inc_ms=args.inc * 1000,
    )  # fmt: skip


def reach(args: argparse.Namespace) -> int:
    """固定深さの探索が打ち切りの秒数内に終わるかを測る。"""
    name = paths.check_name(args.name)
    engine = args.engine or str(paths.release_bin("himawari"))
    eval_file = args.eval_file or config.get("EVAL_FILE")
    log = paths.log("diag-reach", name)
    if args.dry_run:
        print(
            f"[dry-run] {paths.rel(engine)} で {args.positions} を深さ{args.depth}、"
            f"{args.limit:g}秒で打ち切って測る"
        )
        print(f"[dry-run] ログ: {paths.rel(log)}")
        return proc.OK
    for path, what in (
        (engine, "エンジン"),
        (eval_file, "評価関数"),
        (args.positions, "局面リスト"),
    ):
        if not path or not Path(path).is_file():
            raise proc.Fail(f"{what}がない: {path}")
    code = depth_reach.run(
        engine, Path(args.positions),
        eval_file=eval_file, depth=args.depth, limit=args.limit,
        threads=args.threads, hash_mb=args.hash, only=args.only, log=log,
    )  # fmt: skip
    print(f"ログ: {paths.rel(log)}")
    return code
