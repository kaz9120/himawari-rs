"""配布物のPGOが、現行の評価関数で学習走行することを検証する。

`release.yml` はネットをReleaseのタグから取るので、`hmwr/config.py` の
`EVAL_FILE` と二重に持っている。ネットを切り替えたPRで片方だけ変えると、
配布物が古いネットでプロファイルを取る（issue #645。net-v7のまま4世代残った）。
"""

import re
from pathlib import Path

from hmwr import config, paths


def test_release_pgo_uses_the_current_eval_file():
    text = (paths.REPO / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
    m = re.search(r"^\s*NET_ASSET:\s*(\S+)\s*$", text, re.MULTILINE)
    assert m, "release.yml に NET_ASSET がない"
    assert m.group(1) == Path(config.EVAL_FILE).name, (
        "release.yml の NET_TAG と NET_ASSET を、EVAL_FILE のネットを配ったReleaseへ揃える"
    )
