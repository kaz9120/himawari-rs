"""USIログ（`DebugLogFile`）を1局ごとのテレメトリへ構造化する（issue #748）。

ログの1行は `<UNIX時刻のミリ秒> <向き> <USIの行>` である。向きは `<` が受信、
`>` が送信、`!` がpanicの記録（`crates/usi/src/main.rs` の `log_line`）。
送信の行は非ASCIIを `\\u{5c06}` の形で書く（#745）ので、読むときに戻す。

1局は `usinewgame` から `gameover` までとする。ログが局の途中で始まって
いても（日毎のファイルの境目）、最初の `go` から局として拾う。出力の形は
`SCHEMA` の版で区別し、壊す変更は版を上げる（ADR-0221）。
"""

from __future__ import annotations

import datetime
import re
from collections.abc import Iterable
from pathlib import Path

SCHEMA = "himawari-usilog-game/1"

# パスを値に持つオプション。公開データへ出さない（ADR-0221の3）
PATH_OPTIONS = frozenset({"EvalFile", "BookFile", "DebugLogFile"})

_LINE = re.compile(r"^(\d+) ([<>!]) ?(.*)$")
_ESCAPED = re.compile(r"\\u\{([0-9a-fA-F]{1,6})\}")
_WAIT_THREADS = re.compile(r"^waiting for threads (\S+) elapsed (\d+)$")
_WAIT_PREVIOUS = re.compile(r"^waiting for previous search: thread (\d+) elapsed (\d+)$")
_INFO_NUMBERS = ("depth", "seldepth", "multipv", "nodes", "nps", "time", "hashfull")
_GO_NUMBERS = ("btime", "wtime", "binc", "winc", "byoyomi", "movetime", "depth", "nodes")


def unescape(text: str) -> str:
    """`\\u{5c06}` の形を元の文字へ戻す。"""
    return _ESCAPED.sub(lambda m: chr(int(m.group(1), 16)), text)


def read_lines(paths: Iterable[Path]) -> list[tuple[int, str, str]]:
    """ログを読み、時刻順に並べた (ms, 向き, 本文) を返す。

    日毎に分けたファイルは、先頭の時刻の順に並べてつなぐ。ファイルの中は
    書かれた順を保つ。同じミリ秒の行（time planとbestmoveなど）の前後を
    崩さないためである。形の合わない行（書きかけの末尾など）は捨てる。
    """
    blocks: list[list[tuple[int, str, str]]] = []
    for path in paths:
        text = Path(path).read_bytes().decode("utf-8", errors="replace")
        block = []
        for raw in text.splitlines():
            m = _LINE.match(raw)
            if m:
                block.append((int(m.group(1)), m.group(2), m.group(3)))
        if block:
            blocks.append(block)
    blocks.sort(key=lambda b: b[0][0])
    return [row for block in blocks for row in block]


def parse_info(text: str) -> dict:
    """`info depth ... pv ...` の反復の行を辞書にする。"""
    tokens = text.split()[1:]
    out: dict = {}
    i = 0
    while i < len(tokens):
        key = tokens[i]
        if key in _INFO_NUMBERS and i + 1 < len(tokens):
            out[key] = int(tokens[i + 1])
            i += 2
        elif key == "score" and i + 2 < len(tokens):
            out["score"] = {tokens[i + 1]: _score_value(tokens[i + 2])}
            i += 3
        elif key in ("lowerbound", "upperbound"):
            out["bound"] = key
            i += 1
        elif key == "pv":
            out["pv"] = tokens[i + 1 :]
            break
        else:
            i += 1
    return out


def _score_value(token: str) -> int | str:
    # 詰みの手数が不明なときは `mate +` / `mate -` になる
    try:
        return int(token)
    except ValueError:
        return token


def parse_time_plan(text: str) -> dict:
    """`time plan: minimum 1880 ... stop true` を辞書にする。"""
    tokens = text.split(":", 1)[1].split()
    out: dict = {}
    for key, value in zip(tokens[::2], tokens[1::2]):
        if value in ("true", "false"):
            out[key] = value == "true"
        else:
            try:
                out[key] = int(value)
            except ValueError:
                out[key] = value
    return out


def parse_go(text: str) -> dict:
    tokens = text.split()[1:]
    out: dict = {"ponder": "ponder" in tokens}
    for key, value in zip(tokens, tokens[1:]):
        if key in _GO_NUMBERS:
            try:
                out[key] = int(value)
            except ValueError:
                pass
    if "infinite" in tokens:
        out["infinite"] = True
    return out


def parse_position(text: str) -> tuple[int | None, str | None, str | None]:
    """局面の行から、これから指す手の手数・手番・直前の手を返す。"""
    tokens = text.split()[1:]
    moves = tokens[tokens.index("moves") + 1 :] if "moves" in tokens else []
    last = moves[-1] if moves else None
    if tokens[:1] == ["startpos"]:
        ply = 1 + len(moves)
        return ply, "b" if ply % 2 == 1 else "w", last
    if tokens[:1] == ["sfen"] and len(tokens) >= 5:
        side, number = tokens[2], tokens[4]
        try:
            start = int(number)
        except ValueError:
            return None, None, last
        ply = start + len(moves)
        flip = len(moves) % 2 == 1
        if side not in ("b", "w"):
            return ply, None, last
        return ply, ({"b": "w", "w": "b"}[side] if flip else side), last
    return None, None, last


