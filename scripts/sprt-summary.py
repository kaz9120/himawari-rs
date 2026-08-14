#!/usr/bin/env python3
"""SPRTのログから結果を抜き、そのまま貼れる形で出す（ADR-0081）。

毎回ログを目で読んでコミットトレーラとRESULTS.mdの行を書き写していた。
数字の転記ミスは後から気づけない。ここで機械的に作る。

`scripts/sprt-summary.sh` のPython移植（ADR-0122）。出力形式は移植前と
1文字も変えていない。旧shell版は `set -e` を外していた。判定行を
grepで探して1件も無いとき、grepは非0で終わるが、それは異常ではなく
「打ち切りなので最終pairs行を使う」への正常な分岐だったためである。
Pythonでは例外を投げずNoneのまま次の分岐へ進む形で同じ制御を表す。

終了コード: 0=H1、1=H0、2=判定に至らず、3=読めない。
"""

import argparse
import os
import re
import sys

USAGE = """\
使い方:
  scripts/sprt-summary.py <SPRTのログファイル> [機能名]

判定に達していれば結論行から、達していなければ最終のpairs行から作る。
出力は3つ。

  1. コミットの SPRT: トレーラ（ADR-0071の書式）
  2. RESULTS.md へ貼る表の行
  3. PR本文へ貼る表

終了コード: 0=H1、1=H0、2=判定に至らず、3=読めない。
"""

# 判定行の例:
#   H1採択（候補は有意に強い） | pairs 525 games 1050 | +602 =46 -402 | Elo +67.0 [+46.4,+88.0] | LLR +3.05
VERDICT_RE = re.compile(r"^(H1採択|H0採択|判定に至らず)")
# 途中経過の例:
#   pairs   525 | +602 =46 -402 | [73,22,236,20,174] | Elo +67.0 [+46.4,+88.0] | LLR +3.05 [-2.94,2.94]
PAIRS_LINE_PREFIX = "pairs "

ELO_RE = re.compile(r"Elo ([+-][0-9.]+) (\[[+-][0-9.]+,[+-][0-9.]+\])")
LLR_RE = re.compile(r"LLR ([+-][0-9.]+)")
WDL_RE = re.compile(r"(\+[0-9]+ =[0-9]+ -[0-9]+)")
GAMES_RE = re.compile(r"games ([0-9]+)")
PAIRS_NUM_RE = re.compile(r"pairs +([0-9]+)")
# 起動行の例:
#   selfplay: cand vs base | tc 10+0.1 | 並列 3 | SPRT elo[-5, 0] α=0.05 β=0.05 | 開始局面 30053件
HYPOTHESIS_RE = re.compile(r"SPRT elo\[(-?[0-9.]+), *(-?[0-9.]+)\]")

# 既定の対立仮説（CLAUDE.mdのSPRTゲート）。
DEFAULT_HYPOTHESIS = ("0", "5")
# 非劣性の対立仮説（ADR-0163）。
NON_INFERIORITY_HYPOTHESIS = ("-5", "0")

EXIT_BY_VERDICT = {"H1": 0, "H0": 1, "打ち切り": 2, "判定前": 2}


class ArgParser(argparse.ArgumentParser):
    """引数エラーを旧shell版と同じ書式・終了コード3に揃える。"""

    def error(self, message):
        print(USAGE, end="")
        sys.exit(3)


def build_parser():
    parser = ArgParser(add_help=False)
    parser.add_argument("log")
    parser.add_argument("feature", nargs="?", default=None)
    return parser


def default_feature(log_path):
    """機能名の省略時は、ログのbasenameから .log を外した名を使う。"""
    base = os.path.basename(log_path)
    if base.endswith(".log"):
        base = base[: -len(".log")]
    return base


def find_source_line(lines):
    """判定行、無ければ最終pairs行を返す。(元の行, 判定) のタプル。

    どちらも無ければ (None, None)。
    """
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


def last_run_lines(lines):
    """最後の起動行以降だけを返す。起動行が無ければ全体を返す。

    ログは追記式で、`--resume` の再開分も同じファイルへ積む
    （[ADR-0087](../docs/adr/0087-sprt-resume.md)）。前の走行の判定行が
    残っているので、全体から探すと古い結果を拾う。再開後のpairs行は通算値を
    出すため、最後の走行だけを見れば累積の結果になる。
    """
    start = 0
    for i, line in enumerate(lines):
        if HYPOTHESIS_RE.search(line):
            start = i
    return lines[start:]


