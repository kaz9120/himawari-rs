"""PSV dataset reader via Rust PyO3 bridge (ADR-0043, ADR-0045, ADR-0065)."""

import math
import os
import queue
import threading

import numpy as np
import torch
from torch.utils.data import Dataset

import himawari

#: psvの1レコード（ADR-0038）
PSV_BYTES = 40
#: .focus の1レコード（ADR-0213）。psv40＋熱地図81＋関与81
FOCUS_BYTES = 202
#: 盤の升数。熱地図と関与フラグの長さになる
SQUARES = 81
#: 熱地図の向き。盤の向きのまま使うか、手番側から見た向きへ揃えるか
FOCUS_ORIENTS = ("board", "stm")


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
                 shuffle=True, chunk_positions=1 << 20, seed=0, prefetch=3, effect=False,
                 positions=None):
        size = os.path.getsize(path)
        if size % 40 != 0:
            raise ValueError(f"ファイルサイズが40の倍数でない: {size}")
        self.n = size // 40
        # 先頭のこの件数だけを使う（ADR-0219）。大きなpsvの一部を、コピーを
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
        # 利きラベル（ADR-0133）。使わないときは抽出させない。長さ0の
        # 配列が返るので、受け取り側の分解は9本のままでよい
        self.effect = effect
        self.shuffle = shuffle
        self.chunk = max(chunk_positions, batch)
        self.seed = seed
        self.prefetch = prefetch
        self.epoch = 0
        # 再開時にエポック内で読み飛ばすバッチ数（ADR-0159）。1回の
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
        arrays = himawari.extract_batch(
            raw.tobytes(), self.lambda_, self.score_limit, self.score_clamp,
            self.effect,
        )
        if len(arrays[4]) == 0:
            return None
        return tuple(torch.from_numpy(a) for a in arrays)

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


class FocusBatchLoader:
    """.focus を読み、psvの特徴と焦点の熱地図を返す（ADR-0213）。

    レコードは202バイト固定長で、先頭40バイトがpsv、続く81バイトが熱地図、
    残りの81バイトが駒ごとの関与フラグである。psvの部分は `PsvBatchLoader` と
    同じRustの抽出へ流し、熱地図を10本目のテンソルとして足す。バッチの形が
    9本から10本に増えるだけなので、学習ループの受け取り方は変わらない。

    抽出はstrictで行う。**黙って落ちるとラベルとの整列が壊れ、別の局面の
    熱地図を当てることになる。** 落ちたら即座に失敗させる。

    `lo` と `hi` でレコードの区間を切る。学習と検証の分割はこの区間で行い、
    同じファイルの先頭を学習、末尾を検証に回す。

    `orient` は熱地図の向きを選ぶ。`board` は書かれたまま、`stm` は後手番の
    局面で180度回す。**蓄積器は手番側から見た向きで並ぶ**ので、盤の向きの
    ままだと的と表現の向きが局面の半分でずれる。
    """

    def __init__(self, path, batch, *, lo=0, hi=None, lambda_=0.7,
                 shuffle=True, seed=0, prefetch=3, orient="board"):
        size = os.path.getsize(path)
        if size % FOCUS_BYTES != 0:
            raise ValueError(f"ファイルサイズが{FOCUS_BYTES}の倍数でない: {size}")
        total = size // FOCUS_BYTES
        hi = total if hi is None else min(hi, total)
        if not 0 <= lo < hi:
            raise ValueError(f"レコードの区間が空だ: [{lo}, {hi}) / 全{total}件")
        self.data = np.memmap(
            path, dtype=np.uint8, mode="r", shape=(total, FOCUS_BYTES),
        )[lo:hi]
        if orient not in FOCUS_ORIENTS:
            raise ValueError(f"熱地図の向きが不明: {orient}")
        self.n = hi - lo
        self.batch = batch
        self.orient = orient
        self.lambda_ = lambda_
        self.shuffle = shuffle
        self.seed = seed
        self.prefetch = prefetch
        self.epoch = 0
        # エポック内で読み飛ばすバッチ数（ADR-0159）。他のローダと揃える
        self.skip_batches = 0

    def __len__(self):
        return math.ceil(self.n / self.batch)

    def _heat(self, raw):
        """レコードの束から熱地図を取り出し、指定の向きへ揃える。

        後手番の180度回転は升の並びを逆にするだけでよい。升は
        `(筋 - 1) * 9 + (段 - 1)` なので、回した先は `80 - 升` になる。
        手番はpacked sfenの先頭ビットにある。
        """
        heat = np.array(raw[:, PSV_BYTES:PSV_BYTES + SQUARES])
        if self.orient == "stm":
            white = (raw[:, 0] & 1).astype(bool)
            heat[white] = heat[white][:, ::-1]
        return heat

    def heat_mean(self):
        """区間の熱地図の平均。升ごとの頻度事前で、probeの自明解になる。"""
        return self._heat(self.data).mean(axis=0, dtype=np.float64)

    def _extract(self, raw):
        arrays = himawari.extract_batch(
            raw[:, :PSV_BYTES].tobytes(), self.lambda_, 0, 0, False, True,
        )
        heat = torch.from_numpy(self._heat(raw).astype(np.float32))
        return (*(torch.from_numpy(a) for a in arrays), heat)

    def __iter__(self):
        rng = np.random.default_rng(self.seed + self.epoch)
        self.epoch += 1
        order = rng.permutation(self.n) if self.shuffle else np.arange(self.n)
        skip = self.skip_batches
        self.skip_batches = 0
        q = queue.Queue(maxsize=self.prefetch)

        def produce():
            try:
                for i, s in enumerate(range(0, self.n, self.batch)):
                    if i < skip:
                        continue
                    # バッチの中は昇順に読む。集合は変わらないので学習には
                    # 影響せず、memmapの読み出しだけが素直になる
                    idx = np.sort(order[s:s + self.batch])
                    q.put(self._extract(np.asarray(self.data[idx])))
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


