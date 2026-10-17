"""局面ごとに、固定深さの探索が持ち時間内に終わるかを測る（診断、ADR-0233）。

Issue #653では、大差の局面で深さ16の探索が20秒で終わらなかった。`hmwr bench` は
局面ごとの到達深さを出さず、打ち切りで止まるので、この測定には使えない。

局面ごとに `go depth N` を送り、打ち切りの秒数を過ぎたら `stop` を送る。
`bestmove` までの所要時間と、確定した最後の反復の深さ・評価値・ノード数を出す。
置換表は局面ごとに `usinewgame` で空にする。
"""

from __future__ import annotations

import queue
import re
import subprocess
import threading
import time
from dataclasses import dataclass
from pathlib import Path

INFO_RE = re.compile(r"^info depth (\d+) .*?score (cp|mate) (\S+).*? nodes (\d+)")


@dataclass
class Reach:
    index: int  # 局面リストでの番号（1始まり、コメント行を数えない）
    seconds: float
    finished: bool  # 打ち切る前に bestmove が返ったか
    depth: int  # 確定した最後の反復の深さ
    score: str
    nodes: int


def read_positions(path: Path) -> list[str]:
    """局面リストを読む。`#` で始まる行と空行を飛ばす。"""
    out = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        out.append(line if line.startswith(("sfen ", "startpos")) else f"sfen {line}")
    return out


class Engine:
    """USIエンジンを1本動かす。標準出力は別スレッドで読み、待ちに期限を付ける。"""

    def __init__(self, path: str, options: dict[str, str]) -> None:
        self.proc = subprocess.Popen(
            [path], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1
        )
        self.lines: queue.Queue[str | None] = queue.Queue()
        threading.Thread(target=self._pump, daemon=True).start()
        self.send("usi")
        self.until("usiok", None)
        for name, value in options.items():
            self.send(f"setoption name {name} value {value}")
        self.send("isready")
        self.until("readyok", None)

    def _pump(self) -> None:
        assert self.proc.stdout
        for line in self.proc.stdout:
            self.lines.put(line.rstrip())
        self.lines.put(None)

    def send(self, s: str) -> None:
        assert self.proc.stdin
        self.proc.stdin.write(s + "\n")
        self.proc.stdin.flush()

    def until(self, prefix: str, deadline: float | None) -> tuple[list[str], bool]:
        """prefixで始まる行まで読む。期限を過ぎたら (読めた分, False) を返す。"""
        got: list[str] = []
        while True:
            timeout = None if deadline is None else max(deadline - time.monotonic(), 0)
            try:
                line = self.lines.get(timeout=timeout)
            except queue.Empty:
                return got, False
            if line is None:
                raise RuntimeError("エンジンが終了した")
            got.append(line)
            if line.startswith(prefix):
                return got, True

    def close(self) -> None:
        self.send("quit")
        self.proc.wait(timeout=10)


def measure(
    engine: Engine, index: int, position: str, depth: int, limit: float
) -> Reach:
    engine.send("usinewgame")
    engine.send("isready")
    engine.until("readyok", None)
    engine.send(f"position {position}")
    started = time.monotonic()
    engine.send(f"go depth {depth}")
    lines, finished = engine.until("bestmove", started + limit)
    if not finished:
        engine.send("stop")
        rest, _ = engine.until("bestmove", None)
        lines += rest
    seconds = time.monotonic() - started
    last = (0, "-", 0)
    for line in lines:
        m = INFO_RE.match(line)
        # 窓を外した反復（lowerbound/upperbound）は確定値でないので数えない
        if m and "bound" not in line:
            last = (int(m[1]), f"{m[2]} {m[3]}", int(m[4]))
    return Reach(index, seconds, finished, *last)


def run(
    engine_path: str,
    positions: Path,
    *,
    eval_file: str,
    depth: int,
    limit: float,
    threads: int,
    hash_mb: int,
    only: list[int] | None,
    log: Path,
) -> int:
    items = list(enumerate(read_positions(positions), 1))
    if only:
        items = [(i, p) for i, p in items if i in set(only)]
    engine = Engine(
        engine_path,
        {"EvalFile": eval_file, "Threads": str(threads), "USI_Hash": str(hash_mb)},
    )
    log.parent.mkdir(parents=True, exist_ok=True)
    results: list[Reach] = []
    with log.open("a", encoding="utf-8") as f:
        head = (
            f"# reach engine={engine_path} eval={eval_file} positions={positions} "
            f"depth={depth} limit={limit}s threads={threads} hash={hash_mb}"
        )
        f.write(head + "\n")
        print(
            f"{'番号':>4} {'秒':>6} {'完了':>4} {'深さ':>4} {'評価値':>12} {'ノード':>14}"
        )
        for index, position in items:
            r = measure(engine, index, position, depth, limit)
            results.append(r)
            row = (
                f"{r.index:>4} {r.seconds:>6.2f} {'yes' if r.finished else 'NO':>4} "
                f"{r.depth:>4} {r.score:>12} {r.nodes:>14,}"
            )
            # 終わらなかった局面と5秒を超えた局面だけを画面へ出す。全行はログに残す
            if not r.finished or r.seconds > 5:
                print(row, flush=True)
            f.write(row + "\n")
        engine.close()
        times = sorted(r.seconds for r in results)
        median = times[len(times) // 2] if times else 0.0
        stopped = [r.index for r in results if not r.finished]
        summary = (
            f"局面 {len(results)} / 所要の中央値 {median:.2f}秒 / "
            f"5秒超 {sum(t > 5 for t in times)}件 / {limit:g}秒で打ち切り {len(stopped)}件"
            f"{'（' + ', '.join(map(str, stopped)) + '番）' if stopped else ''}"
        )
        print(summary)
        f.write(f"# {summary}\n")
    return 0
