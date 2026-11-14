"""PSV dataset reader via Rust PyO3 bridge."""

import math
import os
import queue
import threading

import numpy as np
import torch
from torch.utils.data import Dataset

import himawari

#: psvの1レコード
PSV_BYTES = 40
#: `extract_batch` が返す配列の本数。末尾2本が利きの入力特徴
BATCH_ARRAYS = 9


def to_tensors(arrays):
    """抽出結果をテンソルの組にし、9本へ揃える。

    足りない分は長さ0で埋め、受け取り側の分解を1通りに保つ。
    """
    tensors = [torch.from_numpy(a) for a in arrays]
    while len(tensors) < BATCH_ARRAYS:
        tensors.append(torch.zeros(0, dtype=torch.int64))
    return tuple(tensors)


def extract(records, *args, effect_inputs=False):
    """`extract_batch` を呼ぶ。利きの入力特徴は要るときだけ頼む。

    古い拡張は `effect_inputs` を受けないので、要らないときは渡さない。
    """
    if effect_inputs:
        return himawari.extract_batch(records, *args, effect_inputs=True)
    return himawari.extract_batch(records, *args)


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
    """バッチ単位でPSVを読み、Rust側で一括抽出する。

    DataLoaderのworkerを使わない。`extract_batch` がGILを解放して
    rayonで並列に抽出するため、1プロセスで全コアを使える。
    抽出はprefetchスレッドが先回りし、GPU計算と重ねる。

    mmap時はチャンク単位でシャッフルする。ファイル上の連続領域を
    読んでからチャンク内を混ぜるので、読み出しがシーケンシャルに
    保たれる。全体の一様性は事前シャッフル済みのファイルで担保する。
    """

    def __init__(self, path, batch, lambda_=0.7, score_limit=0, mmap=False, score_clamp=0,
                 shuffle=True, chunk_positions=1 << 20, seed=0, prefetch=3,
                 positions=None, effect_inputs=False):
        size = os.path.getsize(path)
        if size % 40 != 0:
            raise ValueError(f"ファイルサイズが40の倍数でない: {size}")
        self.n = size // 40
        # 先頭のこの件数だけを使う。大きなpsvの一部を、コピーを
        # 作らずに学習するためのもので、バッチの並びは件数だけで決まる
        if positions is not None:
            if positions <= 0 or positions > self.n:
                raise ValueError(f"positionsが範囲外: {positions}（ファイルは{self.n}件）")
            self.n = positions
        self.batch = batch
        self.lambda_ = lambda_
        self.score_limit = score_limit
        self.score_clamp = score_clamp
        self.mmap = mmap
        # 利きの入力特徴。処置群だけが抽出する
        self.effect_inputs = effect_inputs
        self.shuffle = shuffle
        self.chunk = max(chunk_positions, batch)
        self.seed = seed
        self.prefetch = prefetch
        self.epoch = 0
        # 再開時にエポック内で読み飛ばすバッチ数。1回の
        # __iter__ でだけ効く。バッチの並びは seed と epoch で決まるので、
        # 同じ数だけ飛ばせば中断した位置から続けられる
        self.skip_batches = 0
        if mmap:
            self.data = np.memmap(path, dtype=np.uint8, mode="r", shape=(self.n, 40))
        else:
            self.data = np.fromfile(path, dtype=np.uint8, count=self.n * 40).reshape(-1, 40)

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
        arrays = extract(
            raw.tobytes(), self.lambda_, self.score_limit, self.score_clamp,
            effect_inputs=self.effect_inputs,
        )
        if len(arrays[4]) == 0:
            return None
        return to_tensors(arrays)

    def __iter__(self):
        rng = np.random.default_rng(self.seed + self.epoch)
        self.epoch += 1
        skip = self.skip_batches
        self.skip_batches = 0
        q = queue.Queue(maxsize=self.prefetch)

        def produce():
            try:
                for i, raw in enumerate(self._raw_batches(rng)):
                    # 読み飛ばす区間も生成器は回す。チャンクのシャッフルが
                    # 乱数の状態を進めるので、飛ばすと以降の並びがずれる。
                    # 特徴抽出だけを省くので、ここは読むだけで済む
                    if i < skip:
                        continue
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


class RankLoader:
    """兄弟局面のランキング群を供給する。

    ファイルは `psv rank` が書いた40バイト×3（正例・負例・負例）の群の
    連なり。ステップごとに群を一様に引き、順序を保ったまま特徴を抽出して
    返す。抽出はstrictで行い、1レコードでも落ちたら例外にする。黙って
    落ちると群の整列が壊れ、別の局面と比較してしまうためである。

    読み出しと抽出は先読みスレッドが先回りし、GPU計算と重ねる。学習
    ループ内の同期呼び出しのままだと、memmapへのランダム読みがページ
    キャッシュを外れたとき1ステップ300ms規模の待ちになる（issue #409）。
    乱数を消費するのは先読みスレッドだけなので、seedで決まるバッチの列は
    先読みの有無で変わらない。

    予備バイト（b[39]）は親からの手数の偶奇で、呼び出し側はこれで
    評価値を親視点の符号へ戻す。
    """

    def __init__(self, path, groups_per_step, seed=0, prefetch=2, effect_inputs=False):
        size = os.path.getsize(path)
        if size % 120 != 0:
            raise ValueError(f"ファイルサイズが120の倍数でない: {size}")
        self.n = size // 120
        self.batch = groups_per_step
        self.rng = np.random.default_rng(seed)
        self.data = np.memmap(path, dtype=np.uint8, mode="r", shape=(self.n, 120))
        self.prefetch = prefetch
        # 利きの入力特徴。処置群は兄弟局面にも同じ特徴を立てる
        self.effect_inputs = effect_inputs
        self._queue = None

    def _sample_now(self):
        idx = np.sort(self.rng.integers(0, self.n, size=self.batch))
        recs = np.array(self.data[idx]).reshape(-1, 40)
        parity = torch.from_numpy(recs[:, 39].astype(np.float32).copy())
        arrays = to_tensors(extract(
            recs.tobytes(), 0.0, 0, 0, True, effect_inputs=self.effect_inputs,
        ))
        stm_i, stm_o, opp_i, opp_o = arrays[:4]
        return stm_i, stm_o, opp_i, opp_o, parity, arrays[7], arrays[8], arrays[9]

    def _produce(self):
        while True:
            try:
                self._queue.put(self._sample_now())
            except Exception as e:  # 生産側の例外を消費側へ伝える
                self._queue.put(e)
                return

    def sample(self):
        if self._queue is None:
            self._queue = queue.Queue(maxsize=self.prefetch)
            threading.Thread(target=self._produce, daemon=True).start()
        item = self._queue.get()
        if isinstance(item, Exception):
            raise item
        return item
