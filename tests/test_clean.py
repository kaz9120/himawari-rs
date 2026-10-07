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


def test_old_builds_are_cleaned_unless_a_spec_names_them(tmp_path, monkeypatch):
    for sub in ("data/sprt", "data/bin", "data/nets", "data/logs", "data/train"):
        (tmp_path / sub).mkdir(parents=True)
    monkeypatch.setattr(paths, "REPO", tmp_path)
    monkeypatch.setattr(paths, "STATUS", tmp_path / "data/status")
    monkeypatch.setattr(spec, "names", lambda: ["x"])
    monkeypatch.setattr(spec, "load", lambda name, ref="origin/main": spec.parse(SPEC, name))
    past = time.time() - 60 * 86400
    for name in ("bench-himawari", "base-old"):
        p = tmp_path / "data/bin" / name
        p.write_bytes(b"x")
        os.utime(p, (past, past))

    assert clean._spec_builds() == {"bench-himawari"}
    removed = {p.name for kind, p in clean._candidates(30) if kind == "バイナリ"}
    assert removed == {"base-old"}
