#!/usr/bin/env bash
# ネットワーク構成ごとにエンジンと評価ファイルを用意する（ADR-0127）。
#
# 構成を変えるとバイナリと評価ファイルの次元が同時に変わる。片方だけ
# 作り直すと読み込みエラーになるので、対で作るところまでを1本にする。
# 測定そのものは bench が行う（ADR-0122の言語の境界）。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./env.sh
source "${SCRIPT_DIR}/env.sh"

usage() {
	cat <<'USAGE'
使い方:
  scripts/build-shapes.sh [--from <元ネット> [--tag <名前>]] <構成> [構成...]

構成は <FT>x<L1>[x<L2>[x<L3>]]（例 256x16、512x16x32）。
要素の数が層の数を決める。256x16 なら隠れ層1つ。

構成ごとにエンジン data/bin/shape-<構成> と、評価ファイル
data/nets/<名前>-<構成>.hmwr を作る。既定の名前は shape。

--from を付けると、その評価ファイルを各構成の次元へ合わせる。広げる向きは
足したぶんの重みがゼロなので**評価値が元と完全に一致し、構成を変えても
探索木が変わらない**。速度の差だけを取り出せる。切り詰める向きは評価値が
変わるので、比べる構成すべてを同じ元から作ること。

--from を付けないと構成ごとに乱数ネットを作る。評価値が構成ごとに違ううえ
活性が飽和するため、NPSを構成間で比べられない。
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
	usage
	exit 0
fi

FROM=""
TAG="shape"
while [[ $# -gt 0 ]]; do
	case "$1" in
	--from)
		FROM="${2:-}"
		[[ -n "$FROM" ]] || die "--from に元ネットのパスが要る"
		shift 2
		;;
	--tag)
		TAG="${2:-}"
		[[ -n "$TAG" ]] || die "--tag に名前が要る"
		shift 2
		;;
	-*)
		die "不明なオプション: $1"
		;;
	*)
		break
		;;
	esac
done

if [[ $# -lt 1 ]]; then
	usage
	exit 2
fi

cd "$REPO_ROOT"
mkdir -p data/bin data/nets

if [[ -n "$FROM" ]]; then
	[[ -f "$FROM" ]] || die "元ネットがない: ${FROM}"
	[[ "$TAG" != "shape" ]] || TAG="exp"
fi

log_step "ネットワーク構成のビルド（${#@}件、名前 ${TAG}）"
log_info "RUSTFLAGS: ${RUSTFLAGS_NATIVE}"
[[ -n "$FROM" ]] && log_info "元ネット: ${FROM}（各構成へ広げる）"

for spec in "$@"; do
	if [[ ! "$spec" =~ ^[0-9]+x[0-9]+(x[0-9]+){0,2}$ ]]; then
		die "構成の書き方が違う: ${spec}（<FT>x<L1>[x<L2>[x<L3>]]）"
	fi
	bin="data/bin/shape-${spec}"
	net="data/nets/${TAG}-${spec}.hmwr"
	# 構成ごとにtargetを分ける。1つのtargetを使い回すと、構成を変える
	# たびに全部が再コンパイルされる
	target="target/shape/${spec}"

	log_info "ビルド: ${spec}"
	HIMAWARI_ARCH="$spec" CARGO_TARGET_DIR="$target" \
		RUSTFLAGS="$RUSTFLAGS_NATIVE" \
		cargo build --release --quiet -p himawari-usi -p himawari-tools --bin himawari --bin makenet
	cp "${target}/release/himawari" "$bin"

	log_info "評価ファイル: ${net}"
	if [[ -n "$FROM" ]]; then
		"${target}/release/makenet" --resize "$FROM" --out "$net" >/dev/null
	else
		"${target}/release/makenet" --seed 1 --out "$net" >/dev/null
	fi
done

log_step "完了"
log_info "NPSを測る:"
cmd="cargo run --release -p himawari-tools --bin bench -- --runs 5"
for spec in "$@"; do
	cmd+=" data/bin/shape-${spec}=data/nets/${TAG}-${spec}.hmwr"
done
echo "  ${cmd}"
