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
STATUS = REPO / "data" / "status"  # 心拍（ADR-0220）
PROFILE = REPO / "data" / "profile"
RAW = REPO / "data" / "raw"
QUEUE = REPO / "data" / "queue"

SCRIPTS = REPO / "scripts"
CHECKPOINTS = REPO / "training" / "checkpoints"

# 実験名。ファイル名になるので、パス区切りと空白を弾く。ネット名は
# アンダースコアを含むため（pairprod_2990M_q1）、そこは許す
NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")

# ネットワークの構成。<FT>x<L1>[x<L2>[x<L3>]] の形だけを通す（512x16x64など）
ARCH_RE = re.compile(r"^\d+x\d+(x\d+){0,2}$")


class BadName(ValueError):
    """実験名として使えない文字列。"""


class BadArch(BadName):
    """ネットワークの構成として使えない文字列。

    BadNameを継承するのは、入口の `cli.main` が1か所で拾って終了コード2を
    返すためである。検査を足すたびにcliのexcept節を増やさない。
    """


def check_name(name: str) -> str:
    """実験名を検証する。ここを通った名前だけがファイル名になる。"""
    if not NAME_RE.match(name):
        raise BadName(
            f"実験名に使えない文字がある: {name!r}\n"
            "英数字で始め、英数字・ハイフン・アンダースコア・ドットだけを使う"
        )
    return name


def check_arch(spec: str) -> str:
    """ネットワークの構成を検証する。ビルドと学習で同じ書式を使う。"""
    if not ARCH_RE.match(spec):
        raise BadArch(f"構成の書き方が違う: {spec}（<FT>x<L1>[x<L2>[x<L3>]]）")
    return spec


def log(area: str, name: str) -> Path:
    """ログの置き場を決める。呼び出し側はリダイレクト先を書かない。

    領域のプレフィックスを機械的に付けることで、`data/logs/` を見たときに
    何の記録かが名前から分かる。
    """
    LOGS.mkdir(parents=True, exist_ok=True)
    return LOGS / f"{area}-{check_name(name)}.log"


def release_bin(name: str) -> Path:
    """`cargo build --release` が置くバイナリの場所を決める。

    呼び出し側が `target/release` を書かないようにする。出力先の命名が
    変わったとき、直す場所をここ1か所にするためである。
    """
    return REPO / "target" / "release" / name


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
