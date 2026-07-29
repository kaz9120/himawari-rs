#!/usr/bin/env bash
# SPRTのログから結果を抜き、そのまま貼れる形で出す（ADR-0081）。
#
# 毎回ログを目で読んでコミットトレーラとRESULTS.mdの行を書き写していた。
# 数字の転記ミスは後から気づけない。ここで機械的に作る。
set -uo pipefail

usage() {
	cat <<'USAGE'
使い方:
  scripts/sprt-summary.sh <SPRTのログファイル> [機能名]

判定に達していれば結論行から、達していなければ最終のpairs行から作る。
出力は3つ。

  1. コミットの SPRT: トレーラ（ADR-0071の書式）
  2. RESULTS.md へ貼る表の行
  3. PR本文へ貼る表

終了コード: 0=H1、1=H0、2=判定に至らず、3=読めない。
USAGE
}

if [[ $# -lt 1 || $# -gt 2 ]]; then
	usage
	exit 3
fi

LOG="$1"
FEATURE="${2:-$(basename "$LOG" .log)}"

if [[ ! -f "$LOG" ]]; then
	echo "エラー: ログがない: $LOG" >&2
	exit 3
fi

# 判定行の例:
#   H1採択（候補は有意に強い） | pairs 525 games 1050 | +602 =46 -402 | Elo +67.0 [+46.4,+88.0] | LLR +3.05
VERDICT_LINE="$(grep -E '^(H1採択|H0採択|判定に至らず)' "$LOG" | tail -1)"
# 途中経過の例:
#   pairs   525 | +602 =46 -402 | [73,22,236,20,174] | Elo +67.0 [+46.4,+88.0] | LLR +3.05 [-2.94,2.94]
LAST_LINE="$(grep -E '^pairs ' "$LOG" | tail -1)"

if [[ -n "$VERDICT_LINE" ]]; then
	SRC="$VERDICT_LINE"
	case "$VERDICT_LINE" in
	H1*) VERDICT="H1" ;;
	H0*) VERDICT="H0" ;;
	*) VERDICT="打ち切り" ;;
	esac
elif [[ -n "$LAST_LINE" ]]; then
	SRC="$LAST_LINE"
	VERDICT="打ち切り"
else
	echo "エラー: 結果行が見つからない: $LOG" >&2
	exit 3
fi

parse() {
	awk -v src="$SRC" '
	BEGIN {
		# Elo +67.0 [+46.4,+88.0]
		if (match(src, /Elo [+-][0-9.]+ \[[+-][0-9.]+,[+-][0-9.]+\]/)) {
			elo = substr(src, RSTART + 4, RLENGTH - 4)
		}
		# LLR +3.05
		if (match(src, /LLR [+-][0-9.]+/)) { llr = substr(src, RSTART + 4, RLENGTH - 4) }
		# +602 =46 -402
		if (match(src, /\+[0-9]+ =[0-9]+ -[0-9]+/)) { wdl = substr(src, RSTART, RLENGTH) }
		# games 1050 があればそれ、なければ pairs から2倍する
		if (match(src, /games [0-9]+/)) { games = substr(src, RSTART + 6, RLENGTH - 6) }
		else if (match(src, /pairs +[0-9]+/)) {
			p = substr(src, RSTART + 6, RLENGTH - 6); gsub(/ /, "", p); games = p * 2
		}
		printf "%s\t%s\t%s\t%s\n", elo, llr, wdl, games
	}'
}

IFS=$'\t' read -r ELO LLR WDL GAMES < <(parse)
# "+67.0 [+46.4,+88.0]" を数値とCIへ分ける
ELO_NUM="${ELO%% *}"
ELO_CI="${ELO#* }"

echo "=== ${FEATURE}（${VERDICT}） ==="
echo
echo "--- コミットのトレーラ（ADR-0071） ---"
echo "SPRT: ${ELO_NUM} ${ELO_CI} ${GAMES}games ${VERDICT}"
echo
echo "--- RESULTS.md の表 ---"
echo "| 比較 | 結果 |"
echo "|---|---|"
if [[ "$VERDICT" == "打ち切り" ]]; then
	echo "| ${FEATURE} | **${ELO_NUM} ${ELO_CI}**（${GAMES}局、LLR ${LLR}で打ち切り） |"
else
	echo "| ${FEATURE} | **${ELO_NUM} ${ELO_CI}**（${GAMES}局、LLR ${LLR}で${VERDICT}採択） |"
fi
echo
echo "--- PR本文の表 ---"
echo "| 項目 | 値 |"
echo "|---|---|"
echo "| 対局数 | ${GAMES}（$((GAMES / 2))ペア） |"
echo "| W-D-L | ${WDL} |"
echo "| Elo [95%CI] | **${ELO_NUM} ${ELO_CI}** |"
echo "| LLR | ${LLR} |"
echo "| 判定 | **${VERDICT}** |"

case "$VERDICT" in
H1) exit 0 ;;
H0) exit 1 ;;
*) exit 2 ;;
esac