class _Game:
    def __init__(self, start_ms: int, started_by: str, engine: dict) -> None:
        self.start_ms = start_ms
        self.data: dict = {
            "schema": SCHEMA,
            "start_ms": start_ms,
            "end_ms": start_ms,
            "started_by": started_by,
            "ended_by": "log_end",
            "result": None,
            "engine": {"name": engine.get("name"), "options": dict(engine["options"])},
            "moves": [],
            "events": [],
        }
        self.position: tuple[int | None, str | None, str | None] = (None, None, None)
        self.move: dict | None = None
        self.go_ms = 0

    def open_move(self, ms: int, go: dict) -> None:
        self.close_move()
        ply, side, last = self.position
        self.go_ms = ms
        self.move = {
            "ply": ply,
            "side": side,
            "last_move": last,
            "t_go": ms - self.start_ms,
            "go": go,
            "ponderhit_ms": None,
            "stop_ms": None,
            "bestmove_ms": None,
            "think_ms": None,
            "outcome": "no_bestmove",
            "bestmove": None,
            "ponder": None,
            "iterations": [],
            "last_currmove": None,
            "time_plan": None,
            "waits": [],
            "notes": [],
        }

    def close_move(self) -> None:
        if self.move is not None:
            self.move["last_pv_change_depth"] = last_pv_change(self.move["iterations"])
            self.data["moves"].append(self.move)
            self.move = None

    def event(self, ms: int, kind: str, **fields) -> None:
        """手の外で起きたこと。手の中なら手へ、外なら局へ付ける。"""
        if self.move is not None:
            target = self.move["waits"] if kind.startswith("wait") else self.move["notes"]
            target.append({"t": ms - self.go_ms, "kind": kind, **fields})
        else:
            self.data["events"].append({"t": ms - self.start_ms, "kind": kind, **fields})

    def receive(self, ms: int, text: str) -> None:
        word = text.split(" ", 1)[0]
        if word == "position":
            self.position = parse_position(text)
        elif word == "go":
            self.open_move(ms, parse_go(text))
        elif word == "ponderhit" and self.move is not None:
            self.move["ponderhit_ms"] = ms - self.go_ms
        elif word == "stop" and self.move is not None:
            self.move["stop_ms"] = ms - self.go_ms

    def send(self, ms: int, text: str) -> None:
        if text.startswith("bestmove") and self.move is not None:
            self.finish_move(ms, text)
        elif text.startswith("info string "):
            self.note(ms, unescape(text[len("info string ") :]))
        elif text.startswith("info ") and self.move is not None:
            self.info(ms, text)

    def finish_move(self, ms: int, text: str) -> None:
        move = self.move
        assert move is not None
        tokens = text.split()
        move["bestmove"] = tokens[1] if len(tokens) > 1 else None
        if len(tokens) > 3 and tokens[2] == "ponder":
            move["ponder"] = tokens[3]
        move["bestmove_ms"] = ms - self.go_ms
        if move["go"]["ponder"] and move["ponderhit_ms"] is None:
            # stopで打ち切られた先読み。この指し手はGUIに捨てられる
            move["outcome"] = "ponder_miss"
        else:
            move["outcome"] = "bestmove"
            move["think_ms"] = move["bestmove_ms"] - (move["ponderhit_ms"] or 0)
        self.close_move()

    def info(self, ms: int, text: str) -> None:
        move = self.move
        assert move is not None
        info = parse_info(text)
        t = ms - self.go_ms
        if "currmove" in text.split():
            tokens = text.split()
            move["last_currmove"] = {
                "t": t,
                "depth": info.get("depth"),
                "move": tokens[tokens.index("currmove") + 1],
            }
        elif "pv" in info:
            move["iterations"].append({"t": t, **info})

    def note(self, ms: int, text: str) -> None:
        if text.startswith("time plan:") and self.move is not None:
            self.move["time_plan"] = {"t": ms - self.go_ms, **parse_time_plan(text)}
            return
        m = _WAIT_THREADS.match(text)
        if m:
            threads = [int(x) for x in m.group(1).split(",") if x.isdigit()]
            self.event(ms, "wait_threads", threads=threads, elapsed=int(m.group(2)))
            return
        m = _WAIT_PREVIOUS.match(text)
        if m:
            self.event(
                ms, "wait_previous_search", threads=[int(m.group(1))], elapsed=int(m.group(2))
            )
            return
        self.event(ms, "info_string", text=text)

    def end(self, ms: int, ended_by: str, result: str | None = None) -> dict:
        self.close_move()
        self.data["end_ms"] = ms
        self.data["ended_by"] = ended_by
        self.data["result"] = result
        self.data["summary"] = summarize(self.data["moves"])
        return self.data


