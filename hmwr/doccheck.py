"""文書の相対リンクの行き先を検査する。

リンクを辿って行き止まる事故を防ぐ。
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path

from . import paths

LINK_RE = re.compile(r"(?<!\!)\[[^\]]*\]\(([^)\s]+)\)")
FENCE_RE = re.compile(r"^(```|~~~).*?^\1", re.M | re.S)
INLINE_CODE_RE = re.compile(r"`[^`\n]*`")
DOC_GLOBS = ("*.md", "docs/**/*.md", ".claude/**/*.md", ".github/**/*.md")


@dataclass(frozen=True)
class Finding:
    file: str
    message: str

    def __str__(self) -> str:
        return f"{self.file}: {self.message}"


def doc_files(root: Path) -> list[Path]:
    files: set[Path] = set()
    for pattern in DOC_GLOBS:
        files.update(root.glob(pattern))
    return sorted(f for f in files if not {"node_modules", "worktrees"} & set(f.parts))


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


def run_all(root: Path | None = None) -> list[Finding]:
    return check_links(root or paths.REPO)
