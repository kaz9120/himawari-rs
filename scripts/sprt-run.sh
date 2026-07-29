#!/usr/bin/env bash
# SPRTを判定が出るまで回し続ける（ADR-0087）。
#
# 長時間のSPRTは外部要因で止まる。実際に1つのSPRTで3回止まった
# （2026-07-29）。そのたびに手で --resume するのは自動化として不完全
# なので、ここで繰り返す。
#
# 効く範囲と効かない範囲がある。
#   効く : selfplayプロセスだけが落ちた場合（メモリ不足、エンジンの異常
#          終了など）。このループが --resume で拾い直す
#   効かない: このスクリプト自体が止められた場合（セッションの終了、
#          マシンの再起動）。ループごと消える
#
# 後者でも棋譜は残る。次にこのスクリプトを実行すれば、既存の棋譜から
# 続きを回す（ADR-0087）。「止まらない」ではなく「止まっても失わない」
# のが本質である。
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./env.sh
source "${SCRIPT_DIR}/env.sh"

usage() {
	cat <<'USAGE'
使い方:
  scripts/sprt-run.sh <baselineバイナリ> <candidateバイナリ> <名前> [再試行の上限]

scripts/sprt.sh を呼び、判定が出る前に落ちたら --resume で再開する。
既存の棋譜があれば最初から --resume で始める。

再試行の上限は既定20回。判定が出るか上限に達するまで繰り返す。
終了コードは sprt.sh のもの（0=H1採択、1=H0採択、2=判定に至らず、3=実行エラー）。
USAGE
}

if [[ $# -lt 3 || $# -gt 4 ]]; then
	usage
	exit 3
fi

BASELINE="$1"
CANDIDATE="$2"
NAME="$3"
MAX_RETRY="${4:-20}"
JSONL="${REPO_ROOT}/data/sprt/${NAME}.jsonl"

for ((attempt = 1; attempt <= MAX_RETRY; attempt++)); do
	ARGS=()
	if [[ -s "$JSONL" ]]; then
		ARGS+=(--resume "$JSONL")
		echo "=== 試行 ${attempt}: 既存の棋譜から再開する（$(wc -l <"$JSONL") 局） ==="
	else
		echo "=== 試行 ${attempt}: 新規に開始する ==="
	fi

	"${SCRIPT_DIR}/sprt.sh" "$BASELINE" "$CANDIDATE" "$NAME" ${ARGS[@]+"${ARGS[@]}"}
	CODE=$?

	case $CODE in
	0 | 1 | 2)
		# 判定が出た（H1・H0）か、上限まで回して判定に至らなかった
		echo "=== SPRT終了: 終了コード ${CODE} ==="
		exit $CODE
		;;
	*)
		echo "=== 試行 ${attempt} が異常終了（コード ${CODE}）。再開する ===" >&2
		sleep 5
		;;
	esac
done

echo "エラー: ${MAX_RETRY}回試しても判定に至らなかった" >&2
exit 3
