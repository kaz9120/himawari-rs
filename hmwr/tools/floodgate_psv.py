"""floodgateの全対局の棋譜から、学習用の局面を取り出す（ADR-0228）。

wdoorは年ごとの棋譜を7zで配っている（GitHubの shogi-server のリリースにミラーがある）。
年ごとに取得・展開し、各対局を初期局面から再生して、指す前の局面を1手ずつ psv へ書く。

- 評価値は0にする。点数はあとでdlshogiで付け直す（ADR-0228の型）
- 勝敗は手番側から見た値（勝ち1・負け−1・引き分けと不明0）
- 指し手はやねうら王の Move16 にする。cshogi の move16 とは、成りと打つ手の表し方が違う
  （to_yo_move16 の説明を参照）
- 手数の短い対局（接続切れなど）は飛ばす

展開した棋譜は年ごとに消すので、ディスクに残るのはアーカイブと出力だけである。
"""

from __future__ import annotations

import shutil
import subprocess
import urllib.request
from multiprocessing import Pool
from pathlib import Path

import numpy as np

PSV_BYTES = 40
URL = "https://github.com/shogi-server/shogi-server/releases/download/{tag}/wdoor{year}.7z"


def to_yo_move16(m16: int) -> int:
    """cshogi の move16 を、やねうら王の Move16 へ直す。

    cshogi は成りを14ビット目、打つ手を移動元の欄の 81＋駒種−1 で表す。やねうら王は
    成りを15ビット目、打つ手を14ビット目の印と移動元の欄の駒種で表す。駒種の番号は同じ。
    """
    to = m16 & 0x7F
    fr = (m16 >> 7) & 0x7F
    if fr >= 81:
        return to | ((fr - 80) << 7) | (1 << 14)
    return to | (fr << 7) | (((m16 >> 14) & 1) << 15)


def from_yo_move16(y: int) -> int:
    """やねうら王の Move16 を cshogi の move16 へ戻す（検証用）。"""
    to = y & 0x7F
    fr = (y >> 7) & 0x7F
    if y & (1 << 14):
        return to | ((80 + fr) << 7)
    return to | (fr << 7) | (((y >> 15) & 1) << 14)


def game_records(path: str, min_plies: int) -> bytes:
    """1局を再生し、指す前の局面を psv の行の並びにする。読めない棋譜は空を返す。"""
    import cshogi
    from cshogi import CSA

    try:
        kif = CSA.Parser.parse_file(path)[0]
    except Exception:
        return b""
    moves = kif.moves
    if len(moves) < min_plies:
        return b""
    board = cshogi.Board()
    try:
        board.set_sfen(kif.sfen)
    except Exception:
        return b""
    # kif.win: 1=先手勝ち、2=後手勝ち、0=引き分けか不明
    winner = {1: 0, 2: 1}.get(kif.win)
    out = np.zeros((len(moves), PSV_BYTES), np.uint8)
    start_ply = board.move_number
    n = 0
    for i, mv in enumerate(moves):
        if not board.is_legal(mv):
            break
        row = out[n]
        board.to_psfen(row[:32])
        # score は0のまま（32〜33バイト）
        m16 = to_yo_move16(int(cshogi.move16(mv)))
        row[34] = m16 & 0xFF
        row[35] = m16 >> 8
        ply = start_ply + i
        row[36] = ply & 0xFF
        row[37] = (ply >> 8) & 0xFF
        if winner is not None:
            row[38] = np.uint8(1 if board.turn == winner else 255)  # −1は255
        n += 1
        board.push(mv)
    return out[:n].tobytes()


def _work(args):
    path, min_plies = args
    return game_records(path, min_plies)


def fetch(year: int, dest: Path, tag: str) -> Path:
    """年のアーカイブを取得する。取得済みならそのまま使う（追記専用）。"""
    dest.mkdir(parents=True, exist_ok=True)
    path = dest / f"wdoor{year}.7z"
    if path.is_file() and path.stat().st_size > 0:
        return path
    part = path.with_name(path.name + ".part")
    with urllib.request.urlopen(URL.format(tag=tag, year=year)) as r, open(part, "wb") as f:
        shutil.copyfileobj(r, f, length=1 << 20)
    part.replace(path)
    return path


def convert(years: list[int], archive_dir: Path, part: Path, *, tag: str, min_plies: int, jobs: int,
            limit: int | None = None, report=print, progress=None) -> dict:
    total = games = kept_games = 0
    with open(part, "wb") as out:
        for year in years:
            arc = fetch(year, archive_dir, tag)
            work = archive_dir / f"x{year}"
            if work.exists():
                shutil.rmtree(work)
            work.mkdir()
            subprocess.run(["7zz", "x", "-y", "-bso0", "-bsp0", f"-o{work}", str(arc)], check=True)
            files = sorted(str(p) for p in work.rglob("*.csa"))
            if limit:
                files = files[:limit]
            year_rows = 0
            with Pool(jobs) as pool:
                for data in pool.imap(_work, [(f, min_plies) for f in files], chunksize=64):
                    if data:
                        out.write(data)
                        year_rows += len(data) // PSV_BYTES
                        kept_games += 1
            shutil.rmtree(work)
            games += len(files)
            total += year_rows
            report(f"{year}年: {len(files):,}局から {year_rows:,}局面")
            if progress:
                progress(total)
    return {"read": total, "kept": total, "games": games, "games_used": kept_games}
