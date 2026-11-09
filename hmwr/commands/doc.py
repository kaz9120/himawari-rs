"""文書の検査。"""

from __future__ import annotations

import argparse

from .. import doccheck, proc


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("doc", help="文書を検査する")
    ss = p.add_subparsers(dest="sub", metavar="<操作>")

    t = ss.add_parser(
        "check",
        help="文書の相対リンクの行き先を検査する",
        description="Markdownの相対リンクが、リポジトリの中の実在するファイルを指しているかを見る。",
    )
    t.set_defaults(func=check)


def check(args: argparse.Namespace) -> int:
    found = doccheck.run_all()
    for finding in found:
        print(finding)
    if found:
        print(f"\n{len(found)}件の食い違いがある")
        return proc.JUDGE
    print("文書の整合: 問題なし")
    return proc.OK
