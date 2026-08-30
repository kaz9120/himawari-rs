"""対局ゲートのログから結果を抜き、そのまま貼れる形にする。

毎回ログを目で読んでコミットトレーラと表の行を書き写していた。数字の転記
ミスは後から気づけないので、ここで機械的に作る。

判定が出た走行は結果ファイル（`data/sprt/<名前>.result`）を残す。
**このファイルの有無が「完了したか」の定義になる**（ADR-0175）。判定に
至っていない走行では書かない。中途半端な結果を完了として記録しないためである。
"""

from __future__ import annotations

import datetime
import os
import re
from pathlib import Path

# 判定行の例:
#   H1採択（候補は有意に強い） | pairs 525 games 1050 | +602 =46 -402 | Elo +67.0 [...] | LLR +3.05
VERDICT_RE = re.compile(r"^(H1採択|H0採択|判定に至らず)")
# 途中経過の例:
#   pairs   525 | +602 =46 -402 | [73,22,236,20,174] | Elo +67.0 [...] | LLR +3.05 [-2.94,2.94]
PAIRS_LINE_PREFIX = "pairs "

ELO_RE = re.compile(r"Elo ([+-][0-9.]+) (\[[+-][0-9.]+,[+-][0-9.]+\])")
LLR_RE = re.compile(r"LLR ([+-][0-9.]+)")
WDL_RE = re.compile(r"(\+[0-9]+ =[0-9]+ -[0-9]+)")
GAMES_RE = re.compile(r"games ([0-9]+)")
PAIRS_NUM_RE = re.compile(r"pairs +([0-9]+)")
# 起動行の例:
#   selfplay: cand vs base | tc 10+0.1 | 並列 3 | SPRT elo[-5, 0] α=0.05 β=0.05 | ...
HYPOTHESIS_RE = re.compile(r"SPRT elo\[(-?[0-9.]+), *(-?[0-9.]+)\]")

# 既定の対立仮説（CLAUDE.mdの対局ゲート）
DEFAULT_HYPOTHESIS = ("0", "5")
# 非劣性の対立仮説（ADR-0163）
NON_INFERIORITY_HYPOTHESIS = ("-5", "0")

# 判定を終了コードへ写す。0=H1、1=H0、2=判定に至らず
EXIT_BY_VERDICT = {"H1": 0, "H0": 1, "打ち切り": 2, "判定前": 2}


class Unreadable(Exception):
    """ログから結果を読み取れない。"""


def last_run_lines(lines: list[str]) -> list[str]:
    """最後の起動行以降だけを返す。起動行が無ければ全体を返す。

    ログは追記式で、再開分も同じファイルへ積む（ADR-0087）。前の走行の
    判定行が残っているので、全体から探すと古い結果を拾う。再開後の行は
    通算値を出すため、最後の走行だけを見れば累積の結果になる。
    """
    start = 0
    for i, line in enumerate(lines):
        if HYPOTHESIS_RE.search(line):
            start = i
    return lines[start:]


def find_source_line(lines: list[str]) -> tuple[str | None, str | None]:
    """判定行、無ければ最終の途中経過行を返す。(行, 判定) を組で返す。"""
    verdict_line = None
    for line in lines:
        if VERDICT_RE.match(line):
            verdict_line = line
    if verdict_line is not None:
        if verdict_line.startswith("H1"):
            return verdict_line, "H1"
        if verdict_line.startswith("H0"):
            return verdict_line, "H0"
        return verdict_line, "打ち切り"

    pairs_line = None
    for line in lines:
        if line.startswith(PAIRS_LINE_PREFIX):
            pairs_line = line
    if pairs_line is not None:
        # 判定行がまだ無い。実行中の途中経過か、判定前に止まったログ
        return pairs_line, "判定前"

    return None, None


def find_hypothesis(lines: list[str]) -> tuple[str, str] | None:
    """最後の起動行から (elo0, elo1) を返す。無ければNone。"""
    found = None
    for line in lines:
        m = HYPOTHESIS_RE.search(line)
        if m:
            found = (m.group(1), m.group(2))
    return found


def hypothesis_note(hyp: tuple[str, str] | None) -> str:
    """既定でない対立仮説なら、トレーラへ添える注記を返す（ADR-0163）。

    条件を書かずに数値だけが独り歩きするのを防ぐ。
    """
    if hyp is None or hyp == DEFAULT_HYPOTHESIS:
        return ""
    if hyp == NON_INFERIORITY_HYPOTHESIS:
        return "（非劣性 elo0=-5 elo1=0）"
    return f"（elo0={hyp[0]} elo1={hyp[1]}）"