def find_hypothesis(lines):
    """最後の起動行から (elo0, elo1) を返す。無ければ None。"""
    found = None
    for line in lines:
        m = HYPOTHESIS_RE.search(line)
        if m:
            found = (m.group(1), m.group(2))
    return found


def hypothesis_note(hyp):
    """既定でない対立仮説なら、トレーラへ添える注記を返す（ADR-0163）。

    既定（elo0=0, elo1=5）のときは空文字。非劣性は名前で呼ぶ。
    """
    if hyp is None or hyp == DEFAULT_HYPOTHESIS:
        return ""
    if hyp == NON_INFERIORITY_HYPOTHESIS:
        return "（非劣性 elo0=-5 elo1=0）"
    return f"（elo0={hyp[0]} elo1={hyp[1]}）"


def parse_fields(src):
    """結果行からElo・CI・LLR・W-D-L・対局数を取り出す。

    games フィールドが無ければ pairs から2倍して求める。
    """
    m_elo = ELO_RE.search(src)
    m_llr = LLR_RE.search(src)
    m_wdl = WDL_RE.search(src)
    if not (m_elo and m_llr and m_wdl):
        raise ValueError(f"結果行の形式を読み取れない: {src}")

    m_games = GAMES_RE.search(src)
    if m_games:
        games = int(m_games.group(1))
    else:
        m_pairs = PAIRS_NUM_RE.search(src)
        if not m_pairs:
            raise ValueError(f"対局数を読み取れない: {src}")
        games = int(m_pairs.group(1)) * 2

    return {
        "elo_num": m_elo.group(1),
        "elo_ci": m_elo.group(2),
        "llr": m_llr.group(1),
        "wdl": m_wdl.group(1),
        "games": games,
    }


def build_report(feature, verdict, fields, note=""):
    """3形式（トレーラ・RESULTS.md表・PR本文表）を1つの文字列にする。

    noteは既定でない対立仮説の注記（ADR-0163）。トレーラと表の両方へ入れ、
    条件を書かずに数値だけが独り歩きするのを防ぐ。
    """
    elo_num, elo_ci = fields["elo_num"], fields["elo_ci"]
    llr, wdl, games = fields["llr"], fields["wdl"], fields["games"]

    if verdict == "打ち切り":
        results_row = f"| {feature} | **{elo_num} {elo_ci}**（{games}局、LLR {llr}で打ち切り）{note} |"
    elif verdict == "判定前":
        results_row = f"| {feature} | {elo_num} {elo_ci}（{games}局、LLR {llr}、判定前の途中経過）{note} |"
    else:
        results_row = (
            f"| {feature} | **{elo_num} {elo_ci}**（{games}局、LLR {llr}で{verdict}採択）{note} |"
        )

    lines = [
        f"=== {feature}（{verdict}{note}） ===",
        "",
        "--- コミットのトレーラ（ADR-0071） ---",
        f"SPRT: {elo_num} {elo_ci} {games}games {verdict}{note}",
        "",
        "--- RESULTS.md の表 ---",
        "| 比較 | 結果 |",
        "|---|---|",
        results_row,
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
    return "\n".join(lines)


def main(argv=None):
    parser = build_parser()
    args = parser.parse_args(argv)

    log_path = args.log
    feature = args.feature if args.feature is not None else default_feature(log_path)

    if not os.path.isfile(log_path):
        print(f"エラー: ログがない: {log_path}", file=sys.stderr)
        return 3

    with open(log_path, encoding="utf-8") as f:
        lines = f.read().splitlines()

    # 再開したログは前の走行の判定行を含む。最後の走行だけを見る（ADR-0087）
    lines = last_run_lines(lines)
    src, verdict = find_source_line(lines)
    if src is None:
        print(f"エラー: 結果行が見つからない: {log_path}", file=sys.stderr)
        return 3

    try:
        fields = parse_fields(src)
    except ValueError as e:
        print(f"エラー: {e}", file=sys.stderr)
        return 3

    print(build_report(feature, verdict, fields, hypothesis_note(find_hypothesis(lines))))
    return EXIT_BY_VERDICT[verdict]


if __name__ == "__main__":
    sys.exit(main())
