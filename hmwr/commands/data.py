"""教師データの取得と前処理。"""

from __future__ import annotations

import argparse

from .. import proc


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("data", help="教師データを取得・加工する")
    ss = p.add_subparsers(dest="sub", metavar="<操作>")

    t = ss.add_parser(
        "fetch",
        help="公開データセットを取得してpsvを作る",
        description="生データ116GBと加工後120GBで、空きが236GB要る。"
        "取得は再実行できる。妥当なサイズのファイルは飛ばす。",
    )
    t.add_argument(
        "stage",
        nargs="?",
        default="all",
        choices=["download", "verify", "prepare", "all"],
        metavar="<段>",
        help="download / verify / prepare / all（既定 all）",
    )
    t.set_defaults(func=fetch)

    t = ss.add_parser(
        "quiet",
        help="教師局面を静止局面へ置き換える",
        description="評価関数が探索中に見るのは静止局面だが、公開データは"
        "取り合いの途中の局面へ収束後の評価値を付けて配られている。"
        "そのずれを消す。29.9億で7.0時間、3億で50分かかる。",
    )
    t.add_argument("input", metavar="入力psv")
    t.add_argument("output", metavar="出力psv")
    t.add_argument("--name", metavar="名前", help="ログ名。省くと出力名から作る")
    t.add_argument("--max-plies", type=int, metavar="N", help="進める手数の上限（既定 1）")
    t.add_argument("--limit", type=int, metavar="N", help="先頭のこの件数だけ処理する")
    t.set_defaults(func=quiet)


def fetch(args: argparse.Namespace) -> int:
    return proc.run(
        [proc.script("fetch-dataset.sh"), args.stage],
        dry_run=args.dry_run,
    )


def quiet(args: argparse.Namespace) -> int:
    env: dict[str, str] = {}
    if args.max_plies is not None:
        env["QUIET_MAX_PLIES"] = str(args.max_plies)
    if args.limit is not None:
        env["QUIET_LIMIT"] = str(args.limit)
    argv = [proc.script("quiet.sh"), args.input, args.output]
    if args.name:
        argv.append(args.name)
    return proc.run(argv, dry_run=args.dry_run, env=env)
