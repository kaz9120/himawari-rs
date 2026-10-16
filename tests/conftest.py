"""hmwrパッケージをテストからimportできるようにする。"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))


import pytest  # noqa: E402

from hmwr import paths  # noqa: E402


@pytest.fixture(autouse=True)
def _places_in_tmp(tmp_path_factory, monkeypatch):
    """data/ と training/checkpoints/ の置き場を、本物のリポジトリへ書かせない。

    dry-runでもログの置き場や状態ファイル（ADR-0220）の親を作るので、コマンドを
    通すテストはどれも書く。個々のテストに任せると、書き忘れが本物を汚す。
    置き場はリポジトリと同じ形で一時ディレクトリへ移すので、表示は変わらない。
    """
    work = tmp_path_factory.mktemp("work")
    monkeypatch.setattr(paths, "WORK", work)
    for name in paths.PLACES:
        monkeypatch.setattr(paths, name, work / getattr(paths, name).relative_to(paths.REPO))