def parse_fields(src: str) -> dict[str, object]:
    """結果行からElo・CI・LLR・W-D-L・対局数を取り出す。"""
    m_elo = ELO_RE.search(src)
    m_llr = LLR_RE.search(src)
    m_wdl = WDL_RE.search(src)
    if not (m_elo and m_llr and m_wdl):
        raise Unreadable(f"結果行の形式を読み取れない: {src}")

    m_games = GAMES_RE.search(src)
    if m_games:
        games = int(m_games.group(1))
    else:
        m_pairs = PAIRS_NUM_RE.search(src)
        if not m_pairs:
            raise Unreadable(f"対局数を読み取れない: {src}")
        games = int(m_pairs.group(1)) * 2

    return {
        "elo_num": m_elo.group(1),
        "elo_ci": m_elo.group(2),
        "llr": m_llr.group(1),
        "wdl": m_wdl.group(1),
        "games": games,
    }


def build_report(name: str, verdict: str, fields: dict, note: str = "") -> str:
    """3形式（トレーラ・結果表・PR本文の表）を1つの文字列にする。"""
    elo_num, elo_ci = fields["elo_num"], fields["elo_ci"]
    llr, wdl, games = fields["llr"], fields["wdl"], fields["games"]

    if verdict == "打ち切り":
        row = f"| {name} | **{elo_num} {elo_ci}**（{games}局、LLR {llr}で打ち切り）{note} |"
    elif verdict == "判定前":
        row = (
            f"| {name} | {elo_num} {elo_ci}"
            f"（{games}局、LLR {llr}、判定前の途中経過）{note} |"
        )
    else:
        row = (
            f"| {name} | **{elo_num} {elo_ci}**"
            f"（{games}局、LLR {llr}で{verdict}採択）{note} |"
        )

    return "\n".join(
        [
            f"=== {name}（{verdict}{note}） ===",
            "",
            "--- コミットのトレーラ ---",
            f"SPRT: {elo_num} {elo_ci} {games}games {verdict}{note}",
            "",
            "--- 結果表の行 ---",
            "| 比較 | 結果 |",
            "|---|---|",
            row,
            "",
            "--- PR本文の表 ---",
            "| 項目 | 値 |",
            "|---|---|",
            f"| 対局数 | {games}（{games // 2}ペア） |",
            f"| W-D-L | {wdl} |",
            f"| Elo [95%CI] | **{elo_num} {elo_ci}** |",
            f"| LLR | {llr} |",
            f"| 判定 | **{verdict}**{note} |",
        ]
    )


def write_result(path: Path, name: str, verdict: str, fields: dict, hyp) -> None:
    """判定が出た走行の結果を key=value のファイルへ書く（ADR-0175）。

    書き込みは一時ファイル経由のrenameで行う。途中まで書けたファイルを
    完了と誤読させないためである。
    """
    ci = str(fields["elo_ci"]).strip("[]").split(",")
    elo0, elo1 = hyp if hyp else DEFAULT_HYPOTHESIS
    stamp = datetime.datetime.now(datetime.timezone.utc)
    lines = [
        f"name={name}",
        f"decision={verdict}",
        f"elo={fields['elo_num']}",
        f"ci_low={ci[0]}",
        f"ci_high={ci[1]}",
        f"games={fields['games']}",
        f"wdl={fields['wdl']}",
        f"llr={fields['llr']}",
        f"elo0={elo0}",
        f"elo1={elo1}",
        f"finished_at={stamp:%Y-%m-%dT%H:%M:%SZ}",
    ]
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text("\n".join(lines) + "\n", encoding="utf-8")
    os.replace(tmp, path)


def report(log: Path, name: str, result: Path | None = None) -> tuple[str, str]:
    """ログを読んで (整形した報告, 判定) を返す。

    resultを渡し、判定が出ていればその場所へ結果ファイルを書く。
    """
    if not log.is_file():
        raise Unreadable(f"ログがない: {log}")

    lines = last_run_lines(log.read_text(encoding="utf-8").splitlines())
    src, verdict = find_source_line(lines)
    if src is None or verdict is None:
        # 起動直後はpairs行がまだない。エラーではなく走行前として報告する
        if any(line.startswith("selfplay:") or "--baseline" in line for line in lines):
            return f"=== {name}（判定前） ===\n\nまだ対局結果がない（起動直後）。", "判定前"
        raise Unreadable(f"結果行が見つからない: {log}")

    fields = parse_fields(src)
    hyp = find_hypothesis(lines)
    text = build_report(name, verdict, fields, hypothesis_note(hyp))

    if result is not None and verdict in ("H1", "H0"):
        write_result(result, name, verdict, fields, hyp)
    return text, verdict
