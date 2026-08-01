"""scripts/ 配下のハイフン付きファイル名をテストからimportするための補助。

`scripts/sprt-summary.py` のようにファイル名にハイフンを含むため、
`import sprt_summary` はできない。importlib で明示的にロードする。
"""

import importlib.util
import pathlib
import sys

SCRIPTS_DIR = pathlib.Path(__file__).resolve().parent.parent


def load_module(module_name, filename):
    """scripts/<filename> をmodule_nameとしてロードして返す。"""
    path = SCRIPTS_DIR / filename
    spec = importlib.util.spec_from_file_location(module_name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module
