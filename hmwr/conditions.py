"""同じ名前の続きへ、違う条件の実行を積まない。

対局も学習も、再開は途中の成果物（棋譜・チェックポイント）の有無だけで
決まる。名前が既存の実行とぶつかると、条件の違う実行が黙って続きに積まれる。
最初の起動で条件を控え、再開のたびに比べる。
"""

from __future__ import annotations

import json
from pathlib import Path

from . import paths, proc


def recorded(record: Path) -> str | None:
    """控えてある条件。記録がなければNone。"""
    if not record.is_file():
        return None
    return json.loads(record.read_text(encoding="utf-8"))["command"]


def check(
    record: Path, line: str, *, resuming: bool, adopt: bool, dry_run: bool, what: str
) -> None:
    """続きから走るなら条件を比べ、新規なら条件を控える。

    whatは途中の成果物の呼び名（「棋譜」「チェックポイント」）で、エラー文に使う。
    """
    before = recorded(record) if resuming else None
    if before is not None:
        if before != line:
            raise proc.Fail(
                f"同じ名前で条件の違う{what}がある: {paths.rel(record.parent)}\n"
                f"前回: {before}\n今回: {line}\n名前を変える"
            )
        return
    if resuming and not adopt:
        raise proc.Fail(
            f"条件の記録がない{what}がある: {paths.rel(record.parent)}\n"
            "続きから走らせるなら --adopt を付ける。条件が同じであることは、"
            "ログの起動行で確かめる"
        )
    if not dry_run:
        record.parent.mkdir(parents=True, exist_ok=True)
        record.write_text(
            json.dumps({"command": line}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )
