"""点数を付けた局面の台帳（ADR-0228）。

dlshogiで点数を付ける局面を増やし続けるので、同じ局面に二度点数を付けないよう、
付けた局面の盤面ハッシュを台帳に持つ。付ける前に台帳と突き合わせ、初めて見る局面
だけを残す。検証集合の局面も台帳に入れ、学習へ混ざらないようにする。

盤面はpacked sfenの32バイト（盤・手番・持ち駒）で比べ、64ビットのハッシュにする。
200億局面で誤って同一視する組の期待値は約10組で、学習には影響しない。台帳は
ハッシュの先頭8ビットで256個のファイルに分け、それぞれソートして重複なく持つ。
1区分ずつ読めばよいので、台帳が大きくなってもメモリは1区分ぶんで済む。
"""

from __future__ import annotations

import json
import time
from pathlib import Path

import numpy as np

PSV_BYTES = 40
SHARDS = 256
CHUNK = 10_000_000
_M1 = np.uint64(0xBF58476D1CE4E5B9)
_M2 = np.uint64(0x94D049BB133111EB)


def _mix(x: np.ndarray) -> np.ndarray:
    """splitmix64 の終段。"""
    x = x ^ (x >> np.uint64(30))
    x = x * _M1
    x = x ^ (x >> np.uint64(27))
    x = x * _M2
    return x ^ (x >> np.uint64(31))


def board_hash(rec: np.ndarray) -> np.ndarray:
    """(n,40) の局面から、packed sfen 32バイトの64ビットハッシュを作る。"""
    w = np.ascontiguousarray(rec[:, :32]).view(np.uint64)
    with np.errstate(over="ignore"):
        h = _mix(w[:, 3])
        for k in (2, 1, 0):
            h = _mix(h ^ w[:, k])
    return h


def _shard(h: np.ndarray) -> np.ndarray:
    return (h >> np.uint64(56)).astype(np.int64)


def _memmap(path: Path) -> np.ndarray:
    n = path.stat().st_size // PSV_BYTES
    return np.memmap(path, dtype=np.uint8, mode="r", shape=(n, PSV_BYTES))


def _shard_path(root: Path, s: int) -> Path:
    return root / f"h{s:02x}.u64"


def load_shard(root: Path, s: int) -> np.ndarray:
    p = _shard_path(root, s)
    return np.fromfile(p, dtype=np.uint64) if p.is_file() else np.zeros(0, np.uint64)


def size(root: Path) -> int:
    return sum(p.stat().st_size for p in root.glob("h*.u64")) // 8


def add(root: Path, sources: list[Path], *, progress=None) -> dict:
    """局面のハッシュを台帳へ足す。何度足しても台帳は重複を持たない。"""
    root.mkdir(parents=True, exist_ok=True)
    tmp = root / "add.tmp"
    if tmp.exists():
        # 前回が途中で落ちた残り。足し終えていない一時ファイルは捨てて読み直す
        for f in tmp.iterdir():
            f.unlink()
    tmp.mkdir(exist_ok=True)
    seen = 0
    for src in sources:
        rec = _memmap(src)
        for lo in range(0, len(rec), CHUNK):
            h = board_hash(np.asarray(rec[lo:lo + CHUNK]))
            sh = _shard(h)
            order = np.argsort(sh, kind="stable")
            h, sh = h[order], sh[order]
            cuts = np.searchsorted(sh, np.arange(SHARDS + 1))
            # 256区分を同時に開くと、launchdの常駐ではファイル数の上限（256）を超える。
            # 区分ごとに追記で開いて閉じる
            for s in range(SHARDS):
                if cuts[s] < cuts[s + 1]:
                    with open(tmp / f"{s:02x}", "ab") as f:
                        f.write(h[cuts[s]:cuts[s + 1]].tobytes())
            seen += len(h)
            if progress:
                progress(seen)
    before = size(root)
    for s in range(SHARDS):
        piece = tmp / f"{s:02x}"
        new = np.fromfile(piece, dtype=np.uint64) if piece.is_file() else np.zeros(0, np.uint64)
        merged = np.unique(np.concatenate([load_shard(root, s), new]))
        out = _shard_path(root, s)
        part = out.with_name(out.name + ".part")
        merged.tofile(part)
        part.replace(out)
        piece.unlink(missing_ok=True)
    tmp.rmdir()
    after = size(root)
    log = root / "added.jsonl"
    with open(log, "a", encoding="utf-8") as fh:
        fh.write(json.dumps({"sources": [str(s) for s in sources], "read": seen, "new": after - before,
                             "size": after, "at": time.strftime("%Y-%m-%dT%H:%M:%S%z")},
                            ensure_ascii=False) + "\n")
    return {"read": seen, "new": after - before, "size": after}


def filter_new(root: Path, source: Path, part: Path, *, progress=None) -> dict:
    """台帳に無く、入力の中でも初出の局面だけを、入力の順に書き出す。"""
    rec = _memmap(source)
    n = len(rec)
    h = np.empty(n, np.uint64)
    for lo in range(0, n, CHUNK):
        h[lo:lo + CHUNK] = board_hash(np.asarray(rec[lo:lo + CHUNK]))
    uniq, first = np.unique(h, return_index=True)  # uniq は昇順、first は初出の位置
    del h
    keep = np.zeros(n, bool)
    sh = _shard(uniq)
    cuts = np.searchsorted(sh, np.arange(SHARDS + 1))
    known = 0
    for s in range(SHARDS):
        a, b = cuts[s], cuts[s + 1]
        if a == b:
            continue
        led = load_shard(root, s)
        u = uniq[a:b]
        if len(led):
            pos = np.searchsorted(led, u)
            pos[pos >= len(led)] = 0
            hit = led[pos] == u
        else:
            hit = np.zeros(len(u), bool)
        known += int(hit.sum())
        keep[first[a:b][~hit]] = True
    written = 0
    with open(part, "wb") as out:
        for lo in range(0, n, CHUNK):
            m = keep[lo:lo + CHUNK]
            rows = np.asarray(rec[lo:lo + CHUNK])[m]
            out.write(rows.tobytes())
            written += len(rows)
            if progress:
                progress(min(lo + CHUNK, n))
    return {"read": n, "kept": written, "dup_in_input": n - len(uniq), "known": known}
