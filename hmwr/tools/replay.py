"""CSAの棋譜を、実戦の持ち時間のまま手元のエンジンで指し直す（診断）。

単発の局面では出ないが対局の流れの中で出る問題を再現する。置換表・
時間管理の予約・ponderの状態は前の手から持ち越されるので、局面だけを
渡す検討では見えない。2026-09-18のfloodgateの切れ負けは、この方法で
再現できた（Issue #471、ADR-0211）。

自分の手番ごとに、その時点の残り時間で `go` を送り、返ってくるまでの
時間と最終の反復を記録する。相手の手は実戦のとおりに流す。
"""

from __future__ import annotations

import re
import subprocess
import sys
import time
from pathlib import Path

PROMOTED = {"TO": "FU", "NY": "KY", "NK": "KE", "NG": "GI", "UM": "KA", "RY": "HI"}
DROP = {"FU": "P", "KY": "L", "KE": "N", "GI": "S", "KI": "G", "KA": "B", "HI": "R"}
INITIAL = [
    "-KY-KE-GI-KI-OU-KI-GI-KE-KY",
    " * -HI *  *  *  *  * -KA * ",
    "-FU-FU-FU-FU-FU-FU-FU-FU-FU",
    " *  *  *  *  *  *  *  *  * ",
    " *  *  *  *  *  *  *  *  * ",
    " *  *  *  *  *  *  *  *  * ",
    "+FU+FU+FU+FU+FU+FU+FU+FU+FU",
    " * +KA *  *  *  *  * +HI * ",
    "+KY+KE+GI+KI+OU+KI+GI+KE+KY",
]
MOVE_RE = re.compile(r"^([+-])(\d)(\d)(\d)(\d)([A-Z]{2})")


def csa_to_usi(text: str) -> tuple[list[str], list[int], str]:
    """CSAの指し手をUSIへ直す。(指し手, 消費秒, 後手の名前) を返す。

    成りの判定には盤上の駒を追う。CSAの指し手は移動後の駒種しか持たない
    ためである。平手の初期局面から始まる棋譜だけを扱う。
    """
    board: dict[tuple[int, int], str] = {}
    for rank, line in enumerate(INITIAL, 1):
        for i in range(9):
            cell = line[i * 3 : i * 3 + 3]
            if cell.strip() != "*":
                board[(9 - i, rank)] = cell[1:]
    moves, seconds, white = [], [], ""
    pending = None
    for line in text.replace("\r", "").splitlines():
        if line.startswith("N-"):
            white = line[2:]
        m = MOVE_RE.match(line)
        if m:
            fx, fy, tx, ty, piece = (int(m[2]), int(m[3]), int(m[4]), int(m[5]), m[6])
            to = f"{tx}{chr(ord('a') + ty - 1)}"
            if fx == 0:
                moves.append(f"{DROP[piece]}*{to}")
            else:
                src = board.pop((fx, fy), piece)
                promoted = piece in PROMOTED and src == PROMOTED[piece]
                moves.append(f"{fx}{chr(ord('a') + fy - 1)}{to}{'+' if promoted else ''}")
            board[(tx, ty)] = piece
            pending = len(moves)
        elif line.startswith("T") and line[1:].isdigit() and pending:
            seconds.append(int(line[1:]))
            pending = None
    return moves, seconds, white


def replay(
    engine: str,
    eval_file: str,
    csa: Path,
    *,
    player: str,
    threads: int,
    start_ply: int,
    hash_mb: int,
    initial_ms: int,
    inc_ms: int,
) -> int:
    moves, seconds, white = csa_to_usi(csa.read_text(encoding="utf-8", errors="replace"))
    if len(seconds) < len(moves):
        seconds += [0] * (len(moves) - len(seconds))
    ours = 1 if player and player in white else 0  # 0=先手、1=後手
    proc = subprocess.Popen(
        [engine], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1
    )
    assert proc.stdin and proc.stdout

    def send(s: str) -> None:
        proc.stdin.write(s + "\n")
        proc.stdin.flush()

    def until(prefix: str) -> list[str]:
        got = []
        while True:
            line = proc.stdout.readline()
            if not line:
                return got
            got.append(line.rstrip())
            if line.startswith(prefix):
                return got

    send("usi")
    until("usiok")
    send(f"setoption name EvalFile value {eval_file}")
    send(f"setoption name Threads value {threads}")
    send(f"setoption name USI_Hash value {hash_mb}")
    send("isready")
    until("readyok")

    clock = [initial_ms, initial_ms]
    print(f"{'手数':>4} {'残り':>6} {'実戦':>5} {'経過':>6}  最終の反復 / 実戦の手")
    for i in range(len(moves) + 1):
        # i手目までを流した局面で、次の手番が自分なら探索させる
        to_move = i % 2
        if to_move == ours and i + 1 >= start_ply:
            send("position startpos moves " + " ".join(moves[:i]))
            started = time.time()
            send(f"go btime {clock[0]} wtime {clock[1]} binc {inc_ms} winc {inc_ms}")
            lines = until("bestmove")
            elapsed = time.time() - started
            infos = [x for x in lines if x.startswith("info depth") and "score" in x]
            last = infos[-1] if infos else ""
            m = re.search(r"depth (\d+) .*score (\S+ \S+) nodes (\d+)", last)
            summary = f"深さ{m[1]} {m[2]} {int(m[3]):,}ノード" if m else "（反復なし）"
            actual = moves[i] if i < len(moves) else "（実戦はここで終局）"
            spent = seconds[i] if i < len(seconds) else 0
            print(
                f"{i + 1:>4} {clock[ours] / 1000:>5.1f}s {spent:>4}s {elapsed:>5.2f}s  "
                f"{lines[-1]} | {summary} | {actual}"
            )
        if i < len(moves):
            clock[i % 2] = clock[i % 2] - seconds[i] * 1000 + inc_ms
    send("quit")
    proc.wait(timeout=10)
    return 0


def main(argv: list[str] | None = None) -> int:
    import argparse

    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("engine")
    p.add_argument("csa", type=Path)
    p.add_argument("--eval-file", required=True)
    p.add_argument("--player", default="Himawari")
    p.add_argument("--threads", type=int, default=4)
    p.add_argument("--from", dest="start_ply", type=int, default=1)
    p.add_argument("--hash", type=int, default=256)
    p.add_argument("--initial", type=int, default=300, help="持ち時間[秒]")
    p.add_argument("--inc", type=int, default=10, help="1手ごとの加算[秒]")
    a = p.parse_args(argv)
    return replay(
        a.engine, a.eval_file, a.csa,
        player=a.player, threads=a.threads, start_ply=a.start_ply, hash_mb=a.hash,
        initial_ms=a.initial * 1000, inc_ms=a.inc * 1000,
    )  # fmt: skip


if __name__ == "__main__":
    sys.exit(main())
