"""配布物のPGOが、現行の評価関数で学習走行することを検証する。

`release.yml` はネットをReleaseのタグから取るので、`hmwr/config.py` の
`EVAL_FILE` と二重に持っている。ネットを切り替えたPRで片方だけ変えると、
配布物が古いネットでプロファイルを取る（issue #645。net-v7のまま4世代残った）。

net-v16からアセットは固定名になる（ADR-0080の追記、issue #744）。
それ以前のReleaseは学習時の名前のまま残るので、タグの番号で期待値を分ける。
"""

import re
from pathlib import Path

from hmwr import config, paths


def _release_env(key: str) -> str:
    text = (paths.REPO / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
    m = re.search(rf"^\s*{key}:\s*(\S+)\s*$", text, re.MULTILINE)
    assert m, f"release.yml に {key} がない"
    return m.group(1)


def test_release_pgo_uses_the_current_eval_file():
    tag = _release_env("NET_TAG")
    assert tag == config.EVAL_TAG, (
        "release.yml の NET_TAG を、EVAL_FILE のネットを配ったRelease（config.EVAL_TAG）へ揃える"
    )
    number = int(tag.removeprefix("net-v"))
    expect = Path(config.EVAL_FILE).name if number <= config.LAST_NAMED_NET else config.NET_ASSET
    assert _release_env("NET_ASSET") == expect, (
        "release.yml の NET_ASSET を、そのReleaseのアセット名へ揃える"
    )
