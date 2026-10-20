"""マシンごとに変わる設定と、測定の既定条件を持つ。

**既定は測定で決まった結論である。** 数値を変えるときは、なぜ変えるかを
ADRへ書く。環境変数で一時的に上書きできる。

  EVAL_FILE=... hmwr bench <バイナリ>
"""

from __future__ import annotations

import os
import platform
from functools import lru_cache

from . import paths, proc

# 対局ゲートの既定条件（ADR-0028）
SPRT_TC = "10+0.1"
SPRT_ELO0 = "0"
SPRT_ELO1 = "5"
SPRT_ALPHA = "0.05"
SPRT_BETA = "0.05"
SPRT_ADJUDICATE = "2000,8"

# 判定に至らない実行を「見送り」にするペア数の上限（ADR-0217）。判定済みの
# 過去17本は最長8,061ペアで決着しており、それを超えて漂う実行は真のEloが
# 対立仮説の中点の近くにある。H1を採択しないので誤採択率は増えない。
# 10,000ペア＝2万局は8並列の10+0.1で約20時間になる
SPRT_MAX_PAIRS = "10000"

# 対局の置換表と、引き分けにする手数。selfplayの既定と同じ値を明示して渡す。
# チェーンごとに320と400へ割れた事故があり、既定は1か所で持つ（ADR-0208）
MATCH_HASH = "64"
MATCH_MAX_MOVES = "320"

# 現行の評価関数。**ここが一次情報の置き場である**（ROADMAPとREADMEはここを指す。
# ADR-0182）。ネットの世代を替えるときはこの行と EVAL_TAG、配布物のPGOが使う
# .github/workflows/release.yml の NET_TAG・NET_ASSET を更新する（食い違うとpytestが落ちる）
EVAL_FILE = "data/nets/pool1_2ep_f2_reorder.hmwr"
# EVAL_FILE を配ったReleaseのタグ。EVAL_FILE と一緒に替える
EVAL_TAG = "net-v16"
# 配布するネットのアセット名（ADR-0080の追記）。net-v16から固定名で配る。
# エンジンは EvalFile が空のとき、実行ファイルと同じフォルダのこの名前を読む
NET_ASSET = "eval.hmwr"
# 配布する定跡のアセット名（ADR-0080の追記）。形式はやねうら王の定跡のまま。
# エンジンは BookFile が空なら定跡を使わない。置いただけでは読まない
BOOK_ASSET = "book.db"
# 学習時の名前で配った最後の版。これ以前のReleaseは書き換えない
LAST_NAMED_NET = 15
OPENINGS = "openings/start_sfens_ply24.txt"

# 学習の既定の学習データと検証集合。現行の評価関数を学習したときの組（ADR-0228）。
# 評価関数の世代を替えて検証集合も替えたら、ここも揃える。net train・eval・shapesの
# 既定がここを読む
TRAIN_DATA = "data/train/pool1.psv"
TRAIN_VALID = "data/train/valid_tanuki_dl430_q1.psv"

# 計測・対局のビルドフラグ（ADR-0003）
RUSTFLAGS = "-C target-cpu=native"

# 対局は1局1スレッドで回すため、論理プロセッサまで積むと持ち時間の消化が
# 不安定になり測定がぶれる。1コアはOSと計測用に空ける。上限8は既定条件に
# 合わせる。過去の測定と条件を揃えるため、コアが余っていても8を超えない
MAX_CONCURRENCY = 8

DEFAULTS = {
    "SPRT_TC": SPRT_TC,
    "SPRT_ELO0": SPRT_ELO0,
    "SPRT_ELO1": SPRT_ELO1,
    "SPRT_ALPHA": SPRT_ALPHA,
    "SPRT_BETA": SPRT_BETA,
    "SPRT_ADJUDICATE": SPRT_ADJUDICATE,
    "SPRT_MAX_PAIRS": SPRT_MAX_PAIRS,
}


@lru_cache(maxsize=1)
def physical_cores() -> int:
    """物理コア数。ハイパースレッドの論理プロセッサは数えない。"""
    system = platform.system()
    if system == "Darwin":
        # Apple Siliconは高性能コアだけを数える。効率コアは大きく遅く、
        # 混ぜると同じ持ち時間でも到達深さがばらつく
        for key in ("hw.perflevel0.physicalcpu", "hw.physicalcpu"):
            out = proc.capture(["sysctl", "-n", key]).strip()
            if out.isdigit():
                return int(out)
    elif system == "Linux":
        out = proc.capture(["lscpu", "-p=Core,Socket"]).strip()
        rows = {line for line in out.splitlines() if line and not line.startswith("#")}
        if rows:
            return len(rows)
    return os.cpu_count() or 4


def concurrency() -> int:
    """対局の並列度。"""
    override = os.environ.get("SPRT_CONCURRENCY")
    if override and override.isdigit():
        return int(override)
    cores = physical_cores()
    return min(max(cores - 1, 1), MAX_CONCURRENCY)


def get(key: str, default: str = "") -> str:
    """設定を1つ読む。環境変数で明示された値を優先する。"""
    if key in os.environ:
        return os.environ[key]
    if key == "SPRT_CONCURRENCY":
        return str(concurrency())
    if key == "EVAL_FILE":
        return str(paths.REPO / EVAL_FILE)
    if key == "OPENINGS":
        return str(paths.REPO / OPENINGS)
    return DEFAULTS.get(key, default)


def rustflags() -> str:
    """計測・対局のビルドフラグ。"""
    return os.environ.get("RUSTFLAGS") or RUSTFLAGS


def measure_env() -> dict[str, str]:
    """計測ツールへ渡す環境。

    速度と機能検証のツールは評価関数の場所を EVAL_FILE から読む。
    コマンドごとに指定させず、ここで渡す。
    """
    env = {"RUSTFLAGS": rustflags()}
    for key in ("EVAL_FILE", "OPENINGS"):
        if key not in os.environ:
            env[key] = get(key)
    return env


def summary() -> list[tuple[str, str]]:
    """表示用の要約。測る前に条件を確かめるために使う。"""
    return [
        ("物理コア", str(physical_cores())),
        ("対局の並列度", get("SPRT_CONCURRENCY")),
        ("評価関数", paths.rel(get("EVAL_FILE"))),
        ("開始局面", paths.rel(get("OPENINGS"))),
        ("持ち時間", get("SPRT_TC")),
        ("対立仮説", f'elo0={get("SPRT_ELO0")} elo1={get("SPRT_ELO1")}'),
        ("裁定", get("SPRT_ADJUDICATE")),
        ("見送りの上限", f'{get("SPRT_MAX_PAIRS")} ペア'),
        ("ビルドフラグ", rustflags()),
    ]
