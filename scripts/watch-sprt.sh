#!/usr/bin/env bash
# SPRTの判定を待つ（ADR-0098）。Monitorツールから呼ぶ前提。
#
# 待機ループを直接Monitorへ渡すと、複合コマンドのため権限の許可規則で
# 拾えず、そのたびに確認を求められる。ログを読むだけの読み取り専用
# スクリプトへ切り出し、`Bash(./scripts/watch-sprt.sh:*)` で許可する。
set -uo pipefail

usage() {
	cat <<'USAGE'
使い方:
  scripts/watch-sprt.sh <SPRTのログファイル> [確認間隔秒]

判定行（----で始まる区切りの次の行）が出たら結果を表示して終了する。
selfplayプロセスが消えていたら中断とみなし、終了コード1で抜ける。

終了コード: 0=判定が出た、1=中断された、2=引数エラー。
USAGE
}

if [[ $# -lt 1 || $# -gt 2 ]]; then
	usage
	exit 2
fi

LOG="$1"
INTERVAL="${2:-45}"

while true; do
	# 判定に達すると selfplay が "----" の区切り行と結論を書く
	if grep -qE '^----' "$LOG" 2>/dev/null; then
		tail -3 "$LOG"
		exit 0
	fi
	if ! pgrep -f 'selfplay --baseline' >/dev/null 2>&1; then
		echo "SPRTが判定前に止まった（中断・停止・失敗のいずれか）"
		tail -2 "$LOG" 2>/dev/null
		exit 1
	fi
	sleep "$INTERVAL"
done
