#!/usr/bin/env bash
# 探索のプロファイルを取る（ADR-0081・ADR-0099）。
#
# samplyでサンプリングし、self時間の上位をソース行まで落として出す。
# 局面と深さは bench-nps.sh と揃える。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./env.sh
source "${SCRIPT_DIR}/env.sh"

usage() {
	cat <<'USAGE'
使い方:
  scripts/profile.sh [バイナリ] [出力先ディレクトリ]

既定のバイナリは target/release/himawari、出力先は data/profile/。

行番号まで出すにはデバッグ情報が要る。次のようにビルドする。

  CARGO_PROFILE_RELEASE_DEBUG=1 RUSTFLAGS="-C target-cpu=native" \
    cargo build --release

samplyが要る（cargo install samply）。深さは PROFILE_DEPTH（既定19）。

出力は3つ。
  1. self時間の上位（関数）
  2. self時間の上位（ソース行）
  3. プロファイル本体（samply load <path> でUIから見る）
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
	usage
	exit 0
fi

ENGINE="${1:-${REPO_ROOT}/target/release/himawari}"
OUT_DIR="${2:-${REPO_ROOT}/data/profile}"
DEPTH="${PROFILE_DEPTH:-19}"
SAMPLY="${SAMPLY:-$(command -v samply || echo "${HOME}/.cargo/bin/samply")}"

if [[ ! -x "$ENGINE" ]]; then
	echo "エラー: 実行できない: $ENGINE" >&2
	exit 3
fi
if [[ ! -x "$SAMPLY" ]]; then
	echo "エラー: samplyがない。cargo install samply で入れる" >&2
	exit 3
fi
if [[ ! -f "$EVAL_FILE" ]]; then
	echo "エラー: 評価関数がない: $EVAL_FILE" >&2
	exit 3
fi

mkdir -p "$OUT_DIR"
PROF="${OUT_DIR}/profile.json.gz"

SFENS=(
	"startpos"
	"sfen +Bn1g2s1l/2skg2r1/ppppp1n1p/5bpp1/5p1P1/2P6/PP1PP1P1P/1SK2S1R1/LN1G1G1NL w Lp 24"
	"sfen +R1G4nl/1g4+Ss1/1kspp2p1/ppp2pS1p/4n4/P4Gp1P/1P1PP1P2/1+n2K2R1/7NL w G2P2b2lp 24"
	"sfen 1n1gk2nl/1Bsr3s1/lp2ppgpp/p1pp2p2/7P1/P1P6/1PNPPPP1P/1SKG2SR1/L4G1NL w b 24"
)
DEPTH_ADJ=(0 0 -3 0)

echo "=== プロファイル: 深さ ${DEPTH}（局面3は $((DEPTH - 3))）、1スレッド ==="
echo "対象: ${ENGINE}"
echo

dir="$(mktemp -d)"
out="${dir}/out"
fifo="${dir}/in"
mkfifo "$fifo"
: >"$out"

"$SAMPLY" record --save-only --unstable-presymbolicate -r 2000 -o "$PROF" \
	"$ENGINE" <"$fifo" >"$out" 2>"${dir}/err" &
pid=$!
exec 3>"$fifo"
printf 'usi\nsetoption name EvalFile value %s\nsetoption name Threads value 1\nisready\nusinewgame\n' \
	"$EVAL_FILE" >&3

n=0
for pos in "${SFENS[@]}"; do
	printf 'position %s\ngo depth %s\n' "$pos" "$((DEPTH + DEPTH_ADJ[n]))" >&3
	n=$((n + 1))
	waited=0
	while true; do
		got="$(grep -c '^bestmove' "$out" 2>/dev/null || true)"
		[[ "${got:-0}" -ge "$n" ]] && break
		kill -0 "$pid" 2>/dev/null || break
		if ((waited >= 6000)); then
			echo "エラー: 読み終わらなかった" >&2
			kill "$pid" 2>/dev/null || true
			break
		fi
		sleep 0.1
		waited=$((waited + 1))
	done
done
printf 'quit\n' >&3 || true
exec 3>&-
wait "$pid" 2>/dev/null || true
rm -rf "$dir"

echo
python3 "${SCRIPT_DIR}/profile-report.py" "$PROF" "$ENGINE"
echo
echo "プロファイル本体: ${PROF}"
echo "UIで見る: ${SAMPLY} load ${PROF}"
