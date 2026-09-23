"""対局順の生データから、焦点のヒートマップつき局面集を切り出す（ADR-0213）。

焦点のラベルはルールを書かずに棋譜から取る。ある局面から先のk手（既定8、
両者4手ずつ）で駒が動いたマスと取られたマスを9×9のヒートマップにし、盤上の駒ごとに
「続くk手で動いたか、取られたか」を関与フラグにする。probeの学習器が読む。

## 出力の形式

`data/train/<名前>.focus` は202バイト固定長のレコードを並べたものである。
読み手は `np.fromfile(path, dtype=np.uint8).reshape(-1, 202)` で読める。

| 位置 | 大きさ | 中身 |
|---|---|---|
| 0-39 | 40 | 元のpsvのレコード |
| 40-120 | 81 | ヒートマップ。続くk手で動いたマスと取られたマスが1 |
| 121-201 | 81 | 関与フラグ。そのマスの駒がk手以内に動いたか取られたら1 |

psvのレコードは、先頭32バイトがpacked sfen、32-33がscore（i16）、
34-35が指し手（u16）、36-37がgamePly（u16）、38がgame_result（i8）、
39が詰めである。

マスの番号は `cshogi` に合わせ、`sq = (筋 - 1) * 9 + (段 - 1)` とする。
0が1一、80が9九になる。ヒートマップと関与フラグは、その局面の手番ではなく
盤の向きで並ぶ。

## 指し手の取り方

psvのmove（やねうら王のMove16）は復号せず、連続する2局面の盤面の差から取る。
差の出るマスは、移動なら移動元と移動先の2つ、打ちなら打った先の1つに限られる。
手番が入れ替わり、gamePlyが1増えることも確かめる。条件を満たさない境目は
対局の切れ目として扱い、窓はそこをまたがない。続きがk手に満たない局面は捨てる。
"""

from __future__ import annotations

import struct
import time
from collections import deque
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Iterable, Iterator, Sequence

PSV_BYTES = 40
SFEN_BYTES = 32
PLY_OFFSET = 36
SQUARES = 81
RECORD_BYTES = PSV_BYTES + 2 * SQUARES
SUFFIX = ".focus"

# 先を見る手数。両者4手ずつを既定にする（ADR-0213の測定の設計）
PLIES = 8

CHUNK = PSV_BYTES * 65536
REPORT_SECONDS = 60


@dataclass(frozen=True)
class Position:
    """生データの1レコードと、そこから復元した盤。"""

    record: bytes  # psvの40バイト
    ply: int
    board: bytes  # マスごとの駒。81個
    turn: int


@dataclass(frozen=True)
class Move:
    """盤面の差から取った1手。打ちは from_sq がNoneになる。"""

    from_sq: int | None
    to_sq: int
    captured: bool


@dataclass
class Stats:
    """切り出しの結果。完了マーカーにそのまま入れる。"""

    positions: int = 0  # 読んだ局面
    rows: int = 0  # 書いたレコード
    heat: int = 0  # ヒートマップの1の総数
    involved: int = 0  # 関与フラグの1の総数
    per_square: list[int] = field(default_factory=lambda: [0] * SQUARES)
    seconds: float = 0.0

    def summary(self) -> dict:
        """完了マーカーへ入れる要約。マスごとの頻度は率で残す。"""
        rows = max(self.rows, 1)
        return {
            "positions_read": self.positions,
            "rows": self.rows,
            "heat_mean": round(self.heat / rows, 3),
            "involved_mean": round(self.involved / rows, 3),
            "square_rate": [round(n / rows, 5) for n in self.per_square],
            "seconds": round(self.seconds),
        }


# --- 盤面の差から1手を取る --------------------------------------------


def board_move(before: bytes, after: bytes) -> Move | None:
    """2つの盤の差から1手を取る。差が手の形をしていなければNoneを返す。"""
    changed = [sq for sq in range(SQUARES) if before[sq] != after[sq]]
    if len(changed) == 1:
        (to_sq,) = changed
        # 打ちは、空きマスへ駒が現れる形になる
        if before[to_sq] != 0 or after[to_sq] == 0:
            return None
        return Move(None, to_sq, False)
    if len(changed) != 2:
        return None
    a, b = changed
    if after[a] == 0 and after[b] != 0:
        from_sq, to_sq = a, b
    elif after[b] == 0 and after[a] != 0:
        from_sq, to_sq = b, a
    else:
        return None
    return Move(from_sq, to_sq, before[to_sq] != 0)


def transition(before: Position, after: Position) -> Move | None:
    """連続する2局面から1手を取る。対局がつながっていなければNoneを返す。"""
    if after.ply != before.ply + 1 or after.turn == before.turn:
        return None
    return board_move(before.board, after.board)


# --- ラベル ------------------------------------------------------------


