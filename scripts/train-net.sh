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
  TRAIN_DEVICE     既定 cpu
  TRAIN_SEED       既定 0
  TRAIN_NOTES      台帳へ書く備考
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

INIT_ARGS=()
PEAK_LR="${TRAIN_PEAK_LR:-}"
if [[ -n "${TRAIN_INIT_CKPT:-}" ]]; then
	require_file "$TRAIN_INIT_CKPT" "初期値のチェックポイント"
	INIT_ARGS+=(--init-checkpoint "$TRAIN_INIT_CKPT")
	# 前世代の表現を壊さない幅。3e-4では壊れる（ADR-0145）
	PEAK_LR="${PEAK_LR:-1e-4}"
	# 継続学習は総ステップが短い。warmupが既定の100だと割合が大きすぎる
	INIT_ARGS+=(--warmup-steps 20 --valid-interval 100)
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

run_logged "$NAME" python3 training/train.py \
	--data "$DATA" \
	--valid "$VALID" \
	--out "data/nets/${NAME}.hmwr" \
	--batch-loader --dense-ft --factorized --mmap \
	--ft-clip 1.0 \
	--device "${TRAIN_DEVICE:-cpu}" \
	--seed "${TRAIN_SEED:-0}" \
	--checkpoint-dir "training/checkpoints/${NAME}" \
	--log-file "training/runs/net_shape/${NAME}.tsv" \
	--registry training/runs/registry.tsv \
	--name "$NAME" \
	--notes "${TRAIN_NOTES:-${DATA} で学習（ADR-0149のtrain-net.sh）}" \
	${INIT_ARGS[@]+"${INIT_ARGS[@]}"} \
	${LR_ARGS[@]+"${LR_ARGS[@]}"}
