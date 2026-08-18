#!/usr/bin/env bash
# SPRTを判定が出るまで回し続ける（ADR-0087・0175）。
#
# 2つのことを自動化する。
#   1. 走行が落ちたら --resume で拾い直す（ADR-0087）
#   2. 上限に達しても判定が出ていなければ、そのまま走り続ける（ADR-0175）
#
# 2はsprt.shの --max-pairs を安全弁の値（SPRT_HARD_MAX_PAIRS）で1回通す
# ことで実現する。--max-pairs は通算のペア数なので、段階的に広げる必要は
# ない。上限は「収束の判定基準」ではなく「暴走を止める安全弁」である。
#
# 判定（H1・H0）が出たら data/sprt/<名前>.result へ結果を書く。
# **このファイルの有無がSPRTの完了を表す**（ADR-0175）。プロセスの生死や
# セッションの継続に依存しないので、いつ誰が見ても完了を判定できる。
# 既に .result があれば、走らせずにそれを返す（冪等）。
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
#
# -eを付けない。下のループでsprt.shの終了コードを $? で分岐取得する
# 必要があり、-eがあるとsprt.shの非0終了で即座にシェルごと終了して
# しまい、再試行の分岐に届かない
set -uo pipefail

# 判定が出るまで数時間〜数日かかることがあるため、ログに時刻を付ける
LOG_TIMESTAMP=1
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./env.sh
source "${SCRIPT_DIR}/env.sh"

usage() {
	cat <<'USAGE'
使い方:
  scripts/sprt-run.sh <baselineバイナリ> <candidateバイナリ> <名前> [再試行の上限] [追加引数...]

判定（H1・H0）が出るまで走らせる（ADR-0175）。落ちたら --resume で再開し、
既存の棋譜があれば最初から続きを回す。

判定が出たら data/sprt/<名前>.result へ結果を書く。このファイルがあれば
走らせずにそれを返すので、何度実行しても安全である。

上限は SPRT_HARD_MAX_PAIRS（既定60000ペア＝12万局）で、収束の判定基準では
なく暴走を止める安全弁である。ここに達しても判定が出ないなら、局数を積むより
対立仮説の立て方を見直す状況になる（ADR-0163）。

再試行の上限は既定20回。異常終了からの再開の回数を数える。
追加引数はsprt.shへ素通しする（例: --copt MinimumThinkingTime=1）。
終了コード: 0=H1採択、1=H0採択、2=安全弁まで走って判定に至らず、3=実行エラー。
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

BASELINE="$1"
CANDIDATE="$2"
NAME="$3"
shift 3
# 4番目が数値なら再試行の上限。それ以降はsprt.shへ素通しする
MAX_RETRY=20
if [[ $# -gt 0 && "$1" =~ ^[0-9]+$ ]]; then
	MAX_RETRY="$1"
	shift
fi
EXTRA=("$@")
JSONL="${REPO_ROOT}/data/sprt/${NAME}.jsonl"
RESULT="${REPO_ROOT}/data/sprt/${NAME}.result"
LOG="${REPO_ROOT}/data/logs/sprt-${NAME}.log"

# 判定済みなら走らせない（ADR-0175の冪等性）。結果はファイルが持つので、
# セッションをまたいでも同じ答えを返す
if [[ -f "$RESULT" ]]; then
	log_step "判定済み: ${RESULT}"
	cat "$RESULT"
	case "$(sed -n 's/^decision=//p' "$RESULT")" in
	H1) exit 0 ;;
	H0) exit 1 ;;
	*) die "結果ファイルのdecisionを読めない: ${RESULT}" ;;
	esac
fi

# 判定が出るまで走らせるため、--max-pairs は安全弁の値で1回通す。
# --max-pairs は通算のペア数なので、再開しても数え直しにならない（ADR-0087）
export SPRT_MAX_PAIRS="$SPRT_HARD_MAX_PAIRS"
log_step "判定が出るまで走らせる（安全弁 ${SPRT_HARD_MAX_PAIRS} ペア）"

for ((attempt = 1; attempt <= MAX_RETRY; attempt++)); do
	ARGS=()
	if [[ -s "$JSONL" ]]; then
		ARGS+=(--resume "$JSONL")
		log_step "試行 ${attempt}: 既存の棋譜から再開する（$(wc -l <"$JSONL") 局）"
	else
		log_step "試行 ${attempt}: 新規に開始する"
	fi

	"${SCRIPT_DIR}/sprt.sh" "$BASELINE" "$CANDIDATE" "$NAME" \
		${ARGS[@]+"${ARGS[@]}"} ${EXTRA[@]+"${EXTRA[@]}"}
	CODE=$?

	case $CODE in
	0 | 1)
		# 判定が出た。結果をファイルへ残す。以後この走行は再実行しない
		python3 "${SCRIPT_DIR}/sprt-summary.py" "$LOG" "$NAME" --emit-result "$RESULT" >/dev/null || true
		if [[ -f "$RESULT" ]]; then
			log_step "SPRT終了: 終了コード ${CODE}。結果を ${RESULT} へ書いた"
		else
			log_warn "判定は出たが結果ファイルを書けなかった: ${LOG}"
		fi
		exit $CODE
		;;
	2)
		# 安全弁まで走って判定に至らなかった。結果ファイルは書かない
		log_warn "安全弁（${SPRT_HARD_MAX_PAIRS} ペア）まで走って判定に至らず。"
		log_warn "局数を積むより対立仮説の立て方を見直す（ADR-0163）"
		exit 2
		;;
	*)
		log_warn "試行 ${attempt} が異常終了（コード ${CODE}）。再開する"
		sleep 5
		;;
	esac
done

die "${MAX_RETRY}回試しても判定に至らなかった"
