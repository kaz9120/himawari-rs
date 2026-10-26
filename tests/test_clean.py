"""古い成果物の掃除（ADR-0189）を検証する。

実験のspecが `--build` で名指しする比較用ビルドは、日数を過ぎても残す。
測定台のビルドを消して、後の実験の対局が止まった（2026-10-07、ADR-0226）。
"""

import os
import time

from hmwr import paths, spec
from hmwr.commands import clean

SPEC = '''
adr = "0226"

[[step]]
id = "match"
run = "hmwr match run x --build bench-himawari --base-net a --cand-net b --stop pairs:10 --foreground"
'''


def test_old_builds_are_cleaned_unless_a_spec_names_them(monkeypatch):
    # 置き場はconftestのautouse fixtureが一時ディレクトリへ移してある
    for place in (paths.SPRT, paths.BIN, paths.NETS, paths.LOGS, paths.TRAIN):
        place.mkdir(parents=True)
    monkeypatch.setattr(spec, "names", lambda: ["x"])
    monkeypatch.setattr(spec, "load", lambda name, ref="origin/main": spec.parse(SPEC, name))
    past = time.time() - 60 * 86400
    for name in ("bench-himawari", "base-old"):
        p = paths.BIN / name
        p.write_bytes(b"x")
        os.utime(p, (past, past))

    assert clean._spec_builds() == {"bench-himawari"}
    removed = {p.name for kind, p in clean._candidates(30) if kind == "バイナリ"}
    assert removed == {"base-old"}
