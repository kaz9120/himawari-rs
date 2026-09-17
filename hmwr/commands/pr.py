"""PRを非対話で作る（ADR-0070）。

`gh pr create --template` はエディタを開く前提で、`--body-file` と併用できない。
エージェントは非対話で走るので、テンプレートの中身を本文へ手で写していた。
ここでは、本文にテンプレートの見出しが揃っていることを確かめてから `gh` を呼ぶ。
**種別ごとの記入欄を飛ばせない形にすることが目的である。**
"""

from __future__ import annotations

import argparse
import re
from pathlib import Path

from .. import paths, proc

KINDS = {"chore": "その他", "strength": "棋力向上"}
FRONT_MATTER_RE = re.compile(r"\A---\n.*?\n---\n", re.S)
COMMENT_RE = re.compile(r"<!--.*?-->\n?", re.S)
HEADING_RE = re.compile(r"^## (.+)$", re.M)


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("pr", help="PRを作る")
    ss = p.add_subparsers(dest="sub", metavar="<操作>")

    t = ss.add_parser(
        "template",
        help="種別ごとの本文のひな形を出す",
        description="テンプレートから前書きと記入の手引きを除いて出す。"
        "これをファイルへ書き、各節を埋めてから pr create へ渡す。",
    )
    t.add_argument("kind", choices=sorted(KINDS), metavar="<種別>", help="chore / strength")
    t.set_defaults(func=template)

    t = ss.add_parser(
        "create",
        help="本文の見出しを確かめてからPRを作る",
        description="本文に、種別のテンプレートの見出しが全部あることを確かめる。"
        "マージ条件は種別で決まるので、記入欄を飛ばしたPRを作らせない。",
    )
    t.add_argument("--kind", required=True, choices=sorted(KINDS), help="chore / strength")
    t.add_argument("--title", required=True, metavar="件名", help="Conventional Commitsの型で始める")
    t.add_argument("--body-file", required=True, metavar="ファイル", help="本文")
    t.add_argument("--draft", action="store_true", help="ドラフトで作る")
    t.set_defaults(func=create)


def template_text(kind: str) -> str:
    file = paths.REPO / ".github" / "PULL_REQUEST_TEMPLATE" / f"{kind}.md"
    return file.read_text(encoding="utf-8")


def skeleton(kind: str) -> str:
    """前書きと記入の手引きを除いた、本文のひな形。"""
    return COMMENT_RE.sub("", FRONT_MATTER_RE.sub("", template_text(kind))).lstrip("\n")


def headings(kind: str) -> list[str]:
    return HEADING_RE.findall(template_text(kind))


def template(args: argparse.Namespace) -> int:
    print(skeleton(args.kind), end="")
    return proc.OK


def create(args: argparse.Namespace) -> int:
    body_file = Path(args.body_file)
    if not body_file.is_file():
        raise proc.Fail(f"本文のファイルがない: {args.body_file}")
    body = body_file.read_text(encoding="utf-8")
    missing = [h for h in headings(args.kind) if f"## {h}" not in body]
    if missing:
        raise proc.Fail(
            f"本文に「{KINDS[args.kind]}」のテンプレートの見出しが足りない: "
            + "、".join(missing)
            + f"\nひな形は hmwr pr template {args.kind} で出せる",
            proc.USAGE,
        )
    if not re.match(r"^(feat|fix|docs|chore)(\(.+\))?!?: ", args.title):
        raise proc.Fail("件名は feat: / fix: / docs: / chore: のどれかで始める", proc.USAGE)

    argv = ["gh", "pr", "create", "--title", args.title, "--body-file", str(body_file)]
    if args.draft:
        argv.append("--draft")
    return proc.run(argv, dry_run=args.dry_run)
