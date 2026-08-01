#!/usr/bin/env bash
# ネットワーク構成ごとにエンジンと乱数ネットを用意する（ADR-0127）。
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
  scripts/build-shapes.sh <構成> [構成...]

構成は <FT>x<L1>x<L2>（例 512x16x32）。既定は 256x32x32。

構成ごとに次を作る。
  data/bin/shape-<構成>          エンジン
  data/nets/shape-<構成>.hmwr    同じ構成の乱数ネット

最後にbenchへ渡すコマンドを表示する。乱数ネットなので比べられるのは
NPSだけで、ノード数と評価値には意味がない（ADR-0127）。
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
	usage
	exit 0
fi

if [[ $# -lt 1 ]]; then
	usage
	exit 2
fi

cd "$REPO_ROOT"
mkdir -p data/bin data/nets

log_step "ネットワーク構成のビルド（${#@}件）"
log_info "RUSTFLAGS: ${RUSTFLAGS_NATIVE}"

for spec in "$@"; do
	if [[ ! "$spec" =~ ^[0-9]+x[0-9]+x[0-9]+$ ]]; then
		die "構成の書き方が違う: ${spec}（<FT>x<L1>x<L2>）"
	fi
	bin="data/bin/shape-${spec}"
	net="data/nets/shape-${spec}.hmwr"
	# 構成ごとにtargetを分ける。1つのtargetを使い回すと、構成を変える
	# たびに全部が再コンパイルされる
	target="target/shape/${spec}"

	log_info "ビルド: ${spec}"
	HIMAWARI_ARCH="$spec" CARGO_TARGET_DIR="$target" \
		RUSTFLAGS="$RUSTFLAGS_NATIVE" \
		cargo build --release --quiet -p himawari-usi -p himawari-tools --bin himawari --bin makenet
	cp "${target}/release/himawari" "$bin"

	log_info "乱数ネット: ${net}"
	"${target}/release/makenet" --seed 1 --out "$net" >/dev/null
done

log_step "完了"
log_info "NPSを測る:"
cmd="cargo run --release -p himawari-tools --bin bench -- --runs 5"
for spec in "$@"; do
	cmd+=" data/bin/shape-${spec}=data/nets/shape-${spec}.hmwr"
done
echo "  ${cmd}"
