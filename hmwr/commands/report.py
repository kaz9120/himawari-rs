"""実験の数値をMarkdownの表にする。"""

from __future__ import annotations

import argparse

from .. import proc
from .. import report as report_mod


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser(
        "report",
        help="実験の数値をMarkdownの表にする",
        description="対局は結果ファイル、学習は学習の記録から読む。"
        "結果を記録するときは、この表をそのまま貼る。数値を手で書き写さない。",
    )
    p.add_argument("--match", action="append", default=[], metavar="名前", help="対局の名前")
    p.add_argument("--net", action="append", default=[], metavar="名前", help="学習したネットの名前")
    p.add_argument("--data", action="append", default=[], metavar="名前", help="加工した教師データの名前")
    p.set_defaults(func=run)


def run(args: argparse.Namespace) -> int:
    if not (args.match or args.net or args.data):
        raise proc.Fail("--match・--net・--data のどれかを渡す", proc.USAGE)
    print(report_mod.render([(name, "") for name in args.match], args.net, args.data))
    return proc.OK
