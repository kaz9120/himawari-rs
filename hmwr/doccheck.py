"""文書どうしの整合を検査する（ADR-0209）。

textlintは1文ずつの書き方を見る。ここが見るのは文書の間の食い違いで、
どれも「索引だけを見て誤読する」「リンクを辿って行き止まる」事故につながる。

- ADR本文のStatusと、索引（docs/adr/README.md）の表の一致
- ADRのファイルと索引の行の過不足
- 文書の相対リンクの行き先
- proposedのまま動いていないADR（期限つきの警告）
"""

from __future__ import annotations

import re
import time
from dataclasses import dataclass
from pathlib import Path

from . import paths, proc

ADR_DIR = "docs/adr"
INDEX = "docs/adr/README.md"
# 索引の表の行: | [0208](0208-x.md) | 題 | 日付 | 置換 | Status |
ROW_RE = re.compile(r"^\| \[(\d{4})\]\(([^)]+)\) \|.*\| ([^|]*) \|\s*$", re.M)
STATUS_RE = re.compile(r"^- Status: (.+)$", re.M)
LINK_RE = re.compile(r"(?<!\!)\[[^\]]*\]\(([^)\s]+)\)")
FENCE_RE = re.compile(r"^(```|~~~).*?^\1", re.M | re.S)
INLINE_CODE_RE = re.compile(r"`[^`\n]*`")
DOC_GLOBS = ("*.md", "docs/**/*.md", ".claude/**/*.md", ".github/**/*.md", "portal/*.md")


@dataclass(frozen=True)
class Finding:
    file: str
    message: str

    def __str__(self) -> str:
        return f"{self.file}: {self.message}"


def status_head(text: str) -> str:
    """Statusの先頭の語。補足の括弧や置換先は比べない。"""
    return re.split(r"[（(\s]", text.strip(), maxsplit=1)[0]


def adr_files(root: Path) -> list[Path]:
    return sorted((root / ADR_DIR).glob("[0-9][0-9][0-9][0-9]-*.md"))


def check_index(root: Path) -> list[Finding]:
    index = (root / INDEX).read_text(encoding="utf-8")
    rows = {m.group(1): (m.group(2), m.group(3)) for m in ROW_RE.finditer(index)}
    found: list[Finding] = []
    seen = set()
    for file in adr_files(root):
        number = file.name[:4]
        seen.add(number)
        rel = f"{ADR_DIR}/{file.name}"
        m = STATUS_RE.search(file.read_text(encoding="utf-8"))
        if m is None:
            found.append(Finding(rel, "Statusの行がない"))
            continue
        if number not in rows:
            found.append(Finding(rel, "索引の表に行がない"))
            continue
        link, status = rows[number]
        if link != file.name:
            found.append(Finding(INDEX, f"{number} のリンク先がファイル名と違う: {link}"))
        if status_head(m.group(1)) != status_head(status):
            found.append(
                Finding(rel, f"Statusが索引と違う（本文 {m.group(1)!r}、索引 {status.strip()!r}）")
            )
    for number in sorted(set(rows) - seen):
        found.append(Finding(INDEX, f"{number} の行があるが、ADRのファイルがない"))
    return found


def doc_files(root: Path) -> list[Path]:
    files: set[Path] = set()
    for pattern in DOC_GLOBS:
        files.update(root.glob(pattern))
    return sorted(f for f in files if "node_modules" not in f.parts)


def check_links(root: Path) -> list[Finding]:
    """相対リンクの行き先があるかを見る。URLとページ内リンクは見ない。"""
    found: list[Finding] = []
    for file in doc_files(root):
        if file.name == "CHANGELOG.md":
            continue  # release-pleaseが生成する
        text = file.read_text(encoding="utf-8")
        text = INLINE_CODE_RE.sub("", FENCE_RE.sub("", text))
        for m in LINK_RE.finditer(text):
            target = m.group(1)
            if re.match(r"^[a-z][a-z0-9+.-]*:", target) or target.startswith("#"):
                continue
            path = (file.parent / target.split("#", 1)[0]).resolve()
            if root.resolve() not in path.parents and path != root.resolve():
                continue  # リポジトリの外へ出るリンクは、GitHub上の場所（../../releases など）
            if not path.exists():
                found.append(Finding(str(file.relative_to(root)), f"リンク先がない: {target}"))
    return found


def check_stale(root: Path, days: int) -> list[Finding]:
    """proposedのまま、最後のコミットから日数が過ぎたADR。"""
    found: list[Finding] = []
    now = time.time()
    for file in adr_files(root):
        m = STATUS_RE.search(file.read_text(encoding="utf-8"))
        if m is None or status_head(m.group(1)) != "proposed":
            continue
        rel = f"{ADR_DIR}/{file.name}"
        stamp = proc.capture(["git", "log", "-1", "--format=%ct", "--", rel], cwd=root).strip()
        if not stamp.isdigit():
            continue
        age = int((now - int(stamp)) // 86400)
        if age >= days:
            found.append(
                Finding(rel, f"proposedのまま{age}日動いていない。acceptedかrejectedへ進める")
            )
    return found


def run_all(root: Path | None = None, *, stale_days: int = 0) -> list[Finding]:
    root = root or paths.REPO
    found = check_index(root) + check_links(root)
    if stale_days:
        found += check_stale(root, stale_days)
    return found
