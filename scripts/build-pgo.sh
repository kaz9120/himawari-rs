#!/usr/bin/env bash
# PGO（プロファイル起点のビルド最適化）でエンジンを作る（ADR-0151群I）。
#
# 手順は3段になる。計測用ビルド→ベンチ局面の走行→最適化ビルド。
# 手で繰り返すと条件がぶれるので、ここに固定する（build-pair.shと同じ趣旨）。
# コードの意味は変わらないため、ノード数も評価値も変わらない（機能検証で
# 全局面一致を確認済み。ADR-0151）。
#
# SPRTのペアには使わない。ペアは両側を同条件（PGOなし）で作れば公平で、
# build-pair.shの手順が既定のままになる。PGOは対局・リリース用の単体
# ビルドに使う。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./env.sh
source "${SCRIPT_DIR}/env.sh"

usage() {
	cat <<'USAGE'
使い方:
  scripts/build-pgo.sh [出力先]

例:
  scripts/build-pgo.sh                     # data/bin/himawari-pgo へ出す
  scripts/build-pgo.sh data/bin/himawari-floodgate

現在のHEADから作る。学習走行はベンチ4局面（既定は深さ22、
PGO_DEPTH で変更）で、評価関数は env.sh の EVAL_FILE を使う。
llvm-profdataが要る（rustup component add llvm-tools）。
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
	usage
	exit 0
fi
if [[ $# -gt 1 ]]; then
	usage
	exit 2
fi

OUT="${1:-${REPO_ROOT}/data/bin/himawari-pgo}"
PGO_DEPTH="${PGO_DEPTH:-22}"
PGO_DIR="${REPO_ROOT}/target/pgo"

cd "$REPO_ROOT"

# 未コミットの変更があると、どのコードから作ったバイナリか後から辿れない
if ! git diff --quiet -- crates/ || ! git diff --cached --quiet -- crates/; then
	git status --short -- crates/ >&2
	die "crates/ に未コミットの変更がある。コミットしてから実行する"
fi

PROFDATA="$(find "$(rustc --print sysroot)/lib/rustlib" -name llvm-profdata -type f | head -1)"
if [[ -z "$PROFDATA" ]]; then
	die "llvm-profdataがない。rustup component add llvm-tools で入れる"
fi

rm -rf "$PGO_DIR"
mkdir -p "$PGO_DIR/raw" "$(dirname "$OUT")"

log_step "PGOビルド: $(git rev-parse --short HEAD) → ${OUT}"

log_info "1/3: 計測用ビルド（profile-generate）..."
# ビルドスクリプトも計測付きで走るため、静的カウンタ不足の警告が出るが無害
RUSTFLAGS="${RUSTFLAGS_NATIVE} -C profile-generate=${PGO_DIR}/raw" \
	cargo build --release --quiet

log_info "2/3: 学習走行（ベンチ4局面、深さ${PGO_DEPTH}）..."
cp target/release/himawari "$PGO_DIR/himawari-instr"
target/release/bench "$PGO_DIR/himawari-instr" \
	--depth "$PGO_DEPTH" --runs 1 >/dev/null
"$PROFDATA" merge -o "$PGO_DIR/merged.profdata" "$PGO_DIR"/raw/*.profraw

log_info "3/3: 最適化ビルド（profile-use）..."
RUSTFLAGS="${RUSTFLAGS_NATIVE} -C profile-use=${PGO_DIR}/merged.profdata" \
	cargo build --release --quiet
cp target/release/himawari "$OUT"

log_step "完了"
ls -la "$OUT"
log_info ""
log_info "targetにはPGO版が残っている。素のビルドに戻すには:"
echo "  RUSTFLAGS=\"\$RUSTFLAGS_NATIVE\" cargo build --release"
