"""PSV dataset reader via Rust PyO3 bridge (ADR-0043, ADR-0045, ADR-0065)."""

import math
import os
import queue
import threading

import numpy as np
import torch
from torch.utils.data import Dataset

import himawari


class PsvDataset(Dataset):
    """Memory-mapped PSV dataset with Rust feature extraction."""

    def __init__(self, path, lambda_=0.7, score_limit=0, mmap=False, score_clamp=0):
        size = os.path.getsize(path)
        if size % 40 != 0:
            raise ValueError(f"ファイルサイズが40の倍数でない: {size}")
        if mmap:
            # RAMに載らない規模用。DataLoaderのshuffle=Trueと組むと
            # ランダムアクセスがページキャッシュを外れて大幅に遅くなる
            self.data = np.memmap(path, dtype=np.uint8, mode="r", shape=(size // 40, 40))
        else:
            self.data = np.fromfile(path, dtype=np.uint8).reshape(-1, 40)
        self.lambda_ = lambda_
        self.score_limit = score_limit
        self.score_clamp = score_clamp

    def __len__(self):
        return len(self.data)

    def __getitem__(self, idx):
        record = bytes(self.data[idx])
        result = himawari.extract_features(
            record, self.lambda_, self.score_limit, self.score_clamp
        )
        if result is None:
            return None
        stm_feats, opp_feats, target = result
        return stm_feats, opp_feats, target


class PsvBatchLoader:
    """バッチ単位でPSVを読み、Rust側で一括抽出する（ADR-0065）。

    DataLoaderのworkerを使わない。`extract_batch` がGILを解放して
    rayonで並列に抽出するため、1プロセスで全コアを使える。
    抽出はprefetchスレッドが先回りし、GPU計算と重ねる。

    mmap時はチャンク単位でシャッフルする。ファイル上の連続領域を
    読んでからチャンク内を混ぜるので、読み出しがシーケンシャルに
    保たれる。全体の一様性は事前シャッフル済みのファイルで担保する。
    """

    def __init__(self, path, batch, lambda_=0.7, score_limit=0, mmap=False, score_clamp=0,
                 shuffle=True, chunk_positions=1 << 20, seed=0, prefetch=3):
        size = os.path.getsize(path)
        if size % 40 != 0:
            raise ValueError(f"ファイルサイズが40の倍数でない: {size}")
        self.n = size // 40
        self.batch = batch
        self.lambda_ = lambda_
        self.score_limit = score_limit
        self.score_clamp = score_clamp
        self.mmap = mmap
        self.shuffle = shuffle
        self.chunk = max(chunk_positions, batch)
        self.seed = seed
        self.prefetch = prefetch
        self.epoch = 0
        if mmap:
            self.data = np.memmap(path, dtype=np.uint8, mode="r", shape=(self.n, 40))
        else:
            self.data = np.fromfile(path, dtype=np.uint8).reshape(-1, 40)

    def __len__(self):
        return math.ceil(self.n / self.batch)

    def _raw_batches(self, rng):
        if not self.shuffle:
            for s in range(0, self.n, self.batch):
                yield self.data[s:s + self.batch]
        elif not self.mmap:
            perm = rng.permutation(self.n)
            for s in range(0, self.n, self.batch):
                yield self.data[perm[s:s + self.batch]]
        else:
            n_chunks = math.ceil(self.n / self.chunk)
            carry = None
            for c in rng.permutation(n_chunks):
                lo = c * self.chunk
                block = np.array(self.data[lo:min(lo + self.chunk, self.n)])
                rng.shuffle(block)
                if carry is not None:
                    block = np.concatenate([carry, block])
                    carry = None
                full = (len(block) // self.batch) * self.batch
                for s in range(0, full, self.batch):
                    yield block[s:s + self.batch]
                if full < len(block):
                    carry = block[full:]
            if carry is not None and len(carry) > 0:
                yield carry

    def _extract(self, raw):
        arrays = himawari.extract_batch(
            raw.tobytes(), self.lambda_, self.score_limit, self.score_clamp
        )
        if len(arrays[4]) == 0:
            return None
        return tuple(torch.from_numpy(a) for a in arrays)

    def __iter__(self):
        rng = np.random.default_rng(self.seed + self.epoch)
        self.epoch += 1
        q = queue.Queue(maxsize=self.prefetch)

        def produce():
            try:
                for raw in self._raw_batches(rng):
                    q.put(self._extract(raw))
            except Exception as e:  # 生産側の例外を消費側へ伝える
                q.put(e)
            q.put(None)

        t = threading.Thread(target=produce, daemon=True)
        t.start()
        while True:
            item = q.get()
            if item is None:
                break
            if isinstance(item, Exception):
                raise item
            yield item


def collate_psv(batch):
    """Collate variable-length feature lists into EmbeddingBag format."""
    batch = [b for b in batch if b is not None]
    if not batch:
        return None

    stm_all, stm_off = [], [0]
    opp_all, opp_off = [], [0]
    targets = []

    for stm, opp, t in batch:
        stm_all.extend(stm)
        stm_off.append(len(stm_all))
        opp_all.extend(opp)
        opp_off.append(len(opp_all))
        targets.append(t)

    return (
        torch.tensor(stm_all, dtype=torch.long),
        torch.tensor(stm_off[:-1], dtype=torch.long),
        torch.tensor(opp_all, dtype=torch.long),
        torch.tensor(opp_off[:-1], dtype=torch.long),
        torch.tensor(targets, dtype=torch.float32),
    )
