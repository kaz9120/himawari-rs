"""領域ごとのコマンド。

各モジュールが `add_parser(sub)` を持ち、自分のサブコマンドを登録する。
`cli.py` はそれを並べるだけで、領域ごとの事情を知らない。
"""

from . import analyze, book, build, ci, data, doc, env, kifu, measure, net, sprt

MODULES = (env, build, sprt, measure, net, data, book, kifu, analyze, ci, doc)

__all__ = [
    "MODULES",
    "analyze",
    "book",
    "build",
    "ci",
    "data",
    "doc",
    "env",
    "kifu",
    "measure",
    "net",
    "sprt",
]
