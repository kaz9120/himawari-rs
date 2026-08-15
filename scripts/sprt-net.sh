#!/usr/bin/env bash
# 評価関数だけを差し替えてSPRTを回す（ADR-0149）。
#
# scripts/sprt.sh はバイナリ2つを比べる形しか持たない。ネットの比較では
# ビルドが同じで EvalFile だけが違うので、--bopt / --copt で片側ずつ指定する。
# ADR-0136・0138でこの形を何度も手打ちしていた。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./env.sh
source "${SCRIPT_DIR}/env.sh"

usage() {
	cat <<'USAGE'
使い方:
  scripts/sprt-net.sh <baselineネット> <candidateネット> <名前> [追加引数...]

例:
  scripts/sprt-net.sh data/nets/halfkp_2990M_ftclip.hmwr.best \
                      data/nets/halfkp_2990M_q1.hmwr.best adr0136-net

両者とも同じビルドで対局する。既定は data/bin/base-<名前> があればそれ、
なければ target/release/himawari を使う。SPRT_BIN で明示もできる。

**ネットとビルドの次元は揃える。** 既定のビルドはFT1024なので、上の例の
ようなFT256のネットを渡すなら SPRT_BIN でFT256のビルドを指す（ADR-0159）。

条件は env.sh の既定（ADR-0028）を使う。変えるときは環境変数で:
  SPRT_MAX_PAIRS=6000 scripts/sprt-net.sh base cand name   # 上限を上げる
  SPRT_ELO0=-5 SPRT_ELO1=0 scripts/sprt-net.sh base cand name  # 非劣性

棋譜は data/sprt/<名前>.jsonl、ログは data/logs/<名前>.log へ書く。
棋譜が既にあれば --resume で続きから測る。上限に届かず打ち切った測定を
延長するときは、SPRT_MAX_PAIRS を上げて同じコマンドを叩けばよい。

終了コード: 0=H1採択、1=H0採択、2=判定に至らず、3=実行エラー。
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
	usage
	exit 0
fi

if [[ $# -lt 3 ]]; then
	usage
	exit 2
fi

BASE_NET="$1"
CAND_NET="$2"
NAME="$3"
shift 3

cd "$REPO_ROOT"
require_file "$BASE_NET" "baselineのネット"
require_file "$CAND_NET" "candidateのネット"

# 対局に使うビルド。実験ごとに固定したいので data/bin を先に見る
BIN="${SPRT_BIN:-}"
if [[ -z "$BIN" ]]; then
	if [[ -x "data/bin/base-${NAME}" ]]; then
		BIN="data/bin/base-${NAME}"
	else
		BIN="target/release/himawari"
	fi
fi
require_executable "$BIN"

SELFPLAY="${REPO_ROOT}/target/release/selfplay"
require_executable "$SELFPLAY"
require_file "$OPENINGS" "開始局面"

JSONL="${REPO_ROOT}/data/sprt/${NAME}.jsonl"
mkdir -p "${REPO_ROOT}/data/sprt"

RESUME_ARGS=()
if [[ -s "$JSONL" ]]; then
	RESUME_ARGS+=(--resume "$JSONL")
	log_info "既存の棋譜から再開する（$(wc -l <"$JSONL" | tr -d ' ') 局）"
fi

log_step "SPRT（ネット比較）: $NAME"
env_summary
log_info "ビルド    : $BIN"
log_info "baseline  : $BASE_NET"
log_info "candidate : $CAND_NET"

# EvalFile は --option ではなく --bopt/--copt で渡す。--option と併用すると
# どちらが効くかが実装依存になるため、片側指定だけで完結させる
run_logged "$NAME" "$SELFPLAY" \
	--baseline "$BIN" \
	--candidate "$BIN" \
	--openings "$OPENINGS" \
	--tc "$SPRT_TC" \
	--concurrency "$SPRT_CONCURRENCY" \
	--adjudicate "$SPRT_ADJUDICATE" \
	--elo0 "$SPRT_ELO0" \
	--elo1 "$SPRT_ELO1" \
	--alpha "$SPRT_ALPHA" \
	--beta "$SPRT_BETA" \
	--max-pairs "$SPRT_MAX_PAIRS" \
	--bopt "EvalFile=${REPO_ROOT}/${BASE_NET#"${REPO_ROOT}/"}" \
	--copt "EvalFile=${REPO_ROOT}/${CAND_NET#"${REPO_ROOT}/"}" \
	--out "$JSONL" \
	${RESUME_ARGS[@]+"${RESUME_ARGS[@]}"} \
	"$@"
