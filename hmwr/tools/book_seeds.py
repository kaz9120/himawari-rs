"""floodgateの棋譜から、局面ごとの出現率を数えて定跡の種にする。

`book mine` の到達確率の見積もりに使う。両対局者のレートが下限以上の対局だけを数え、
1局の中で同じ局面は1回と数える。出現率は数えた対局数で割った値である。

出力は1行1局面で `<出現率> sfen <盤> <手番> <手駒> <手数>` を出現率の降順に並べる。
手数はその局面が最初に現れた手数である。
"""

from __future__ import annotations

import re
import shutil
import subprocess
from collections import Counter
from multiprocessing import Pool
from pathlib import Path

from . import floodgate_psv

RATE_RE = re.compile(r"^'(?:black|white)_rate:[^:\n]+:([0-9.]+)", re.M)
HIRATE_P1 = "P1-KY-KE-GI-KI-OU-KI-GI-KE-KY"


def game_keys(args: tuple[str, int, float, int]) -> list[tuple[str, int]] | None:
    """1局を再生し、手数max_plyまでの局面のキーと最初の手数を返す。数えない対局はNone。"""
    import cshogi
    from cshogi import CSA

    path, max_ply, min_rate, min_moves = args
    text = Path(path).read_text(errors="replace")
    rates = [float(v) for v in RATE_RE.findall(text)]
    if len(rates) < 2 or min(rates) < min_rate:
        return None
    if HIRATE_P1 not in text or "\nPI" in text:
        return None
    try:
        kif = CSA.Parser.parse_str(text)[0]
    except Exception:
        return None
    if len(kif.moves) < min_moves:
        return None
    board = cshogi.Board()
    out = []
    seen = set()
    for ply in range(min(max_ply, len(kif.moves)) + 1):
        key = " ".join(board.sfen().split()[:3])
        if key not in seen:
            seen.add(key)
            out.append((key, ply))
        if ply < len(kif.moves):
            mv = kif.moves[ply]
            if not board.is_legal(mv):
                break
            board.push(mv)
    return out


def count(years: list[int], archive_dir: Path, out: Path, *, tag: str, max_ply: int,
          min_rate: float, min_moves: int, min_count: int, jobs: int, report=print) -> dict:
    counts: Counter[str] = Counter()
    first_ply: dict[str, int] = {}
    games = 0
    for year in years:
        arc = floodgate_psv.fetch(year, archive_dir, tag)
        work = archive_dir / f"seeds{year}"
        if work.exists():
            shutil.rmtree(work)
        work.mkdir()
        try:
            subprocess.run(["7zz", "x", "-y", "-bso0", "-bsp0", f"-o{work}", str(arc)], check=True)
            files = sorted(str(p) for p in work.rglob("*.csa"))
            used = 0
            with Pool(jobs) as pool:
                tasks = [(f, max_ply, min_rate, min_moves) for f in files]
                for keys in pool.imap(game_keys, tasks, chunksize=64):
                    if keys is None:
                        continue
                    used += 1
                    for key, ply in keys:
                        counts[key] += 1
                        if ply < first_ply.get(key, 1 << 30):
                            first_ply[key] = ply
        finally:
            shutil.rmtree(work, ignore_errors=True)
        games += used
        report(f"{year}年: {len(files):,}局のうち{used:,}局を数えた")
    if games == 0:
        raise ValueError("条件を満たす対局が無い")
    rows = sorted(
        ((c / games, k) for k, c in counts.items() if c >= min_count),
        key=lambda r: (-r[0], r[1]),
    )
    part = out.with_name(out.name + ".part")
    with open(part, "w", encoding="utf-8") as fh:
        for p, k in rows:
            fh.write(f"{p:.6g} sfen {k} {first_ply[k] + 1}\n")
    part.replace(out)
    return {"games": games, "seeds": len(rows)}