def summarize(moves: list[dict]) -> dict:
    """局の要約。平均は自分の手番で指した手（bestmove）だけで取る。"""
    played = [m for m in moves if m["outcome"] == "bestmove"]
    finals = [m["iterations"][-1] for m in played if m["iterations"]]
    depths = [it["depth"] for it in finals if "depth" in it]
    nodes = sum(it.get("nodes", 0) for it in finals)
    time_ms = sum(it.get("time", 0) for it in finals)
    thinks = [m["think_ms"] for m in played if m["think_ms"] is not None]
    return {
        "moves": len(played),
        "ponder_hits": sum(1 for m in played if m["ponderhit_ms"] is not None),
        "ponder_misses": sum(1 for m in moves if m["outcome"] == "ponder_miss"),
        "no_bestmove": sum(1 for m in moves if m["outcome"] == "no_bestmove"),
        "avg_depth": round(sum(depths) / len(depths), 2) if depths else None,
        "avg_nps": nodes * 1000 // time_ms if time_ms else None,
        "max_think_ms": max(thinks) if thinks else None,
        "waits": sum(len(m["waits"]) for m in moves),
    }


def last_pv_change(iterations: list[dict]) -> int | None:
    """最善手が最後に入れ替わった反復の深さ。入れ替わりがなければNone。"""
    found = None
    prev = None
    for it in iterations:
        head = it["pv"][0] if it.get("pv") else None
        if prev is not None and head is not None and head != prev:
            found = it.get("depth")
        if head is not None:
            prev = head
    return found


def split_games(rows: Iterable[tuple[int, str, str]]) -> list[dict]:
    """時刻順の行を1局ごとのテレメトリへ分ける。"""
    games: list[dict] = []
    engine: dict = {"name": None, "options": {}}
    game: _Game | None = None
    last_ms = 0
    for ms, direction, text in rows:
        last_ms = ms
        word = text.split(" ", 1)[0]
        if direction == "!":
            if game is not None:
                game.event(ms, "panic", text=text)
                games.append(game.end(ms, "panic"))
                game = None
            continue
        if direction == ">" and text.startswith("id name "):
            engine["name"] = unescape(text[len("id name ") :])
            continue
        if direction == "<" and word == "setoption":
            _set_option(engine["options"], text)
            continue
        if direction == "<" and word == "usinewgame":
            if game is not None:
                games.append(game.end(ms, "usinewgame"))
            game = _Game(ms, "usinewgame", engine)
            continue
        if direction == "<" and word == "gameover":
            if game is not None:
                parts = text.split()
                games.append(game.end(ms, "gameover", parts[1] if len(parts) > 1 else None))
                game = None
            continue
        if direction == "<" and word == "quit":
            if game is not None:
                games.append(game.end(ms, "quit"))
                game = None
            continue
        if game is None:
            # 局の途中からのログ。最初のgoから局として扱う
            if direction == "<" and word in ("position", "go"):
                game = _Game(ms, "log_start", engine)
            else:
                continue
        if direction == "<":
            game.receive(ms, text)
        else:
            game.send(ms, text)
    if game is not None:
        games.append(game.end(last_ms, "log_end"))
    return games


def _set_option(options: dict, text: str) -> None:
    tokens = text.split()
    if "name" not in tokens:
        return
    i = tokens.index("name")
    if "value" in tokens:
        j = tokens.index("value")
        name, value = " ".join(tokens[i + 1 : j]), " ".join(tokens[j + 1 :])
    else:
        name, value = " ".join(tokens[i + 1 :]), None
    if name in PATH_OPTIONS:
        return
    options[name] = value


def game_name(game: dict) -> str:
    """ファイル名に使う開始時刻（UTC）。"""
    t = datetime.datetime.fromtimestamp(game["start_ms"] / 1000, tz=datetime.timezone.utc)
    return t.strftime("%Y%m%d-%H%M%S")


def table(games: list[dict]) -> str:
    """1局1行の表。異常（bestmoveなし・待ち）が目で拾えるようにする。"""
    head = (
        "| 開始（UTC） | 終局 | 結果 | 手数 | ponderhit | 平均深さ | 平均NPS "
        "| 最長思考ms | bestmoveなし | 待ち |"
    )
    lines = [head, "|" + "---|" * 10]
    for g in games:
        s = g["summary"]
        lines.append(
            f"| {game_name(g)} | {g['ended_by']} | {g['result'] or '-'} | {s['moves']} "
            f"| {s['ponder_hits']} | {_or_dash(s['avg_depth'])} | {_or_dash(s['avg_nps'])} "
            f"| {_or_dash(s['max_think_ms'])} | {s['no_bestmove']} | {s['waits']} |"
        )
    return "\n".join(lines)


def _or_dash(value) -> str:
    return "-" if value is None else str(value)
