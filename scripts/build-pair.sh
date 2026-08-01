#!/usr/bin/env bash
# SPRT用のbaseline/candidateバイナリを作る（ADR-0081）。
#
# 手で作ると条件がぶれる。実際に起きた事故が3つある。
#   1. candidateだけ RUSTFLAGS=-C target-cpu=native を付け忘れ、
#      最適化条件の違う2本を比べた
#   2. git stash の pop 忘れ・競合で、変更前後を取り違えた
#   3. rebaseの競合を解決しないままビルドし、中途半端な木から作った
# ここに固めて、同じ手順を繰り返せるようにする。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./env.sh
source "${SCRIPT_DIR}/env.sh"

usage() {
	cat <<'USAGE'
使い方:
  scripts/build-pair.sh <名前> [baselineのref]

例:
  scripts/build-pair.sh adr0097              # baselineは origin/main
  scripts/build-pair.sh adr0097 v0.12.0      # baselineをタグで指定

data/bin/base-<名前> と data/bin/cand-<名前> を作る。
candidateは現在のHEAD、baselineは指定ref（既定 origin/main）。

未コミットの変更があると中断する。SPRTにかけるのはコミット済みの
状態だけにするため（ADR-0070のブランチ運用）。
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
	usage
	exit 0
fi

if [[ $# -lt 1 || $# -gt 2 ]]; then
	usage
	exit 2
fi

NAME="$1"
BASE_REF="${2:-origin/main}"
BIN_DIR="${REPO_ROOT}/data/bin"
BASE_OUT="${BIN_DIR}/base-${NAME}"
CAND_OUT="${BIN_DIR}/cand-${NAME}"

cd "$REPO_ROOT"

# 未コミットの変更があると、どちらをcandidateにしたか後から辿れない
if ! git diff --quiet -- crates/ || ! git diff --cached --quiet -- crates/; then
	git status --short -- crates/ >&2
	die "crates/ に未コミットの変更がある。コミットしてから実行する"
fi

if ! git rev-parse --verify --quiet "$BASE_REF" >/dev/null; then
	die "baselineのrefが見つからない: $BASE_REF"
fi

mkdir -p "$BIN_DIR"

build() {
	local out="$1"
	RUSTFLAGS="$RUSTFLAGS_NATIVE" cargo build --release --quiet
	cp "${REPO_ROOT}/target/release/himawari" "$out"
}

log_step "SPRT用バイナリの作成: ${NAME}"
log_info "candidate: $(git rev-parse --short HEAD) （現在のHEAD）"
log_info "baseline : $(git rev-parse --short "$BASE_REF") （${BASE_REF}）"
log_info "RUSTFLAGS: ${RUSTFLAGS_NATIVE}"

log_info "candidateをビルド中..."
build "$CAND_OUT"

log_info "baselineをビルド中..."
# crates/ だけ差し替える。worktreeを切るとビルドキャッシュが別になり遅い
git checkout "$BASE_REF" -- crates/
trap 'git checkout HEAD -- crates/' EXIT
build "$BASE_OUT"
git checkout HEAD -- crates/
trap - EXIT

if cmp -s "$BASE_OUT" "$CAND_OUT"; then
	log_warn "2本が同一のバイナリになった。探索に差がない可能性がある"
	log_warn "  機能検証で確かめること"
	exit 1
fi
log_step "完了"
ls -la "$BASE_OUT" "$CAND_OUT"
log_info ""
log_info "次の手順:"
echo "  cargo run --release -p himawari-tools --bin verify -- ${BASE_OUT#"$REPO_ROOT"/} ${CAND_OUT#"$REPO_ROOT"/}"
echo "  scripts/sprt.sh ${BASE_OUT#"$REPO_ROOT"/} ${CAND_OUT#"$REPO_ROOT"/} ${NAME}"
