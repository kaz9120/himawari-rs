#!/usr/bin/env bash
# SPRTを既定条件で回す（ADR-0028）。
#
# 並列度と評価関数は env.sh がマシンに合わせて決める。
# 毎回長いコマンドを打たずに済み、条件の打ち間違いも防げる。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./env.sh
source "${SCRIPT_DIR}/env.sh"

usage() {
	cat <<'USAGE'
使い方:
  scripts/sprt.sh <baselineバイナリ> <candidateバイナリ> <名前> [追加引数...]

例:
  scripts/sprt.sh data/bin/base-adr0079 data/bin/cand-adr0079 adr0079-cutnode

棋譜は data/sprt/<名前>.jsonl へ書く。
終了コード: 0=H1採択、1=H0採択、2=判定に至らず、3=実行エラー。

条件は env.sh の既定（ADR-0028）を使う。変えるときは環境変数で:
  SPRT_TC=60+0.6 scripts/sprt.sh base cand name
  SPRT_ELO0=-5 SPRT_ELO1=0 scripts/sprt.sh base cand name   # 非劣性

SPRTの前に機能検証を済ませること（ADR-0074）。固定深さでノード数が
変わらない変更は、ここへ持ち込んでも中立にしかならない。
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
	usage
	exit 0
fi

# 引数エラーはADR-0122の規約で2。exec後にselfplayが返す2（判定に至らず）
# とは意味が違うが、両者が発生する経路は排他的なので取り違えない
if [[ $# -lt 3 ]]; then
	usage
	exit 2
fi

BASELINE="$1"
CANDIDATE="$2"
NAME="$3"
shift 3

for f in "$BASELINE" "$CANDIDATE"; do
	require_executable "$f"
done

if [[ ! -f "$EVAL_FILE" ]]; then
	log_error "評価関数がない: $EVAL_FILE"
	log_error "  gh release download net-v<N> -D data/nets/ で取得する"
	exit 3
fi

SELFPLAY="${REPO_ROOT}/target/release/selfplay"
if [[ ! -x "$SELFPLAY" ]]; then
	die "${SELFPLAY} がない。cargo build --release を実行する"
fi

OUT_DIR="${REPO_ROOT}/data/sprt"
mkdir -p "$OUT_DIR"

log_step "SPRT: $NAME"
env_summary
log_info "baseline : $BASELINE"
log_info "candidate: $CANDIDATE"

exec "$SELFPLAY" \
	--baseline "$BASELINE" \
	--candidate "$CANDIDATE" \
	--openings "$OPENINGS" \
	--tc "$SPRT_TC" \
	--concurrency "$SPRT_CONCURRENCY" \
	--adjudicate "$SPRT_ADJUDICATE" \
	--elo0 "$SPRT_ELO0" \
	--elo1 "$SPRT_ELO1" \
	--alpha "$SPRT_ALPHA" \
	--beta "$SPRT_BETA" \
	--max-pairs "$SPRT_MAX_PAIRS" \
	--option "EvalFile=${EVAL_FILE}" \
	--out "${OUT_DIR}/${NAME}.jsonl" \
	"$@"
