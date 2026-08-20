"""scripts/ 配下のハイフン付きファイル名をテストからimportするための補助。

`scripts/sprt-summary.py` のようにファイル名にハイフンを含むため、
`import sprt_summary` はできない。importlib で明示的にロードする。
"""

import importlib.util
import pathlib
import sys
from importlib.machinery import SourceFileLoader

SCRIPTS_DIR = pathlib.Path(__file__).resolve().parent.parent


def load_module(module_name, filename):
    """scripts/<filename> をmodule_nameとしてロードして返す。

    拡張子のないファイル（`scripts/hmwr`）は、拡張子からローダーを推測
    できない。SourceFileLoader を明示して読む。
    """
    path = SCRIPTS_DIR / filename
    loader = SourceFileLoader(module_name, str(path))
    spec = importlib.util.spec_from_loader(module_name, loader)
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    loader.exec_module(module)
    return module
