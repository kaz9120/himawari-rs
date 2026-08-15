#!/usr/bin/env bash
# 本番規模のネットを学習する（ADR-0149）。
#
# scripts/train-shapes.sh は構成を比べる小規模実験用で、PyO3拡張を毎回
# ビルドし直す。本番の学習は構成が固定なので、拡張はそのまま使う。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./env.sh
source "${SCRIPT_DIR}/env.sh"

usage() {
	cat <<'USAGE'
使い方:
  scripts/train-net.sh <名前> <学習データ> [検証データ]

例:
  scripts/train-net.sh halfkp_2990M_q1 data/train/train_2990M_q1.psv \
                       data/train/valid_385M_q1.psv

data/nets/<名前>.hmwr へ書き出し、training/checkpoints/<名前>/ へf32の
チェックポイントを残す。ログは data/logs/<名前>.log、実験台帳は
training/runs/registry.tsv へ積む。

**検証データは学習データと同じ土俵に揃える。** 静止化した教師で学習する
なら検証集合も静止化する。土俵がずれるとbest checkpointの選択が歪む
（ADR-0136）。省略すると data/train/valid_385M.psv を使う。

既定はADRの結論に揃えてある。
  FTクリップ1.0  ADR-0138。i8で格納するので制約なしだと書き出しが落ちる
  --mmap         ADR-0065。29.9億のpsvは111GBでRAMに載らない
  factorizer     ADR-0066。+28.1 Elo
  epochs=1 batch=16384 peak_lr=1e-3 lambda=0.7  ADR-0135

環境変数で変えられる。
  TRAIN_INIT_CKPT  f32チェックポイントから継続学習する（ADR-0145）
  TRAIN_PEAK_LR    継続学習では1e-4が既定（ADR-0145）
  TRAIN_WARMUP     継続学習のwarmupステップ数。既定は総ステップの4%
  TRAIN_DEVICE     既定 mps（ADR-0064）。MPSがない環境では cpu を渡す
  TRAIN_SEED       既定 0
  TRAIN_NOTES      台帳へ書く備考
  TRAIN_EXTRA_ARGS train.pyへそのまま渡す追加引数（例: --mirror-factor）
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

NAME="$1"
DATA="$2"
VALID="${3:-data/train/valid_385M.psv}"

cd "$REPO_ROOT"
require_file "$DATA" "学習データ"
require_file "$VALID" "検証データ"

# PSVは1局面40バイト固定（ADR-0038）
psv_positions() {
	local bytes
	bytes=$(wc -c <"$1" | tr -d ' ')
	printf '%s' $((bytes / 40))
}

INIT_ARGS=()
PEAK_LR="${TRAIN_PEAK_LR:-}"
if [[ -n "${TRAIN_INIT_CKPT:-}" ]]; then
	require_file "$TRAIN_INIT_CKPT" "初期値のチェックポイント"
	INIT_ARGS+=(--init-checkpoint "$TRAIN_INIT_CKPT")
	# 前世代の表現を壊さない幅。3e-4では壊れる（ADR-0145）
	PEAK_LR="${PEAK_LR:-1e-4}"
	# warmupは総ステップの4%にする。固定値だと規模で意味が変わり、824万局面
	# （503ステップ）で決めた20は1億局面（6,100ステップ）では0.3%になる。
	# 学習率を上げきるまでの区間が短いほど前世代の表現が壊れやすい
	steps=$(( $(psv_positions "$DATA") / 16384 ))
	warmup="${TRAIN_WARMUP:-$(( steps * 4 / 100 ))}"
	[[ "$warmup" -lt 20 ]] && warmup=20
	# 検証はステップ数に対して細かすぎない間隔にする
	vint=$(( steps / 20 ))
	[[ "$vint" -lt 50 ]] && vint=50
	log_info "総ステップ ${steps}、warmup ${warmup}、検証間隔 ${vint}"
	INIT_ARGS+=(--warmup-steps "$warmup" --valid-interval "$vint")
fi
LR_ARGS=()
if [[ -n "$PEAK_LR" ]]; then
	LR_ARGS+=(--peak-lr "$PEAK_LR")
fi

mkdir -p data/nets training/runs/net_shape "training/checkpoints/${NAME}"

log_step "学習: $NAME"
log_info "学習データ: $DATA"
log_info "検証データ: $VALID"
log_info "出力      : data/nets/${NAME}.hmwr"
if [[ -n "${TRAIN_INIT_CKPT:-}" ]]; then
	log_info "初期値    : ${TRAIN_INIT_CKPT}（継続学習、peak_lr ${PEAK_LR}）"
fi

# 実験ごとの追加引数。既定に無いフラグを試すときに使う（ADR-0158の
# --mirror-factor など）。空白区切りで複数渡せる
EXTRA_ARGS=()
if [[ -n "${TRAIN_EXTRA_ARGS:-}" ]]; then
	read -r -a EXTRA_ARGS <<<"${TRAIN_EXTRA_ARGS}"
	log_info "追加引数  : ${TRAIN_EXTRA_ARGS}"
fi

run_logged "$NAME" python3 training/train.py \
	--data "$DATA" \
	--valid "$VALID" \
	--out "data/nets/${NAME}.hmwr" \
	--batch-loader --dense-ft --factorized --mmap \
	--ft-clip 1.0 \
	--device "${TRAIN_DEVICE:-mps}" \
	--seed "${TRAIN_SEED:-0}" \
	--checkpoint-dir "training/checkpoints/${NAME}" \
	--log-file "training/runs/net_shape/${NAME}.tsv" \
	--registry training/runs/registry.tsv \
	--name "$NAME" \
	--notes "${TRAIN_NOTES:-${DATA} で学習（ADR-0149のtrain-net.sh）}" \
	${INIT_ARGS[@]+"${INIT_ARGS[@]}"} \
	${LR_ARGS[@]+"${LR_ARGS[@]}"} \
	${EXTRA_ARGS[@]+"${EXTRA_ARGS[@]}"}
