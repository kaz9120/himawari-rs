"""文書の検査。"""

from __future__ import annotations

import argparse
import shutil

from .. import doccheck, paths, proc


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

    t = ss.add_parser(
        "check",
        help="文書どうしの整合を検査する",
        description="設計記録（docs/adr）のStatusと索引の一致、ファイルと索引の過不足、"
        "相対リンクの行き先を見る。--stale-days を渡すと、proposedのまま動いていない"
        "設計記録も挙げる。",
    )
    t.add_argument(
        "--stale-days", type=int, default=0, metavar="N", help="proposedのままN日動いていない設計記録を挙げる"
    )
    t.set_defaults(func=check)


def lint(args: argparse.Namespace) -> int:
    if shutil.which("npm") is None:
        raise proc.Fail("npm がない。Node 22以降を入れる")
    if not (paths.REPO / "node_modules").is_dir() and not args.dry_run:
        proc.run(["npm", "ci"], dry_run=args.dry_run)
    return proc.run(
        ["npm", "run", "lint:fix" if args.fix else "lint"],
        dry_run=args.dry_run,
    )


def check(args: argparse.Namespace) -> int:
    found = doccheck.run_all(stale_days=args.stale_days)
    for finding in found:
        print(finding)
    if found:
        print(f"\n{len(found)}件の食い違いがある")
        return proc.JUDGE
    print("文書の整合: 問題なし")
    return proc.OK
