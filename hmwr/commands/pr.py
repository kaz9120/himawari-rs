"""PRを非対話で作る（ADR-0070）。

`gh pr create --template` はエディタを開く前提で、`--body-file` と併用できない。
エージェントは非対話で走るので、テンプレートの中身を本文へ手で書き写していた。
ここでは、本文にテンプレートの見出しが揃っていることを確かめてから `gh` を呼ぶ。
**種別ごとの記入欄を飛ばせない形にすることが目的である。**
"""

from __future__ import annotations

import argparse
import json
import re
import tempfile
from pathlib import Path

from .. import paths, proc, sprt_log

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

    t = ss.add_parser(
        "merge",
        help="squashの本文とSPRTの結果を検査してからマージする",
        description="squashで入るコミットの件名と本文を組み立て、入れ子の半角括弧を検査する。"
        "strengthなら data/sprt/<名前>.result がH1で、本文の SPRT: トレーラが結果と"
        "一致することも確かめる。CIが緑で、全部通ったら gh pr merge --squash "
        "--delete-branch を呼ぶ。--dry-run では検査だけ行い、マージしない。",
    )
    t.add_argument("number", type=int, metavar="PR番号")
    t.add_argument("--kind", required=True, choices=sorted(KINDS), help="chore / strength")
    t.add_argument(
        "--sprt",
        metavar="名前",
        help="strengthのとき必須。data/sprt/<名前>.result の判定を見る",
    )
    t.add_argument(
        "--body-file",
        metavar="ファイル",
        help="squashの本文をこのファイルにする。コミットの多い研究ブランチ向け。検査は同じにかける",
    )
    t.set_defaults(func=merge)


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


# --- マージ ------------------------------------------------------------

# release-please のパーサは入れ子の半角括弧で失敗し、その回のリリースが黙って止まる
NESTED_PARENS_RE = re.compile(r"\([^()]*\(")
CI_PASS = {"SUCCESS", "SKIPPED", "NEUTRAL"}


def squash_message(pr: dict) -> tuple[str, str]:
    """squashで入るコミットの (件名, 本文) を組み立てる。

    リポジトリの設定（件名は COMMIT_OR_PR_TITLE、本文は COMMIT_MESSAGES）に
    倣う。GitHubに任せず自前で組み、同じものを gh へ渡す。検査した文面と
    mainへ入る文面を一致させるためである。
    """
    commits = pr["commits"]
    number = pr["number"]
    if len(commits) == 1:
        c = commits[0]
        return f"{c['messageHeadline']} (#{number})", c["messageBody"].strip()
    body = "\n\n".join(
        f"* {c['messageHeadline']}\n\n{c['messageBody'].strip()}".rstrip() for c in commits
    )
    return f"{pr['title']} (#{number})", body


def read_result(path: Path) -> dict[str, str]:
    """結果ファイル（key=value）を読む（ADR-0175）。"""
    fields = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        key, sep, value = line.partition("=")
        if sep:
            fields[key] = value
    return fields


def expected_trailer(result: dict[str, str]) -> str:
    """結果ファイルから、本文に載るべき SPRT: トレーラを作る（ADR-0071）。

    注記は `sprt_log.hypothesis_note()` に任せる。トレーラを書く側と
    検査する側で別々に組むと、既定でも非劣性でもない対立仮説（elo1=10の
    加速SPRTなど）で食い違い、マージが止まる。
    """
    line = (
        f"SPRT: {result['elo']} [{result['ci_low']},{result['ci_high']}] "
        f"{result['games']}games {result['decision']}"
    )
    elo0, elo1 = result.get("elo0"), result.get("elo1")
    hyp = (elo0, elo1) if elo0 is not None and elo1 is not None else None
    return line + sprt_log.hypothesis_note(hyp)


