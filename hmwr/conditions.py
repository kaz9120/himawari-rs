"""同じ名前の続きへ、違う条件の実行を積まない。

対局も学習も、再開は途中の成果物（棋譜・チェックポイント）の有無だけで
決まる。名前が既存の実行とぶつかると、条件の違う実行が黙って続きに積まれる。
最初の起動で条件を控え、再開のたびに比べる。
"""

from __future__ import annotations

import json
from pathlib import Path

from . import paths, proc


def normalize(line: str) -> str:
    """リポジトリの絶対パスを相対にする。

    worktreeごとに置き場が違うので、絶対パスのまま控えると同じ条件が食い違う。
    """
    repo = paths.REPO
    for root in (repo, repo.resolve()):
        line = line.replace(f"{root}/", "")
    return line


def recorded(record: Path) -> str | None:
    """控えてある条件の行。記録がなければNone。"""
    if not record.is_file():
        return None
    return normalize(json.loads(record.read_text(encoding="utf-8"))["command"])


def recorded_detail(record: Path) -> dict | None:
    """行と一緒に控えた構造化した条件。記録が無いか、行だけを控えた古い記録ならNone。"""
    if not record.is_file():
        return None
    return json.loads(record.read_text(encoding="utf-8")).get("conditions")


def check(
    record: Path,
    line: str,
    *,
    resuming: bool,
    adopt: bool,
    dry_run: bool,
    what: str,
    detail: dict | None = None,
) -> None:
    """続きから走るなら条件を比べ、新規なら条件を控える。

    whatは途中の成果物の呼び名（「棋譜」「チェックポイント」）で、エラー文に使う。
    比べるのは行だけである。detailを渡すと、新規に控えるときに `conditions` の
    鍵で行と並べて控える。読み戻して同じ実行を組み立て直すためのもので、
    行だけの古い記録もそのまま読める。
    """
    line = normalize(line)
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
        content: dict = {"command": line}
        if detail is not None:
            content["conditions"] = detail
        record.write_text(json.dumps(content, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
