#!/usr/bin/env bash
# ネットワーク構成ごとに小さく学習し、valid lossを比べる（ADR-0127）。
#
# 学習側の次元はPyO3拡張から読む（training/model.py）。構成を変えるには
# 拡張をビルドし直すしかないので、1構成ずつ順に回す。並列に回すと拡張が
# 上書きし合い、次元の違う構成で学習してしまう。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./env.sh
source "${SCRIPT_DIR}/env.sh"

usage() {
	cat <<'USAGE'
使い方:
  scripts/train-shapes.sh <構成> [構成...]

構成は <FT>x<L1>x<L2>[x<L3>]（例 512x16x32、256x32x32x32）。
1構成あたり3億局面で50〜100分かかる。

構成ごとに次を作る。
  data/nets/train-<構成>-s<種>.hmwr       学習したネット
  training/runs/net_shape/<構成>-s<種>.tsv 学習ログ（type=validの行）
結果は training/runs/registry.tsv にも1行ずつ積む。

データと乱数種は環境変数で変えられる。
  SHAPE_TRAIN_DATA  既定 data/train/train_300M.psv
  SHAPE_VALID_DATA  既定 data/train/valid_385M.psv
  SHAPE_SEED        既定 0。同じ構成を別の種で回すと、条件の差と初期値の
                    差を切り分けられる（ADR-0127）
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

DATA="${SHAPE_TRAIN_DATA:-data/train/train_300M.psv}"
VALID="${SHAPE_VALID_DATA:-data/train/valid_385M.psv}"
SEED="${SHAPE_SEED:-0}"
[[ -f "$DATA" ]] || die "学習データがない: ${DATA}"
[[ -f "$VALID" ]] || die "検証データがない: ${VALID}"
mkdir -p training/runs/net_shape data/nets
# wheelの名前は構成によらず同じなので、同じ場所へ上書きされる
WHEEL_DIR="${REPO_ROOT}/target/wheels-shape"

log_step "構成ごとの学習（${#@}件）"
log_info "学習データ: ${DATA}"
log_info "検証データ: ${VALID}"

for spec in "$@"; do
	if [[ ! "$spec" =~ ^[0-9]+x[0-9]+x[0-9]+(x[0-9]+)?$ ]]; then
		die "構成の書き方が違う: ${spec}（<FT>x<L1>x<L2>[x<L3>]）"
	fi

	log_info "PyO3拡張をビルド: ${spec}"
	# maturin develop はvirtualenvを要求する。macOSではpyenvのglobalへ
	# 入れているので、wheelを作って入れ替える
	HIMAWARI_ARCH="$spec" CARGO_TARGET_DIR="target/shape/${spec}" \
		maturin build --release --quiet -m crates/py/Cargo.toml --out "$WHEEL_DIR"
	python3 -m pip install --force-reinstall --no-deps --quiet "$WHEEL_DIR"/*.whl

	# 学習側と推論側で次元が食い違ったまま20分回すのを防ぐ
	got="$(python3 -c 'import himawari; print(himawari.ARCH)')"
	[[ "$got" == "$spec" ]] || die "拡張の構成が合わない: ${got}（期待 ${spec}）"

	log_info "学習: ${spec}"
	# 条件はADR-0126の詰みスコア実験と揃える。差が構成だけから出るようにする
	python3 training/train.py \
		--data "$DATA" \
		--valid "$VALID" \
		--out "data/nets/train-${spec}-s${SEED}.hmwr" \
		--batch-loader --dense-ft --factorized \
		--seed "$SEED" \
		--log-file "training/runs/net_shape/${spec}-s${SEED}.tsv" \
		--registry training/runs/registry.tsv \
		--name "shape_${spec}_s${SEED}" \
		--notes "ネットワーク構成の比較（ADR-0127）: ${spec}、seed ${SEED}"
done

log_step "完了"
log_info "valid lossを比べる:"
echo "  column -t -s \$'\\t' training/runs/registry.tsv | grep shape_"
