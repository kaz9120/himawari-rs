"""領域ごとのコマンド。

各モジュールが `add_parser(sub)` を持ち、自分のサブコマンドを登録する。
`cli.py` はそれを並べるだけで、領域ごとの事情を知らない。
"""

from . import build, data, doc, env, measure, net, sprt

MODULES = (env, build, sprt, measure, net, data, doc)

__all__ = ["MODULES", "build", "data", "doc", "env", "measure", "net", "sprt"]
