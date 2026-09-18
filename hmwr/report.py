"""実験の数値を、成果物からMarkdownの表へ写す（ADR-0209）。

**LLMを通さない。** 対局は結果ファイル、学習は実験台帳、データは完了印から
読む。ログを目で読んで書き写すと、転記の誤りに後から気づけない。結果の記録を
書く側（人でもLLMでも）は、ここが出した表をそのまま貼る。
"""

from __future__ import annotations

import csv
import json
from pathlib import Path

from . import paths, sprt_log

REGISTRY = "training/runs/registry.tsv"


def _option(argv: list[str], flag: str) -> str:
    return argv[argv.index(flag) + 1] if flag in argv and argv.index(flag) + 1 < len(argv) else ""


# --- 対局 --------------------------------------------------------------


def match_row(name: str, note: str = "") -> list[str]:
    """対局1本を表の1行にする。結果ファイルが無ければ、ログの途中経過を読む。"""
    result = paths.SPRT / f"{name}.result"
    if result.is_file():
        f = dict(
            line.split("=", 1) for line in result.read_text(encoding="utf-8").splitlines() if "=" in line
        )
        return [
            name, note, f["games"], f["wdl"],
            f"{f['elo']} [{f['ci_low']}, {f['ci_high']}]", f["llr"], f["decision"],
            _timeloss(name, f.get("timeloss")),
        ]  # fmt: skip
    try:
        lines = sprt_log.last_run_lines(
            (paths.LOGS / f"sprt-{name}.log").read_text(encoding="utf-8").splitlines()
        )
        src, verdict = sprt_log.find_source_line(lines)
        f = sprt_log.parse_fields(src) if src else None
    except (OSError, sprt_log.Unreadable):
        f = None
    if f is None:
        return [name, note, "", "", "", "", "未着手", ""]
    # 結果ファイルが無い走行。判定行まで出ていれば打ち切り、無ければ走行中か中断
    state = "打ち切り" if verdict == "打ち切り" else "途中"
    ci = str(f["elo_ci"]).strip("[]").replace(",", ", ")
    elo = f"{f['elo_num']} [{ci}]"
    return [name, note, str(f["games"]), str(f["wdl"]), elo, str(f["llr"]), state, _timeloss(name)]


def _timeloss(name: str, recorded: str | None = None) -> str:
    """切れ負けの局数。結果ファイルに無い古い走行は、棋譜を数える。"""
    if recorded is None:
        counts = sprt_log.reasons(paths.SPRT / f"{name}.jsonl")
        if not counts:
            return ""
        recorded = str(counts.get("timeloss", 0))
        games = sum(counts.values())
    else:
        games = 0
    lost = int(recorded)
    if games and lost / games > sprt_log.TIMELOSS_WARN_RATE:
        return f"**{lost}**"
    return recorded


MATCH_HEAD = ["対局", "baseline → candidate", "局数", "W-D-L", "Elo [95%CI]", "LLR", "判定", "切れ負け"]


# --- 学習 --------------------------------------------------------------


def train_row(name: str) -> list[str]:
    """学習1本を表の1行にする。台帳に同じ名前が複数あれば、最後の行を使う。"""
    registry = paths.REPO / REGISTRY
    row = None
    if registry.is_file():
        with open(registry, encoding="utf-8", newline="") as fh:
            for r in csv.DictReader(fh, delimiter="\t"):
                if r.get("name") == name:
                    row = r
    if row is None:
        return [name, "", "", "", "", "", "台帳に無い（未了か失敗）"]
    hours = f"{int(row['elapsed_s']) / 3600:.1f}時間" if row.get("elapsed_s") else ""
    return [
        name, row["data"], f"{int(row['data_n']):,}", row["total_steps"],
        f"{row['best_valid']}（step {row['best_step']}）", row["final_valid"], hours,
    ]  # fmt: skip


TRAIN_HEAD = ["ネット", "学習データ", "局面数", "ステップ", "最良のvalid", "最終のvalid", "所要"]


# --- データ ------------------------------------------------------------


def data_row(name: str) -> list[str] | None:
    for suffix in (".psv", ".rankpsv"):
        done = paths.TRAIN / f"{name}{suffix}.done"
        if done.is_file():
            info = json.loads(done.read_text(encoding="utf-8"))
            positions = f"{info['bytes'] // 40:,}" if suffix == ".psv" else ""
            return [name + suffix, positions, f"{info['bytes']:,}", f"{info.get('seconds', '')}秒"]
    return None


DATA_HEAD = ["データ", "局面数", "バイト", "所要"]


# --- 組み立て ----------------------------------------------------------


def table(head: list[str], rows: list[list[str]]) -> str:
    lines = ["| " + " | ".join(head) + " |", "|" + "---|" * len(head)]
    lines += ["| " + " | ".join(r) + " |" for r in rows]
    return "\n".join(lines)


def from_steps(steps: list[list[str]]) -> tuple[list, list, list]:
    """specのステップ（hmwrの後ろの引数）から、対局・学習・データの名前を拾う。"""
    matches, nets, data = [], [], []
    for argv in steps:
        head = tuple(argv[:2])
        if head == ("match", "run") and len(argv) > 2:
            base = _option(argv, "--base-net") or _option(argv, "--base-build") or "既定"
            cand = _option(argv, "--cand-net") or _option(argv, "--cand-build") or "既定"
            matches.append((argv[2], f"{base} → {cand}"))
        elif head == ("sprt", "run") and len(argv) > 2:
            matches.append((argv[2], "origin/main → HEAD"))
        elif head == ("sprt", "net") and len(argv) > 4:
            matches.append((argv[4], f"{Path(argv[2]).stem} → {Path(argv[3]).stem}"))
        elif head == ("net", "train") and len(argv) > 2:
            nets.append(argv[2])
        elif argv[:1] == ["data"] and len(argv) > 2 and argv[1] not in ("rm", "stats", "openings"):
            data.append(argv[2])
    return matches, nets, data


def render(matches: list[tuple[str, str]], nets: list[str], data: list[str]) -> str:
    parts = []
    if matches:
        parts.append("### 対局\n\n" + table(MATCH_HEAD, [match_row(n, note) for n, note in matches]))
    if nets:
        parts.append("### 学習\n\n" + table(TRAIN_HEAD, [train_row(n) for n in nets]))
    rows = [r for r in (data_row(n) for n in data) if r]
    if rows:
        parts.append("### データ\n\n" + table(DATA_HEAD, rows))
    return "\n\n".join(parts) if parts else "（表にする成果物がない）"