class GeneratedBatchLoader:
    """局面をその場で作り、利きラベルを付けて返す（ADR-0133）。

    利き予測は決定的な構造タスクで、的は局面の分布に依存しない。だから
    教師データファイルが要らない。**生成した局面を使い捨てにすれば同じ
    局面は二度と出ず、訓練損失がそのまま未見データの損失になる。**
    過学習が構造的に起こらないので、別の検証集合を持つ意味がない。

    バッチの形は `PsvBatchLoader` と同じ9本で、`.n` に1エポックの局面数を
    持つ。評価値の的はないので targets は0.5、指し手ラベルは-1で埋まる。
    `--lambda-value 0` で使う前提である。

    生成はRust側がGILを解放して並列に行う。prefetchスレッドで先回りさせ、
    GPU計算と重ねるのは `PsvBatchLoader` と同じ。
    """

    #: SplitMix64と同じ攪拌定数。種を通し番号から散らすのに使う
    _GAMMA = 0x9E3779B97F4A7C15
    _MASK = (1 << 64) - 1

    def __init__(self, n, batch, seed=0, max_plies=256, prefetch=3):
        self.n = n
        self.batch = batch
        self.seed = seed
        self.max_plies = max_plies
        self.prefetch = prefetch
        self.epoch = 0
        # 再開時にエポック内で読み飛ばすバッチ数（ADR-0159）。1回の
        # __iter__ でだけ効く。バッチの並びは seed と epoch で決まるので、
        # 同じ数だけ飛ばせば中断した位置から続けられる
        self.skip_batches = 0
        # バッチの通し番号。エポックをまたいで進めるので、同じ乱数列は
        # 二度使わない。序盤の数手だけは playout の作りから必ず重なる
        self.cursor = 0

    def __len__(self):
        return math.ceil(self.n / self.batch)

    def _next_seed(self):
        z = (self.seed + (self.cursor + 1) * self._GAMMA) & self._MASK
        self.cursor += 1
        return z

    def __iter__(self):
        self.epoch += 1
        q = queue.Queue(maxsize=self.prefetch)
        sizes = [min(self.batch, self.n - s) for s in range(0, self.n, self.batch)]
        seeds = [self._next_seed() for _ in sizes]

        def produce():
            try:
                for size, seed in zip(sizes, seeds):
                    arrays = himawari.generate_batch(size, seed, self.max_plies)
                    q.put(tuple(torch.from_numpy(a) for a in arrays))
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
    """兄弟局面のランキング群を供給する（ADR-0185）。

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

    def __init__(self, path, groups_per_step, seed=0, prefetch=2):
        size = os.path.getsize(path)
        if size % 120 != 0:
            raise ValueError(f"ファイルサイズが120の倍数でない: {size}")
        self.n = size // 120
        self.batch = groups_per_step
        self.rng = np.random.default_rng(seed)
        self.data = np.memmap(path, dtype=np.uint8, mode="r", shape=(self.n, 120))
        self.prefetch = prefetch
        self._queue = None

    def _sample_now(self):
        idx = np.sort(self.rng.integers(0, self.n, size=self.batch))
        recs = np.array(self.data[idx]).reshape(-1, 40)
        parity = torch.from_numpy(recs[:, 39].astype(np.float32).copy())
        arrays = himawari.extract_batch(recs.tobytes(), 0.0, 0, 0, False, True)
        stm_i, stm_o, opp_i, opp_o = (torch.from_numpy(a) for a in arrays[:4])
        return stm_i, stm_o, opp_i, opp_o, parity

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
