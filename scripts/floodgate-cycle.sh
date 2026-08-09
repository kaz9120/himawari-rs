#!/usr/bin/env bash
# floodgateの棋譜サイクルを1本で回す（ADR-0152）。
#
# 回収→分析レポート→定跡追加→網羅率の4段。各段の道具は独立に使えるが、
# 定期実行の手順をここに固定する。分析と定跡追加は入力集合・エンジン・
# 評価関数・探索条件の純関数で、定跡追加は冪等（ADR-0152の決定論）。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./env.sh
source "${SCRIPT_DIR}/env.sh"

usage() {
	cat <<'USAGE'
使い方:
  scripts/floodgate-cycle.sh [年]

例:
  scripts/floodgate-cycle.sh          # 既定は2026
  SEED_MAX=100 scripts/floodgate-cycle.sh

回収（floodgate-fetch.py）→分析（kifu）→定跡追加（book seed）→
book stats の順で回す。ログは data/logs/floodgate-cycle.log へ追記する。

定跡追加は1局面あたり深さ28で約34秒かかる（ADR-0146）。1回の追加数は
SEED_MAX（既定50、約30分）で絞り、残りは次回のサイクルが続きから足す
（book seedは冪等なので、何度回しても取得済みの局面は増えない）。
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
	usage
	exit 0
fi

YEAR="${1:-2026}"
SEED_MAX="${SEED_MAX:-50}"
GAMES_DIR="${REPO_ROOT}/data/raw/floodgate/${YEAR}"
BOOK_FILE="${REPO_ROOT}/data/book/main.db"
REPORT="${REPO_ROOT}/data/logs/floodgate-report-$(date '+%Y%m%d').md"

cd "$REPO_ROOT"

log_step "floodgateサイクル: ${YEAR}年（エンジン $(git rev-parse --short HEAD)）"

log_info "1/4: 回収..."
run_logged floodgate-cycle python3 scripts/floodgate-fetch.py

if [[ ! -d "$GAMES_DIR" ]]; then
	die "棋譜がない: ${GAMES_DIR}"
fi

log_info "2/4: 分析レポート → ${REPORT}"
RUSTFLAGS="$RUSTFLAGS_NATIVE" cargo build --release --quiet
run_logged floodgate-cycle target/release/kifu target/release/himawari \
	"$GAMES_DIR" --eval-file "$EVAL_FILE" --out "$REPORT"

log_info "3/4: 定跡追加（最大${SEED_MAX}局面。冪等なので続きから足す）..."
# --depth 28 は定跡の規格（ADR-0146のbook-v3）。seedの既定はgenと共通の
# 24なので、ここで明示しないと浅い探索の局面が混ざる（初回運用で実際に
# 起き、book-v3からの復元でやり直した）
run_logged floodgate-cycle target/release/book seed \
	--games "$GAMES_DIR" --out "$BOOK_FILE" \
	--eval "$EVAL_FILE" --depth 28 --max-positions "$SEED_MAX"

log_info "4/4: 網羅率..."
run_logged floodgate-cycle target/release/book stats --out "$BOOK_FILE"

log_step "完了"
log_info "レポート: ${REPORT}"
