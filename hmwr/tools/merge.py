"""シャッフル済みのpsvを、ランダムに織り交ぜて1本にする（ADR-0228）。

一様にシャッフルされた列どうしを、残り件数に比例した確率で1件ずつ取り出して並べると、
和集合の一様なシャッフルになる。全体シャッフルをやり直さずに混ぜられる。

入力は末尾から読み、読んだ分だけファイルを切り詰める。出力は入力の末尾側から順に
並ぶが、一様な並びを逆にしても一様なので構わない。ディスクの空きは、出力1本ぶんと
読みかけの区間だけで済む。大きな母集団へ新しい局面を足すときに使う。
"""

from __future__ import annotations

import os
from pathlib import Path

import numpy as np

PSV_BYTES = 40
BLOCK = 4_000_000


def merge(sources: list[Path], part: Path, *, seed: int, consume: bool, progress=None) -> dict:
    sizes = [p.stat().st_size for p in sources]
    for p, s in zip(sources, sizes):
        if s % PSV_BYTES:
            raise ValueError(f"大きさが40の倍数でない: {p}")
    left = np.array([s // PSV_BYTES for s in sizes], dtype=np.int64)
    total = int(left.sum())
    rng = np.random.default_rng(seed)
    handles = [open(p, "r+b" if consume else "rb") for p in sources]
    # 入力ごとに、末尾から読んだがまだ出していない局面を持つ
    buf = [np.zeros((0, PSV_BYTES), np.uint8) for _ in sources]
    written = 0
    try:
        with open(part, "wb") as out:
            while left.sum() > 0:
                k = int(min(BLOCK, left.sum()))
                take = rng.multivariate_hypergeometric(left, k)
                rows = []
                for i, t in enumerate(take):
                    if t == 0:
                        continue
                    while len(buf[i]) < t:
                        # 末尾からまだ読んでいない区間の最後のかたまりを読む
                        unread = int(left[i]) - len(buf[i])
                        n = min(BLOCK, unread)
                        start = (unread - n) * PSV_BYTES
                        handles[i].seek(start)
                        chunk = np.frombuffer(handles[i].read(n * PSV_BYTES), np.uint8).reshape(-1, PSV_BYTES)
                        if consume:
                            handles[i].truncate(start)
                        buf[i] = np.concatenate([chunk, buf[i]])
                    rows.append(buf[i][-t:])
                    buf[i] = buf[i][:-t]
                    left[i] -= t
                block = np.concatenate(rows)
                block = block[rng.permutation(len(block))]
                out.write(block.tobytes())
                written += len(block)
                if progress:
                    progress(written)
    finally:
        for h in handles:
            h.close()
    if consume:
        for p in sources:
            os.remove(p)
    if written != total:
        raise RuntimeError(f"書いた件数が合わない: {written} / {total}")
    return {"read": total, "kept": written}
