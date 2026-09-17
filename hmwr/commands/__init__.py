"""領域ごとのコマンド。

各モジュールが `add_parser(sub)` を持ち、自分のサブコマンドを登録する。
`cli.py` はそれを並べるだけで、領域ごとの事情を知らない。
"""

from . import analyze, book, build, ci, clean, data, diag, doc, env, exp, kifu, match, measure, net, queue, sprt, spsa

MODULES = (env, build, exp, queue, match, sprt, spsa, measure, net, data, book, kifu, analyze, diag, ci, doc, clean)

__all__ = [
    "MODULES",
    "analyze",
    "book",
    "build",
    "ci",
    "clean",
    "data",
    "diag",
    "doc",
    "env",
    "exp",
    "kifu",
    "match",
    "measure",
    "net",
    "queue",
    "sprt",
    "spsa",
]
