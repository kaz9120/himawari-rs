"""SPSAの数理（ADR-0143）。

同時摂動による確率近似。全定数を同時に±cだけ摂動した2つの設定を
対局させ、勝敗の差から勾配を推定してθを動かす。式はfishtestのSPSA
チューナーに合わせ、ステップ幅と摂動幅は終端値（r_end・c_end）で
指定する。ペア番号kが進むほど摂動は細く、歩幅は小さくなる。

ここには純関数だけを置き、対局の起動やファイル入出力は
commands/spsa.py が持つ。乱数はペア番号から決定論で引くので、
中断・再開しても同じ摂動列をたどる。
"""

from __future__ import annotations

import random
from dataclasses import dataclass

# fishtestの既定と同じ減衰指数。alphaが歩幅、gammaが摂動幅の減衰
ALPHA = 0.602
GAMMA = 0.101
# 歩幅の分母に足す安定化項Aは、総ペア数の1割に置く（fishtestと同じ）
A_RATIO = 0.1


@dataclass(frozen=True)
class Param:
    """チューニング対象1項目。可動域はエンジン側の宣言と揃える。"""

    name: str
    default: float
    lo: float
    hi: float
    c_end: float
    r_end: float


def schedule(k: int, total_pairs: int) -> tuple[float, float]:
    """ペア番号k（1始まり）での倍率（c_mult, r_mult）を返す。

    c_k = c_end * c_mult、r_k = r_end * r_mult。k = total_pairs で
    どちらも1になり、指定した終端値そのものになる。
    """
    if not 1 <= k:
        raise ValueError(f"kは1始まり: {k}")
    n = float(total_pairs)
    c_mult = (n / k) ** GAMMA
    a = A_RATIO * n
    r_mult = ((a + n) / (a + k)) ** ALPHA
    return c_mult, r_mult


def deltas(seed: int, k: int, params: list[Param]) -> dict[str, int]:
    """ペア番号kの摂動方向（±1）を引く。seedとkだけで決まる。"""
    rng = random.Random(f"{seed}:{k}")
    return {p.name: rng.choice((-1, 1)) for p in params}


def perturbed(
    theta: dict[str, float],
    params: list[Param],
    delta: dict[str, int],
    c_mult: float,
    sign: int,
) -> dict[str, int]:
    """θ±c_kΔをエンジンへ渡す整数値にする。可動域でclampする。"""
    out: dict[str, int] = {}
    for p in params:
        v = theta[p.name] + sign * delta[p.name] * p.c_end * c_mult
        out[p.name] = round(min(max(v, p.lo), p.hi))
    return out


def update(
    theta: dict[str, float],
    params: list[Param],
    delta: dict[str, int],
    c_mult: float,
    r_mult: float,
    score: float,
) -> dict[str, float]:
    """ペアの結果でθを1歩動かす。

    scoreは（θ+側の得点）−（θ−側の得点）で、1ペア2局なら−2〜+2。
    更新は θ += r_k * c_k * Δ * score / 2 で、r = a/c² の関係から
    SPSAの標準形 θ += a * ĝ と同値になる。
    """
    out = dict(theta)
    for p in params:
        step = (p.r_end * r_mult) * (p.c_end * c_mult) * delta[p.name] * score / 2.0
        out[p.name] = min(max(out[p.name] + step, p.lo), p.hi)
    return out


def pair_score(lines: list[dict]) -> float:
    """selfplayのjsonl（1ペア=2行）から（θ+側）−（θ−側）の得点差を出す。

    candidate側をθ+に割り当てる規約なので、candidateの得点合計をsとして
    差は 2s - 局数 になる。
    """
    diff = 0.0
    for rec in lines:
        winner = rec["winner"]
        if winner == "draw":
            continue
        diff += 1.0 if winner == rec["candidate"] else -1.0
    return diff
