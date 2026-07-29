#!/usr/bin/env bash
# 固定深さでNPSを測る（ADR-0081）。
#
# 速度改善の効果は固定深さのノード数では見えない（ADR-0074の機能検証は
# 「変わらない」を確かめるもの）。同じノード数を何秒で読むかを測る。
#
# 局面と深さは verify-feature.sh と揃える。局面3だけ枝が広く、同じ深さ
# では時間を独占するため深さを3浅くする。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./env.sh
source "${SCRIPT_DIR}/env.sh"

usage() {
	cat <<'USAGE'
使い方:
  scripts/bench-nps.sh <バイナリ> [<バイナリ>...]

例:
  scripts/bench-nps.sh data/bin/base-adr0101 data/bin/cand-adr0101

複数指定すると交互に測る。機体の温度や背景の負荷でNPSは数%動くため、
続けて測って比べる。1本ずつ別々に測った値を比べない。

深さは BENCH_DEPTH（既定19）、繰り返しは BENCH_RUNS（既定3）で変える。
出力はADRへ転記できるmarkdown表。
USAGE
}

if [[ $# -lt 1 ]]; then
	usage
	exit 2
fi

DEPTH="${BENCH_DEPTH:-19}"
RUNS="${BENCH_RUNS:-3}"
TIMEOUT_SEC="${BENCH_TIMEOUT:-600}"

for f in "$@"; do
	if [[ ! -x "$f" ]]; then
		echo "エラー: 実行できない: $f" >&2
		exit 3
	fi
done
if [[ ! -f "$EVAL_FILE" ]]; then
	echo "エラー: 評価関数がない: $EVAL_FILE" >&2
	exit 3
fi

# verify-feature.sh と同じ4局面。増やすときは末尾へ足す
SFENS=(
	"startpos"
	"sfen +Bn1g2s1l/2skg2r1/ppppp1n1p/5bpp1/5p1P1/2P6/PP1PP1P1P/1SK2S1R1/LN1G1G1NL w Lp 24"
	"sfen +R1G4nl/1g4+Ss1/1kspp2p1/ppp2pS1p/4n4/P4Gp1P/1P1PP1P2/1+n2K2R1/7NL w G2P2b2lp 24"
	"sfen 1n1gk2nl/1Bsr3s1/lp2ppgpp/p1pp2p2/7P1/P1P6/1PNPPPP1P/1SKG2SR1/L4G1NL w b 24"
)
# 局面3は同じ深さだと時間を独占する。深さで揃える
DEPTH_ADJ=(0 0 -3 0)

# 1本を1周ぶん測り、"合計ノード数<TAB>合計ミリ秒" を返す
run_once() {
	local engine="$1"
	local dir out fifo pid n
	dir="$(mktemp -d)"
	out="${dir}/out"
	fifo="${dir}/in"
	mkfifo "$fifo"
	: >"$out"

	"$engine" <"$fifo" >"$out" 2>/dev/null &
	pid=$!
	exec 3>"$fifo"
	printf 'usi\nsetoption name EvalFile value %s\nsetoption name Threads value 1\nisready\nusinewgame\n' \
		"$EVAL_FILE" >&3

	n=0
	for pos in "${SFENS[@]}"; do
		printf 'position %s\ngo depth %s\n' "$pos" "$((DEPTH + DEPTH_ADJ[n]))" >&3
		n=$((n + 1))
		local waited=0 got
		while true; do
			got="$(grep -c '^bestmove' "$out" 2>/dev/null || true)"
			[[ "${got:-0}" -ge "$n" ]] && break
			if ! kill -0 "$pid" 2>/dev/null; then
				echo "エラー: エンジンが終了した: $engine" >&2
				exec 3>&-
				rm -rf "$dir"
				exit 3
			fi
			if ((waited >= TIMEOUT_SEC * 10)); then
				echo "エラー: ${TIMEOUT_SEC}秒で読み終わらなかった: $engine" >&2
				kill "$pid" 2>/dev/null || true
				exec 3>&-
				rm -rf "$dir"
				exit 3
			fi
			sleep 0.1
			waited=$((waited + 1))
		done
	done
	printf 'quit\n' >&3 || true
	exec 3>&-
	wait "$pid" 2>/dev/null || true

	# 各局面の最終info行からnodesとtimeを拾って合計する
	awk '
		/^info depth [0-9]+ .* nodes / {
			for (i = 1; i < NF; i++) {
				if ($i == "nodes") nodes = $(i+1)
				if ($i == "time") t = $(i+1)
			}
		}
		/^bestmove/ { tn += nodes; tt += t }
		END { printf "%d\t%d\n", tn, tt }
	' "$out"
	rm -rf "$dir"
}

echo "=== NPS計測: 深さ ${DEPTH}（局面3は $((DEPTH - 3))）、${RUNS}周、1スレッド ==="
echo "評価関数: ${EVAL_FILE}"
echo

declare -a NAMES=()
declare -a SUMS=()
for f in "$@"; do
	NAMES+=("$f")
	SUMS+=(0)
done

# 交互に測る。1本ずつまとめて測ると温度差が系統誤差になる
for ((r = 1; r <= RUNS; r++)); do
	for i in "${!NAMES[@]}"; do
		IFS=$'\t' read -r nodes ms < <(run_once "${NAMES[$i]}")
		nps=$((nodes * 1000 / ms))
		printf "  %-28s run%d: %'d nps（%'d nodes / %'dms）\n" \
			"$(basename "${NAMES[$i]}")" "$r" "$nps" "$nodes" "$ms"
		SUMS[i]=$((SUMS[i] + nps))
	done
done

echo
echo "| バイナリ | NPS（${RUNS}周の平均） | 1本目比 |"
echo "|---|---|---|"
base=$((SUMS[0] / RUNS))
for i in "${!NAMES[@]}"; do
	avg=$((SUMS[i] / RUNS))
	ratio="$(awk -v a="$base" -v b="$avg" 'BEGIN { printf "%+.2f%%", (b - a) * 100.0 / a }')"
	printf "| %s | %'d | %s |\n" "$(basename "${NAMES[$i]}")" "$avg" \
		"$([[ $i -eq 0 ]] && echo "—" || echo "$ratio")"
done
