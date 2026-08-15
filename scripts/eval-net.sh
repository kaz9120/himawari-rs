#!/usr/bin/env bash
# ネットの検証損失を測る（ADR-0149）。
#
# 学習を回さずに損失だけ見たい場面がある。継続学習の条件比較（ADR-0145）
# のように、書き出したネットを並べて測るときに使う。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./env.sh
source "${SCRIPT_DIR}/env.sh"

usage() {
	cat <<'USAGE'
使い方:
  scripts/eval-net.sh <ネット> [ネット...]

例:
  scripts/eval-net.sh data/nets/ft1024_2990M_q1_reorder.hmwr
  scripts/eval-net.sh data/nets/*.hmwr.best

渡したネットを検証集合で測って表にする。学習はしない。
.hmwr は量子化を経ているので、学習中に出る値とは丸めのぶん違う。
f32のチェックポイント（.ckpt）も同じように渡せる。

検証集合の既定は data/train/valid_385M_q1.psv で、現行の教師と同じ静止化を
かけたものである（ADR-0136）。**学習データと土俵を揃える。**

土俵を跨いで比べたいときだけコンマ区切りで並べる。教師データの分布を変える
実験では物差しも動くので、そのときは複数を渡す必要がある。
  EVAL_VALIDS=data/train/valid_385M.psv,data/train/valid_385M_q1.psv \
    scripts/eval-net.sh <ネット>

ネットワーク構成はPyO3拡張の次元で決まる。構成を変えて測るときは
scripts/build-shapes.sh で拡張を作り直してから呼ぶ。
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
	usage
	exit 0
fi

if [[ $# -lt 1 ]]; then
	usage
	exit 2
fi

cd "$REPO_ROOT"

VALIDS="${EVAL_VALIDS:-data/train/valid_385M_q1.psv}"

log_step "検証損失の測定（${#@}件）"

for v in $(printf '%s' "$VALIDS" | tr ',' ' '); do
	[[ -f "$v" ]] || die "検証データがない: ${v}"
done

printf '%-52s %-32s %s\n' "ネット" "検証集合" "loss"
for net in "$@"; do
	[[ -f "$net" ]] || die "ネットがない: ${net}"
	# 拡張子で初期値の読み方を選ぶ。.hmwr は量子化済み、.ckpt はf32
	case "$net" in
	*.ckpt | *.pt) init_flag="--init-checkpoint" ;;
	*) init_flag="--init-net" ;;
	esac
	for v in $(printf '%s' "$VALIDS" | tr ',' ' '); do
		loss="$(python3 training/train.py --eval-only "$init_flag" "$net" \
			--valid "$v" --batch-loader --dense-ft --factorized \
			--device "${EVAL_DEVICE:-cpu}" 2>/dev/null | awk -F'\t' '{print $3}')"
		[[ -n "$loss" ]] || die "測れない: ${net} / ${v}"
		printf '%-52s %-32s %s\n' "$(basename "$net")" "$(basename "$v")" "$loss"
	done
done