def labels(moves: Sequence[Move]) -> tuple[bytes, bytes]:
    """窓の先頭の局面から見たヒートマップと関与フラグを作る。

    関与は駒を追って決める。`origin` は「今このマスにいる駒が、先頭の局面で
    どのマスにいたか」を持ち、後から打たれた駒はNoneで印を付ける。追わずに
    移動元のマスだけを見ると、2手続けて動いた駒の関与を先頭の局面の別の駒へ
    付けてしまう。
    """
    heat = bytearray(SQUARES)
    involved = bytearray(SQUARES)
    origin: dict[int, int | None] = {}
    for mv in moves:
        if mv.from_sq is not None:
            heat[mv.from_sq] = 1
        heat[mv.to_sq] = 1
        if mv.captured:
            taken = origin.get(mv.to_sq, mv.to_sq)
            if taken is not None:
                involved[taken] = 1
        if mv.from_sq is None:
            origin[mv.to_sq] = None
            continue
        moved = origin.get(mv.from_sq, mv.from_sq)
        if moved is not None:
            involved[moved] = 1
        origin[mv.to_sq] = moved
        origin[mv.from_sq] = None
    return bytes(heat), bytes(involved)


def iter_focus(stream: Iterable[Position], plies: int = PLIES) -> Iterator[bytes]:
    """局面の列から、続きがk手ある局面のレコードを順に返す。"""
    window: deque[Position] = deque()
    moves: deque[Move] = deque()
    for pos in stream:
        if window:
            move = transition(window[-1], pos)
            if move is None:
                # 対局の切れ目。ここから新しい窓を始める
                window.clear()
                moves.clear()
            else:
                moves.append(move)
        window.append(pos)
        if len(moves) == plies:
            heat, involved = labels(moves)
            yield window[0].record + heat + involved
            window.popleft()
            moves.popleft()


# --- 生データの読み取り ------------------------------------------------


def positions(path: Path) -> Iterator[Position]:
    """生データを頭から読み、局面を復元する。

    `set_psfen` はnumpyの配列しか受けないので、読んだ塊をそのまま配列で
    見て、32バイトずつの窓を渡す。
    """
    import cshogi
    import numpy as np

    board = cshogi.Board()
    with open(path, "rb") as fh:
        while chunk := fh.read(CHUNK):
            buffer = np.frombuffer(chunk, dtype=np.uint8)
            for i in range(0, len(chunk) - PSV_BYTES + 1, PSV_BYTES):
                board.set_psfen(buffer[i : i + SFEN_BYTES])
                (ply,) = struct.unpack_from("<H", chunk, i + PLY_OFFSET)
                yield Position(
                    chunk[i : i + PSV_BYTES], ply, bytes(board.pieces), board.turn
                )


def _counted(stream: Iterable[Position], stats: Stats) -> Iterator[Position]:
    for pos in stream:
        stats.positions += 1
        yield pos


def write(
    sources: Sequence[Path],
    out: Path,
    *,
    count: int,
    plies: int = PLIES,
    report: Callable[[str], None] = print,
    source_positions: Callable[[Path], Iterable[Position]] | None = None,
) -> Stats:
    """名前順のファイルから、count件のレコードを書き出す。"""
    read = source_positions or positions
    stats = Stats()
    started = last = time.time()
    with open(out, "wb") as fh:
        for path in sources:
            if stats.rows >= count:
                break
            report(f"読む: {path.name}")
            for row in iter_focus(_counted(read(path), stats), plies):
                fh.write(row)
                stats.rows += 1
                heat = row[PSV_BYTES : PSV_BYTES + SQUARES]
                stats.heat += sum(heat)
                stats.involved += sum(row[PSV_BYTES + SQUARES :])
                sq = heat.find(1)
                while sq >= 0:
                    stats.per_square[sq] += 1
                    sq = heat.find(1, sq + 1)
                now = time.time()
                if now - last >= REPORT_SECONDS:
                    last = now
                    rate = round(stats.rows / max(now - started, 1e-9))
                    report(f"{stats.rows:,}/{count:,}件 {rate:,}件/秒")
                if stats.rows >= count:
                    break
    stats.seconds = time.time() - started
    return stats


# --- 統計の表示 --------------------------------------------------------


def square_grid(rate: Sequence[float]) -> list[str]:
    """マスごとの頻度を、盤の向きの9×9で並べる。"""
    files = "９８７６５４３２１"
    ranks = "一二三四五六七八九"
    lines = ["    " + " ".join(f"{f:>4}" for f in files)]
    for rank in range(9):
        cells = []
        for column in range(9):
            sq = (8 - column) * 9 + rank
            cells.append(f"{rate[sq] * 100:5.1f}")
        lines.append(f"{ranks[rank]}  " + " ".join(cells))
    return lines


def report_stats(stats: Stats, *, plies: int, report: Callable[[str], None] = print) -> None:
    """平均の個数と、マスごとの頻度を出す。頻度ベースラインはprobeの比較基準になる。"""
    rows = max(stats.rows, 1)
    rate = [n / rows for n in stats.per_square]
    report(f"局面: {stats.rows:,}件（読んだ生データ {stats.positions:,}件、先{plies}手）")
    report(f"ヒートマップ: 平均 {stats.heat / rows:.2f} マス")
    report(f"関与  : 平均 {stats.involved / rows:.2f} 駒")
    report("マスごとのヒートマップの頻度（%）")
    for line in square_grid(rate):
        report(line)
