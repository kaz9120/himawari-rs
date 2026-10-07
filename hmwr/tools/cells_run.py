"""`hmwr data pick` と `hmwr data select` の本体（ADR-0227）。

どちらも psv を区間に割り、プロセスを並べて1回だけ読む。並べる数を変えても出力は
変わらない。乱数は（種, 入力の番号, 区間の先頭）で決めるので、区間の大きさを固定すれば
決定的になる。
"""

from __future__ import annotations

from multiprocessing import Pool
from pathlib import Path

import numpy as np

from . import cells

CHUNK = 1_000_000
_G: dict = {}


def _memmap(path: str) -> np.ndarray:
    n = Path(path).stat().st_size // cells.PSV_BYTES
    return np.memmap(path, dtype=np.uint8, mode="r", shape=(n, cells.PSV_BYTES))


# --- pick -------------------------------------------------------------------


def _pick_chunk(args):
    path, lo, hi, king_max, pmin, pmax, scale = args
    rec = np.asarray(_memmap(path)[lo:hi])
    rank, _, _ = cells.decode(rec, features=False)
    p = 1.0 / (1.0 + np.exp(-cells.scores(rec).astype(np.float64) / scale))
    keep = (rank <= king_max) & (p >= pmin) & (p <= pmax)
    return rec[keep].tobytes(), int(keep.sum()), hi - lo


def pick(source: Path, part: Path, *, king_max: int, pmin: float, pmax: float, scale: float,
         jobs: int, progress=None) -> dict:
    """手番側の玉が king_max 段目以内で、勝率が [pmin, pmax] の局面だけを残す。"""
    n = source.stat().st_size // cells.PSV_BYTES
    work = [(str(source), lo, min(lo + CHUNK, n), king_max, pmin, pmax, scale) for lo in range(0, n, CHUNK)]
    kept = seen = 0
    with open(part, "wb") as out, Pool(jobs) as pool:
        for data, k, s in pool.imap(_pick_chunk, work):
            out.write(data)
            kept += k
            seen += s
            if progress:
                progress(seen)
    return {"read": seen, "kept": kept}


# --- select -----------------------------------------------------------------


def _sample_chunk(args):
    path, idx = args
    rec = np.asarray(_memmap(path)[idx])
    rank, flat, lens = cells.decode(rec)
    return rank, cells.scores(rec), flat, lens


def _init_select(logf, scale_c, ref, alpha, eval_scale, seed):
    _G.update(logf=logf, c=scale_c, ref=ref, alpha=alpha, eval_scale=eval_scale, seed=seed)


def _select_chunk(args):
    pool_i, path, lo, hi = args
    rec = np.asarray(_memmap(path)[lo:hi])
    rank, flat, lens = cells.decode(rec)
    cell = cells.king_band(rank) * 3 + cells.eval_band(cells.scores(rec), _G["eval_scale"])
    logt = cells.log_typicality(flat, lens, _G["logf"])
    c, ref = _G["c"][cell], _G["ref"][cell]
    with np.errstate(over="ignore", invalid="ignore"):
        pi = np.where(np.isinf(c), 1.0, np.minimum(1.0, c * np.exp(-_G["alpha"] * (logt - ref))))
    u = np.random.default_rng([_G["seed"], pool_i, lo]).random(len(rec))
    keep = u < pi
    return rec[keep].tobytes(), np.bincount(cell[keep], minlength=9), np.bincount(cell, minlength=9), hi - lo


def select(pools: list[Path], part: Path, *, king: list[float], evals: list[float], count: int,
           alpha: float, eval_scale: float, sample: int, seed: int, jobs: int,
           progress=None, report=print) -> dict:
    sizes = np.array([p.stat().st_size // cells.PSV_BYTES for p in pools], dtype=np.int64)
    # 1. 標本で特徴の頻度表・区画の在庫・区画ごとのありふれ度の分布を見積もる
    share = sizes / sizes.sum()
    per = np.maximum((share * sample).astype(np.int64), np.minimum(sizes, 200_000))
    tasks, weights = [], []
    for i, (p, n, s) in enumerate(zip(pools, sizes, per)):
        idx = np.unique(np.linspace(0, n - 1, int(s)).astype(np.int64))
        for j in range(0, len(idx), 100_000):
            tasks.append((str(p), idx[j:j + 100_000]))
            weights.append(n / len(idx))
    report(f"標本: {int(per.sum()):,}局面（入力 {len(pools)}本、{int(sizes.sum()):,}局面）")
    with Pool(jobs) as pool:
        got = pool.map(_sample_chunk, tasks)
    rank = np.concatenate([g[0] for g in got])
    score = np.concatenate([g[1] for g in got])
    w = np.concatenate([np.full(len(g[0]), wt) for g, wt in zip(got, weights)])
    flat = np.concatenate([g[2] for g in got])
    lens = np.concatenate([g[3] for g in got])
    cnt = np.bincount(flat, weights=np.repeat(w, lens), minlength=cells.NFEAT)
    logf = np.log(np.maximum(cnt, 1.0) / w.sum())
    logt = cells.log_typicality(flat, lens, logf)
    cell = cells.king_band(rank) * 3 + cells.eval_band(score, eval_scale)
    inventory = np.bincount(cell, weights=w, minlength=9).reshape(3, 3)
    target = cells.targets(inventory, king, evals, count)
    c = np.zeros(9)
    ref = np.zeros(9)
    for k in range(9):
        m = cell == k
        if not m.any() or target.flat[k] <= 0:
            c[k], ref[k] = 0.0, 0.0
            continue
        ref[k] = logt[m].max()
        c[k] = cells.solve_scale(logt[m], w[m], float(target.flat[k]), alpha)
    for k in range(3):
        for e in range(3):
            report(
                f"  {cells.KING_BANDS[k]}×{cells.EVAL_BANDS[e]}: 在庫 約{inventory[k, e] / 1e4:,.0f}万 "
                f"目標 {target[k, e] / 1e4:,.0f}万" + ("（全部）" if np.isinf(c[k * 3 + e]) else "")
            )
    # 2. 全件を1回読み、区画ごとの確率で選ぶ
    work = [(i, str(p), lo, min(lo + CHUNK, int(n))) for i, (p, n) in enumerate(zip(pools, sizes))
            for lo in range(0, int(n), CHUNK)]
    kept = np.zeros(9, np.int64)
    seen_cells = np.zeros(9, np.int64)
    seen = 0
    with open(part, "wb") as out, Pool(jobs, initializer=_init_select,
                                       initargs=(logf, c, ref, alpha, eval_scale, seed)) as pool:
        for data, k, s, n in pool.imap(_select_chunk, work):
            out.write(data)
            kept += k
            seen_cells += s
            seen += n
            if progress:
                progress(seen)
    return {
        "read": int(seen),
        "kept": int(kept.sum()),
        "inventory": inventory.round().tolist(),
        "target": target.round().tolist(),
        "kept_by_cell": kept.reshape(3, 3).tolist(),
        "seen_by_cell": seen_cells.reshape(3, 3).tolist(),
    }
