#!/usr/bin/env bash
# 機能検証（ADR-0074）。固定深さでのノード数を変更前後で比べる。
#
# 局面を毎回書き下すと条件がぶれるため、局面と深さをここで固定する。
# ADRへ転記できる形（markdown表）で出力する。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./env.sh
source "${SCRIPT_DIR}/env.sh"

usage() {
	cat <<'USAGE'
使い方:
  scripts/verify-feature.sh <baselineバイナリ> [candidateバイナリ]

例:
  scripts/verify-feature.sh data/bin/base-adr0084 data/bin/cand-adr0084
  scripts/verify-feature.sh target/release/himawari   # 1本だけ測る

深さは VERIFY_DEPTH（既定13）で変えられる。局面はこのスクリプトが持つ
4つ（初期局面と24手目の3局面）を使う。ADR-0074の「3局面以上」を満たす。

全局面でノード数が一致したら、その変更は探索に影響していない。
USAGE
}

if [[ $# -lt 1 || $# -gt 2 ]]; then
	usage
	exit 1
fi

BASELINE="$1"
CANDIDATE="${2:-}"
DEPTH="${VERIFY_DEPTH:-13}"
# エンジンが1局面を読み切るまでの上限。深さを上げるときは併せて延ばす
TIMEOUT_SEC="${VERIFY_TIMEOUT:-300}"

# 検証局面。初期局面と openings/start_sfens_ply24.txt の先頭3行。
# 固定して条件を揃える。増やすときは末尾へ足し、既存の並びは変えない
SFENS=(
	"startpos"
	"sfen +Bn1g2s1l/2skg2r1/ppppp1n1p/5bpp1/5p1P1/2P6/PP1PP1P1P/1SK2S1R1/LN1G1G1NL w Lp 24"
	"sfen +R1G4nl/1g4+Ss1/1kspp2p1/ppp2pS1p/4n4/P4Gp1P/1P1PP1P2/1+n2K2R1/7NL w G2P2b2lp 24"
	"sfen 1n1gk2nl/1Bsr3s1/lp2ppgpp/p1pp2p2/7P1/P1P6/1PNPPPP1P/1SKG2SR1/L4G1NL w b 24"
)

for f in "$BASELINE" ${CANDIDATE:+"$CANDIDATE"}; do
	if [[ ! -x "$f" ]]; then
		echo "エラー: 実行できない: $f" >&2
		exit 3
	fi
done
if [[ ! -f "$EVAL_FILE" ]]; then
	echo "エラー: 評価関数がない: $EVAL_FILE" >&2
	exit 3
fi

# 1局面を1エンジンで読み、"ノード数<TAB>評価値<TAB>最善手" を返す。
# go の直後に quit を送ると探索が切れるため、bestmove を待ってから送る
run_one() {
	local engine="$1" pos="$2"
	local dir out fifo pid
	dir="$(mktemp -d)"
	out="${dir}/out"
	fifo="${dir}/in"
	mkfifo "$fifo"
	: >"$out"

	"$engine" <"$fifo" >"$out" 2>/dev/null &
	pid=$!
	exec 3>"$fifo"
	printf 'usi\nsetoption name EvalFile value %s\nisready\nusinewgame\nposition %s\ngo depth %s\n' \
		"$EVAL_FILE" "$pos" "$DEPTH" >&3

	local waited=0
	while ! grep -q '^bestmove' "$out" 2>/dev/null; do
		if ! kill -0 "$pid" 2>/dev/null; then
			break
		fi
		if ((waited >= TIMEOUT_SEC * 10)); then
			echo "エラー: ${TIMEOUT_SEC}秒で読み終わらなかった: $pos" >&2
			kill "$pid" 2>/dev/null || true
			exec 3>&-
			rm -rf "$dir"
			exit 3
		fi
		sleep 0.1
		waited=$((waited + 1))
	done
	printf 'quit\n' >&3 || true
	exec 3>&-
	wait "$pid" 2>/dev/null || true

	# 最終深さのinfo行から nodes と score を拾う。MultiPVは1本目のみ
	local line best
	line="$(grep "^info depth ${DEPTH} " "$out" | head -1)"
	best="$(grep '^bestmove' "$out" | head -1 | awk '{print $2}')"
	rm -rf "$dir"
	if [[ -z "$line" ]]; then
		echo "エラー: 深さ${DEPTH}のinfo行がない: $pos" >&2
		exit 3
	fi
	awk -v best="$best" '{
		nodes = ""; score = ""
		for (i = 1; i < NF; i++) {
			if ($i == "nodes") nodes = $(i + 1)
			if ($i == "score") score = $(i + 1) " " $(i + 2)
		}
		printf "%s\t%s\t%s\n", nodes, score, best
	}' <<<"$line"
}

echo "=== 機能検証（ADR-0074）: 固定深さ ${DEPTH} ==="
echo "評価関数: ${EVAL_FILE}"
echo "baseline : ${BASELINE}"
[[ -n "$CANDIDATE" ]] && echo "candidate: ${CANDIDATE}"
echo

if [[ -z "$CANDIDATE" ]]; then
	echo "| 局面 | ノード数 | 評価値 | 最善手 |"
	echo "|---|---|---|---|"
	for i in "${!SFENS[@]}"; do
		IFS=$'\t' read -r n s b < <(run_one "$BASELINE" "${SFENS[$i]}")
		printf "| %d | %'d | %s | %s |\n" "$((i + 1))" "$n" "$s" "$b"
	done
	exit 0
fi

same=1
echo "| 局面 | 変更前 | 変更後 | 変化 | 評価値 | 最善手 |"
echo "|---|---|---|---|---|---|"
for i in "${!SFENS[@]}"; do
	IFS=$'\t' read -r bn bs bb < <(run_one "$BASELINE" "${SFENS[$i]}")
	IFS=$'\t' read -r cn cs cb < <(run_one "$CANDIDATE" "${SFENS[$i]}")
	pct="$(awk -v a="$bn" -v b="$cn" 'BEGIN { if (a == 0) print "n/a"; else printf "%+.0f%%", (b - a) * 100.0 / a }')"
	if [[ "$bn" != "$cn" ]]; then
		same=0
	fi
	if [[ "$bs" == "$cs" ]]; then
		score_cell="$bs"
	else
		score_cell="${bs} → ${cs}"
	fi
	if [[ "$bb" == "$cb" ]]; then
		move_cell="同じ"
	else
		move_cell="${bb} → ${cb}"
	fi
	printf "| %d | %'d | %'d | %s | %s | %s |\n" \
		"$((i + 1))" "$bn" "$cn" "$pct" "$score_cell" "$move_cell"
done

echo
if ((same == 1)); then
	echo "全局面でノード数が一致した。この変更は探索に影響していない（ADR-0074）。"
	echo "SPRTにかけても中立にしかならない。"
	exit 1
fi
echo "ノード数が変わった。探索に影響している。SPRTへ進んでよい。"
