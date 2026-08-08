#!/usr/bin/env bash
# 教師局面をqsearchの静止局面へ置き換える（ADR-0136・0149）。
#
# 評価関数が探索中に見るのは静止局面だが、hao_depth9は取り合いの途中の
# 局面へ収束後の探索値を付けて配られている。このずれを消す前処理である。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./env.sh
source "${SCRIPT_DIR}/env.sh"

usage() {
	cat <<'USAGE'
使い方:
  scripts/quiet.sh <入力psv> <出力psv> [名前]

例:
  scripts/quiet.sh data/train/train_2990M.psv data/train/train_2990M_q1.psv

名前を省くと出力ファイル名から作る。ログは data/logs/<名前>.log へ書く。

既定は --max-plies 1 で、評価関数は env.sh の EVAL_FILE を使う。1手に
切るのはADR-0136の結論で、一律に葉まで進める条件と棋力で差がつかず
（+4.7 Elo [-4.0, +13.5]、6000局、判定に至らず）、教師データの改変が
小さいほうを採ったためである。

  QUIET_MAX_PLIES=16 scripts/quiet.sh in.psv out.psv   # 一律に葉まで
  QUIET_LIMIT=1000000 scripts/quiet.sh in.psv out.psv  # 先頭だけ試す

**学習データを静止化したら、検証集合も同じ設定で静止化する。** 土俵が
ずれるとbest checkpointの選択が歪む（ADR-0136）。

29.9億で7.0時間、3億で50分かかる。停止ファイルは持たないので、途中で
止めたら最初からやり直す。
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
	usage
	exit 0
fi

if [[ $# -lt 2 ]]; then
	usage
	exit 2
fi

IN="$1"
OUT="$2"
NAME="${3:-}"

cd "$REPO_ROOT"
require_file "$IN" "入力のpsv"
require_file "$EVAL_FILE" "評価関数"

PSV="${REPO_ROOT}/target/release/psv"
require_executable "$PSV"

if [[ -z "$NAME" ]]; then
	NAME="quiet-$(basename "$OUT" .psv)"
fi

MAX_PLIES="${QUIET_MAX_PLIES:-1}"
LIMIT_ARGS=()
if [[ -n "${QUIET_LIMIT:-}" ]]; then
	LIMIT_ARGS+=(--limit "$QUIET_LIMIT")
fi

log_step "教師局面の静止化: $NAME"
log_info "入力      : $IN"
log_info "出力      : $OUT"
log_info "上限手数  : $MAX_PLIES"
log_info "評価関数  : $EVAL_FILE"

mkdir -p "$(dirname "$OUT")"
run_logged "$NAME" "$PSV" quiet \
	--in "$IN" \
	--out "$OUT" \
	--max-plies "$MAX_PLIES" \
	--eval-file "$EVAL_FILE" \
	${LIMIT_ARGS[@]+"${LIMIT_ARGS[@]}"}
