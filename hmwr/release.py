"""GitHub Releaseを作る共通の骨格。

**外から見える操作は、既定で実行しない。** 走るはずのコマンドとノート本文を
出して終わり、`--apply` を付けたときだけ作る。2026-08-01に、動作確認の
つもりで実際のリリースを作ってしまった事故がある（ADR-0122）。

最初は「予行演習の指定があるときだけ安全」にしたが、それでは思い出した人
しか守られない。既定を逆にすれば、忘れても事故にならない。忘れて困るのは
「作ったつもりが作られていない」ときだけで、そちらは出力を見れば分かる。
"""

from __future__ import annotations

import shutil
import subprocess
import tempfile
from pathlib import Path

from . import paths, proc


def check_version(version: int) -> None:
    if version < 1:
        raise proc.Fail(f"バージョン番号は1以上にする: {version}", proc.USAGE)


def check_prereqs(tag: str, *, dry_run: bool) -> None:
    """ghの存在とタグの重複を確かめる。"""
    if shutil.which("gh") is None:
        raise proc.Fail("gh コマンドが要る")
    if not dry_run and proc.succeeds(["gh", "release", "view", tag]):
        raise proc.Fail(f"{tag} は既にある。番号を上げる")


def file_size(path: Path) -> str:
    size = path.stat().st_size
    for unit in ("B", "K", "M", "G"):
        if size < 1024 or unit == "G":
            return f"{size:.0f}{unit}" if unit == "B" else f"{size:.1f}{unit}"
        size /= 1024
    return f"{size:.1f}G"


def create(
    tag: str,
    title: str,
    notes: str,
    assets: list[Path],
    *,
    apply: bool,
    dry_run: bool,
) -> int:
    """リリースを作る。applyでないときは内容を出して終わる。"""
    if not apply or dry_run:
        print("予行演習のため作成しない。実行するには --apply を付ける")
        print(
            "gh release create "
            + tag
            + " "
            + " ".join(paths.rel(a) for a in assets)
            + f" --title {title} --notes-file <ノート> --latest=false"
        )
        print("--- ノート本文 ---")
        print(notes)
        return proc.OK

    with tempfile.TemporaryDirectory() as tmp:
        notes_file = Path(tmp) / "notes.md"
        notes_file.write_text(notes, encoding="utf-8")
        argv = [
            "gh", "release", "create", tag,
            *[str(a) for a in assets],
            "--title", title,
            "--notes-file", str(notes_file),
            # 利用者が最初に見るのはエンジン本体であるべきなので、
            # 別系統のリリースを「最新」にしない
            "--latest=false",
        ]
        code = subprocess.call(argv, cwd=str(paths.REPO))
    if code != 0:
        raise proc.Fail(f"リリースの作成に失敗した（終了コード {code}）", code)
    url = proc.capture(["gh", "release", "view", tag, "--json", "url", "--jq", ".url"])
    print(f"作成した: {url.strip()}")
    return proc.OK
