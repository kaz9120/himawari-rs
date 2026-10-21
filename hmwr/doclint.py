"""日本語文書の書き方を検査する（ADR-0244）。

同梱したyomiyasu（`hmwr/vendor/yomiyasu`）を各文書へ当て、ゲートにする指摘だけを返す。
コミット件名の検査（`.claude/hooks/check_commits.sh`）が単体で呼べるよう、標準ライブラリ
だけで書く。`python3 hmwr/doclint.py -` で標準入力を検査する。

yomiyasuは指摘を抑える仕組みを持たないので、ここで3つ足す。

- 文書全体の比率を見る規則を外す（箇条書き・太字の割合。ADRのひな形が箇条書きを使う）
- 本リポジトリの用語として文字どおりに使う語を通す（`data ledger` の台帳、SPRTゲート、利きの効く）
- 直前の行の `<!-- yomiyasu-disable-next-line -->` で、その行の指摘を外す（引用など）
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO / "hmwr" / "vendor" / "yomiyasu"))

import yomiyasu_lint  # noqa: E402

# 検査する文書。CHANGELOG.mdはrelease-pleaseが生成し、元のコミット件名をhookが検査する
DIRS = ("docs/", ".claude/", "research/")
EXCLUDE = {"CHANGELOG.md"}

SKIP_RULES = {"excess_list", "excess_bold"}
# infoのうち止めるもの。太字のラベルを並べた箇条書きは、AIの文に出やすい
BLOCKING_INFO = {"bold_label_list"}
ALLOW_WORDS = {"台帳", "ゲート", "効く"}
DISABLE = "<!-- yomiyasu-disable-next-line -->"
WORD_RE = re.compile(r"「(.+?)」")


def targets(root: Path = REPO) -> list[Path]:
    out = subprocess.run(
        ["git", "-C", str(root), "ls-files", "--cached", "--others", "--exclude-standard", "*.md"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.split()
    files = [p for p in out if (p.startswith(DIRS) or "/" not in p) and p not in EXCLUDE]
    return sorted(root / p for p in dict.fromkeys(files) if (root / p).exists())


def blocking(finding: dict) -> bool:
    rule = finding.get("rule")
    if rule in SKIP_RULES:
        return False
    if finding.get("severity") not in ("warn", "error") and rule not in BLOCKING_INFO:
        return False
    if rule in ("slop_vocabulary", "metaphor_verb"):
        m = WORD_RE.search(finding.get("message", ""))
        if m and m.group(1) in ALLOW_WORDS:
            return False
    return True


def lint(text: str) -> list[dict]:
    lines = text.splitlines()
    found = []
    for f in yomiyasu_lint.lint_text(text)["findings"]:
        line = f.get("line") or 0
        if 2 <= line <= len(lines) + 1 and lines[line - 2].strip() == DISABLE:
            continue
        if blocking(f):
            found.append(f)
    return found


def format_finding(name: str, f: dict) -> str:
    return f"{name}:{f.get('line')}: {f['rule']}: {f.get('message', '')}\n    {f.get('snippet', '')}"


def main(argv: list[str]) -> int:
    """`-` なら標準入力、引数が無ければ対象の全文書を検査する。指摘があれば1を返す。"""
    if argv == ["-"]:
        sources = [("<stdin>", sys.stdin.read())]
    else:
        files = [Path(a) for a in argv] or targets()
        sources = [(str(p.relative_to(REPO) if p.is_relative_to(REPO) else p), p.read_text(encoding="utf-8")) for p in files]
    count = 0
    for name, text in sources:
        for f in lint(text):
            print(format_finding(name, f))
            count += 1
    if count:
        print(f"\n{count}件の指摘がある（yomiyasu {yomiyasu_version()}）")
        return 1
    return 0


def yomiyasu_version() -> str:
    return (REPO / "hmwr" / "vendor" / "yomiyasu" / "VERSION").read_text(encoding="utf-8").split()[0]


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
