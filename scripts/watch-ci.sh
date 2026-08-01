#!/usr/bin/env bash
# PRのCI完了を待つ（ADR-0098）。Monitorツールから呼ぶ前提。
#
# `until gh pr checks ...; do sleep; done` を直接Monitorへ渡すと、複合
# コマンドのため権限の許可規則で拾えない。読み取り専用のスクリプトへ
# 切り出し、`Bash(./scripts/watch-ci.sh:*)` で許可する。
# マージはしない（破壊的操作を読み取り専用スクリプトへ混ぜない）。
#
# -eを付けない。gh pr checks は一時的なネットワーク不調などでも
# 非0を返しうるが、それだけでループごと終了させたくない。判定は
# grepの結果で明示的に分岐し、それ以外の失敗はポーリングを続ける
set -uo pipefail

# 最大30分ポーリングするため、ログに時刻を付ける
LOG_TIMESTAMP=1
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./env.sh
source "${SCRIPT_DIR}/env.sh"

usage() {
	cat <<'USAGE'
使い方:
  scripts/watch-ci.sh <PR番号> [確認間隔秒]

CIが pass か fail に確定するまで待ち、結果を1行で表示する。

終了コード: 0=pass、1=fail、2=引数エラー、3=待機の上限に達した。
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

PR="$1"
INTERVAL="${2:-25}"
# 上限30分。CIは通常3分以内に終わる
DEADLINE=$((SECONDS + 1800))

while ((SECONDS < DEADLINE)); do
	OUT="$(gh pr checks "$PR" 2>&1 || true)"
	if grep -qE '^check[[:space:]]+pass' <<<"$OUT"; then
		log_info "PR#${PR} CI pass"
		exit 0
	fi
	if grep -qE '^check[[:space:]]+fail' <<<"$OUT"; then
		log_error "PR#${PR} CI fail"
		echo "$OUT" | head -3 >&2
		exit 1
	fi
	sleep "$INTERVAL"
done

die "PR#${PR} のCIが30分で確定しなかった"
