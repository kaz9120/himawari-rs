"""リポジトリの場所と、名前から決まる置き場を集める。

**名前を検証するのはここだけである。** ログ・棋譜・バイナリのファイル名は
すべて実験名から機械的に決まるので、入口で1回検査すれば以後は信用できる。
"""

from __future__ import annotations

import re
import unicodedata
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

BIN = REPO / "data" / "bin"
LOGS = REPO / "data" / "logs"
NETS = REPO / "data" / "nets"
SPRT = REPO / "data" / "sprt"
SPSA = REPO / "data" / "spsa"
BOOK = REPO / "data" / "book"
TRAIN = REPO / "data" / "train"
PROFILE = REPO / "data" / "profile"
RAW = REPO / "data" / "raw"

SCRIPTS = REPO / "scripts"
CHECKPOINTS = REPO / "training" / "checkpoints"

# 実験名。ファイル名になるので、パス区切りと空白を弾く。ネット名は
# アンダースコアを含むため（pairprod_2990M_q1）、そこは許す
NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")


class BadName(ValueError):
    """実験名として使えない文字列。"""


def check_name(name: str) -> str:
    """実験名を検証する。ここを通った名前だけがファイル名になる。"""
    if not NAME_RE.match(name):
        raise BadName(
            f"実験名に使えない文字がある: {name!r}\n"
            "英数字で始め、英数字・ハイフン・アンダースコア・ドットだけを使う"
        )
    return name


def log(area: str, name: str) -> Path:
    """ログの置き場を決める。呼び出し側はリダイレクト先を書かない。

    領域のプレフィックスを機械的に付けることで、`data/logs/` を見たときに
    何の記録かが名前から分かる。
    """
    LOGS.mkdir(parents=True, exist_ok=True)
    return LOGS / f"{area}-{check_name(name)}.log"


def rel(path: str | Path) -> str:
    """リポジトリの中のパスは相対で見せる。表示が長いと読み飛ばされる。"""
    s = str(path)
    prefix = f"{REPO}/"
    return s[len(prefix) :] if s.startswith(prefix) else s


def display_width(text: str) -> int:
    """端末での表示幅。全角を2桁と数える。"""
    return sum(2 if unicodedata.east_asian_width(c) in "WF" else 1 for c in text)


def pad(text: str, width: int) -> str:
    """表示幅で右を埋める。表の桁を揃えるために使う。"""
    return text + " " * max(width - display_width(text), 1)
