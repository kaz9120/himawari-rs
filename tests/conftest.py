"""hmwrパッケージをテストからimportできるようにする。"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))


import pytest  # noqa: E402

from hmwr import paths  # noqa: E402


@pytest.fixture(autouse=True)
def _status_in_tmp(tmp_path_factory, monkeypatch):
    """状態ファイル（ADR-0220）を本物の data/status/ へ書かせない。

    長く走るコマンドは状態ファイルを書くので、コマンドを通すテストはどれも書く。
    個々のテストに任せると、書き忘れが本物の状態を汚す。
    """
    monkeypatch.setattr(paths, "STATUS", tmp_path_factory.mktemp("status"))
