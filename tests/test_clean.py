"""古い成果物の掃除を検証する。"""

import os
import time

from hmwr import paths
from hmwr.commands import clean


def test_old_builds_are_cleaned(monkeypatch):
    # 置き場はconftestのautouse fixtureが一時ディレクトリへ移してある
    for place in (paths.SPRT, paths.BIN, paths.NETS, paths.LOGS, paths.TRAIN):
        place.mkdir(parents=True)
    past = time.time() - 60 * 86400
    for name in ("base-old",):
        p = paths.BIN / name
        p.write_bytes(b"x")
        os.utime(p, (past, past))
    p = paths.BIN / "base-new"
    p.write_bytes(b"x")

    removed = {p.name for kind, p in clean._candidates(30) if kind == "バイナリ"}
    assert removed == {"base-old"}