def problems(kind: str, subject: str, body: str, result: dict[str, str] | None) -> list[str]:
    """マージを止める理由を並べる。空ならマージしてよい。"""
    found = []
    for label, text in (("件名", subject), ("本文", body)):
        for line in text.splitlines():
            if NESTED_PARENS_RE.search(line):
                found.append(f"{label}に入れ子の半角括弧がある: {line.strip()}")
    is_feat = re.match(r"^feat(\(.+\))?!?: ", subject) is not None
    if kind == "chore" and is_feat:
        found.append("件名が feat: で始まる。棋力の変わる変更は --kind strength で検査する")
    if kind == "strength":
        if not is_feat:
            found.append("棋力向上のPRの件名は feat: で始める（ADR-0071）")
        if result is None:
            found.append("SPRTの結果ファイルがない（--sprt <名前> で data/sprt/<名前>.result を指す）")
        else:
            if result.get("decision") != "H1":
                found.append(f"SPRTの判定が H1 ではない: {result.get('decision')}")
            want = expected_trailer(result)
            # トレーラはrelease-pleaseが最後の段落から読む。途中の段落にあると載らない
            last = body.strip().split("\n\n")[-1].splitlines()
            if not any(line.strip() == want for line in last):
                found.append(f"本文の最後の段落に、結果と一致するトレーラがない: {want}")
    return found


def ci_green(pr: dict) -> bool:
    """全チェックが成功か省略で終わっていればTrue。1件も無いならFalse。"""
    checks = pr.get("statusCheckRollup") or []
    if not checks:
        return False
    for c in checks:
        state = c.get("conclusion") or c.get("state") or ""
        if state.upper() not in CI_PASS:
            return False
    return True


def merge(args: argparse.Namespace) -> int:
    if args.kind == "strength" and not args.sprt:
        raise proc.Fail("--kind strength には --sprt <名前> が要る", proc.USAGE)
    fields = "number,title,state,isDraft,headRefOid,commits,statusCheckRollup"
    code, out, err = proc.capture_code(
        ["gh", "pr", "view", str(args.number), "--json", fields]
    )
    if code != proc.OK:
        raise proc.Fail(f"PR#{args.number} を読めない: {err.strip()}")
    pr = json.loads(out)

    if pr["state"] != "OPEN":
        raise proc.Fail(f"PR#{args.number} は {pr['state']} で、マージできない", proc.JUDGE)
    if pr["isDraft"]:
        raise proc.Fail(f"PR#{args.number} はドラフトである", proc.JUDGE)

    result = None
    if args.sprt:
        path = paths.SPRT / f"{args.sprt}.result"
        if path.is_file():
            result = read_result(path)
    subject, body = squash_message(pr)
    if args.body_file:
        # 研究ブランチは数十件のコミットを持つ。全部をつなげた本文は、古いコミットの
        # 入れ子の括弧を拾い、トレーラも最後の段落から外れる（ADR-0241の取り込み）
        body_file = Path(args.body_file)
        if not body_file.is_file():
            raise proc.Fail(f"本文のファイルがない: {args.body_file}", proc.USAGE)
        body = body_file.read_text(encoding="utf-8").strip()
    found = problems(args.kind, subject, body, result)
    if not ci_green(pr):
        found.append("CIが緑ではない（hmwr ci wait で確定を待つ）")
    if found:
        raise proc.Fail(
            f"PR#{args.number} をマージしない:\n" + "\n".join(f"  - {p}" for p in found),
            proc.JUDGE,
        )

    print(f"検査は通った。件名: {subject}\n--- 本文 ---\n{body}\n------------", flush=True)
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", suffix=".md") as fh:
        fh.write(body + "\n")
        fh.flush()
        # 検査した先端と違うものが入らないよう、先端のコミットを固定して渡す
        argv = [
            "gh", "pr", "merge", str(args.number), "--squash", "--delete-branch",
            "--match-head-commit", pr["headRefOid"],
            "--subject", subject, "--body-file", fh.name,
        ]  # fmt: skip
        return proc.run(argv, dry_run=args.dry_run)
