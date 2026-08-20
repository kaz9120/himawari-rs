"""文書の検査。"""

from __future__ import annotations

import argparse
import shutil

from .. import paths, proc


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("doc", help="文書を検査する")
    ss = p.add_subparsers(dest="sub", metavar="<操作>")

    t = ss.add_parser(
        "lint",
        help="日本語文書の書き方を検査する",
        description="一文の長さ・読点の数・助詞の重複・冗長表現・誇張表現を見る。"
        "CIと同じ検査なので、PRを出す前にここで通す。",
    )
    t.add_argument("--fix", action="store_true", help="自動で直せるものを直す")
    t.set_defaults(func=lint)


def lint(args: argparse.Namespace) -> int:
    if shutil.which("npm") is None:
        raise proc.Fail("npm がない。Node 22以降を入れる")
    if not (paths.REPO / "node_modules").is_dir() and not args.dry_run:
        proc.run(["npm", "ci"], dry_run=args.dry_run)
    return proc.run(
        ["npm", "run", "lint:fix" if args.fix else "lint"],
        dry_run=args.dry_run,
    )
