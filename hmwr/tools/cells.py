"""局面を区画（手番側の玉の帯 × 形勢の帯）に分け、配分とαで選ぶ（ADR-0227）。

区画は2軸で決める。玉は手番側から見た段で、敵陣（1〜3段）・4〜6段・自陣（7〜9段）の3帯。
形勢は教師の勝率 p = σ(score / scale) で、互角（|p − 0.5| < 0.1）・優勢（< 0.3）・
勝勢（それ以外）の3帯。

区画の中では、ありふれ度のα乗に反比例する確率で選ぶ。ありふれ度は、局面でオンになる
特徴（自玉のマス × bona piece）の出現頻度の幾何平均である。α=0 は自然な分布のまま、
α=1 は同じ形を同じ重みにする。間の値は、中心を残して裾を連続的に厚くする。
"""

from __future__ import annotations

import numpy as np

PSV_BYTES = 40
HAND_MAX = (18, 4, 4, 4, 4, 2, 2)  # 歩香桂銀金角飛
HOFF = np.cumsum((0,) + HAND_MAX)
NB = 2 * 14 * 81
NBP = NB + 2 * int(HOFF[-1])
NFEAT = 81 * NBP
KING_BANDS = ("敵陣（1〜3段）", "4〜6段", "自陣（7〜9段）")
EVAL_BANDS = ("互角", "優勢", "勝勢")


def king_band(rank: np.ndarray) -> np.ndarray:
    """手番側から見た段（1=敵陣の奥、9=自陣の最下段）を3帯へ。"""
    return np.where(rank <= 3, 0, np.where(rank <= 6, 1, 2)).astype(np.int8)


def eval_band(score: np.ndarray, scale: float) -> np.ndarray:
    p = 1.0 / (1.0 + np.exp(-score.astype(np.float64) / scale))
    d = np.abs(p - 0.5)
    return np.where(d < 0.1, 0, np.where(d < 0.3, 1, 2)).astype(np.int8)


def scores(rec: np.ndarray) -> np.ndarray:
    return np.ascontiguousarray(rec[:, 32:34]).view(np.int16).ravel()


def decode(rec: np.ndarray, *, features: bool = True):
    """(n,40) の局面から、手番側の玉の段と特徴の並びを取り出す。

    特徴は手番側から見た向きにそろえる（後手番なら盤を180度回し、駒の色を入れ替える）。
    返り値は (段, 特徴の連結, 局面ごとの特徴数)。features=False なら段だけを返す。
    """
    import cshogi

    n = len(rec)
    board = cshogi.Board()
    rank = np.zeros(n, np.int8)
    feats: list[np.ndarray] = []
    lens = np.zeros(n, np.int32)
    for i in range(n):
        board.set_psfen(np.ascontiguousarray(rec[i, :32]))
        side = board.turn
        sq = board.king_square(side)
        rank[i] = sq % 9 + 1 if side == 0 else 9 - sq % 9
        if not features:
            continue
        pcs = np.asarray(board.pieces)
        if side == 1:
            pcs = pcs[::-1]
            pcs = np.where(pcs == 0, 0, np.where(pcs >= 17, pcs - 16, pcs + 16))
        sqs = np.nonzero(pcs)[0]
        pc = pcs[sqs]
        own = pc < 17
        kind = np.where(own, pc, pc - 16) - 1
        myk = int(sqs[pc == cshogi.BKING][0])
        keep = ~((pc == cshogi.BKING) | (pc == cshogi.WKING))
        idx = list(((np.where(own, 0, 1) * 14 + kind) * 81 + sqs)[keep])
        hand = board.pieces_in_hand
        mine, theirs = hand[side], hand[1 - side]
        for s, h in ((0, mine), (1, theirs)):
            for t in range(7):
                for k in range(min(h[t], HAND_MAX[t])):
                    idx.append(NB + s * int(HOFF[-1]) + int(HOFF[t]) + k)
        f = np.asarray(idx, np.int64) + myk * NBP
        feats.append(f)
        lens[i] = len(f)
    flat = np.concatenate(feats) if feats else np.zeros(0, np.int64)
    return rank, flat, lens


def log_typicality(flat: np.ndarray, lens: np.ndarray, logf: np.ndarray) -> np.ndarray:
    """局面ごとの特徴の対数頻度の平均（ありふれ度の対数）。"""
    starts = np.concatenate([[0], np.cumsum(lens)[:-1]])
    return np.add.reduceat(logf[flat], starts) / np.maximum(lens, 1)


def targets(inventory: np.ndarray, king: list[float], evals: list[float], count: int) -> np.ndarray:
    """区画ごとの目標の局面数。在庫が足りない区画は全部を取り、不足分を同じ玉の帯の
    優勢へ回す。優勢でも足りなければ勝勢へ回す。

    inventory: (3,3) の在庫（玉の帯 × 形勢の帯）。返り値も (3,3)。
    """
    if abs(sum(king) - 1) > 1e-6 or abs(sum(evals) - 1) > 1e-6:
        raise ValueError("配分の和は1にする")
    want = np.outer(king, evals) * count
    got = np.minimum(want, inventory)
    for k in range(3):
        short = (want[k] - got[k]).sum()
        for e in (1, 2, 0):
            if short <= 0:
                break
            room = inventory[k, e] - got[k, e]
            add = min(room, short)
            got[k, e] += add
            short -= add
    return got


def solve_scale(logt: np.ndarray, weight: np.ndarray, need: float, alpha: float) -> float:
    """π = min(1, c × exp(−α (logt − max))) の重み付き和が need になる c を二分法で求める。"""
    have = weight.sum()
    if need >= have:
        return float("inf")
    raw = np.exp(-alpha * (logt - logt.max()))
    lo, hi = 0.0, 1.0
    while (np.minimum(1, hi * raw) * weight).sum() < need:
        hi *= 2
    for _ in range(80):
        mid = (lo + hi) / 2
        if (np.minimum(1, mid * raw) * weight).sum() < need:
            lo = mid
        else:
            hi = mid
    return (lo + hi) / 2
