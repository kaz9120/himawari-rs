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

構成は <FT>x<L1>[x<L2>[x<L3>]]（例 256x16、512x16x32）。
1構成あたり3億局面で50〜100分かかる。

構成ごとに次を作る（<札>は既定 train）。
  data/nets/<札>-<構成>-s<種>.hmwr       学習したネット
  training/runs/net_shape/<札>-<構成>-s<種>.tsv 学習ログ（type=validの行）
結果は training/runs/registry.tsv にも1行ずつ積む。

データと乱数種は環境変数で変えられる。
  SHAPE_TRAIN_DATA  既定 data/train/train_300M.psv
  SHAPE_VALID_DATA  既定 data/train/valid_385M.psv
  SHAPE_SEED        既定 0。同じ構成を別の種で回すと、条件の差と初期値の
                    差を切り分けられる（ADR-0127）
  SHAPE_TAG         既定 train。出力名と実験名の頭に付く。データ量や
                    初期値を変えて測るとき、過去の結果を上書きせずに済む
  SHAPE_DEVICE      既定 cpu。mps を指定するとGPUへ載る（ADR-0064）。
                    現行ネットはmpsで学習しているので、本番規模で比べる
                    ときは揃える
  SHAPE_MMAP        1なら学習データをmmapで開く。RAMに載らない規模
                    （train_1900M.psv は79.7GB）ではこれがないと
                    OOMで落ちる。現行ネットもこの経路で学習している
  SHAPE_INIT_NET    既存の.hmwrを初期値に読む（ADR-0130）。FTは常に読み、
                    後段は形が一致する層だけ読む
  SHAPE_FREEZE_FT   1ならFTを凍結する。後段の候補を絞るときに使う。
                    採用の判断には使わない
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
TAG="${SHAPE_TAG:-train}"
DEVICE="${SHAPE_DEVICE:-cpu}"
MMAP_ARGS=()
if [[ -n "${SHAPE_MMAP:-}" ]]; then
	MMAP_ARGS+=(--mmap)
fi
INIT_NET="${SHAPE_INIT_NET:-}"
FREEZE_FT="${SHAPE_FREEZE_FT:-}"
INIT_ARGS=()
if [[ -n "$INIT_NET" ]]; then
	[[ -f "$INIT_NET" ]] || die "初期値のネットがない: ${INIT_NET}"
	INIT_ARGS+=(--init-net "$INIT_NET")
	[[ -n "$FREEZE_FT" ]] && INIT_ARGS+=(--freeze-ft)
fi
[[ -f "$DATA" ]] || die "学習データがない: ${DATA}"
[[ -f "$VALID" ]] || die "検証データがない: ${VALID}"
mkdir -p training/runs/net_shape data/nets
# wheelの名前は構成によらず同じなので、同じ場所へ上書きされる
WHEEL_DIR="${REPO_ROOT}/target/wheels-shape"

log_step "構成ごとの学習（${#@}件）"
log_info "学習データ: ${DATA}"
log_info "検証データ: ${VALID}"
log_info "デバイス: ${DEVICE}、札: ${TAG}、種: ${SEED}"
if [[ -n "$INIT_NET" ]]; then
	log_info "初期値: ${INIT_NET}${FREEZE_FT:+（FT凍結）}"
fi

for spec in "$@"; do
	if [[ ! "$spec" =~ ^[0-9]+x[0-9]+(x[0-9]+){0,2}$ ]]; then
		die "構成の書き方が違う: ${spec}（<FT>x<L1>[x<L2>[x<L3>]]）"
	fi

	log_info "PyO3拡張をビルド: ${spec}"
	# maturin develop はvirtualenvを要求する。macOSではpyenvのglobalへ
	# 入れているので、wheelを作って入れ替える
	HIMAWARI_ARCH="$spec" CARGO_TARGET_DIR="target/shape/${spec}" \
		maturin build --release --quiet -m crates/py/Cargo.toml --out "$WHEEL_DIR"
	# release-pleaseがバージョンを上げると古いwheelが残る。*.whl だと
	# 複数のバージョンにマッチしてpipが解決に失敗するので、最新だけ渡す
	python3 -m pip install --force-reinstall --no-deps --quiet \
		"$(ls -t "$WHEEL_DIR"/*.whl | head -1)"

	# 学習側と推論側で次元が食い違ったまま20分回すのを防ぐ
	got="$(python3 -c 'import himawari; print(himawari.ARCH)')"
	[[ "$got" == "$spec" ]] || die "拡張の構成が合わない: ${got}（期待 ${spec}）"

	log_info "学習: ${spec}"
	# 条件はADR-0126の詰みスコア実験と揃える。差が構成だけから出るようにする
	python3 training/train.py \
		"${INIT_ARGS[@]+"${INIT_ARGS[@]}"}" \
		"${MMAP_ARGS[@]+"${MMAP_ARGS[@]}"}" \
		--data "$DATA" \
		--valid "$VALID" \
		--out "data/nets/${TAG}-${spec}-s${SEED}.hmwr" \
		--batch-loader --dense-ft --factorized \
		--device "$DEVICE" \
		--seed "$SEED" \
		--log-file "training/runs/net_shape/${TAG}-${spec}-s${SEED}.tsv" \
		--registry training/runs/registry.tsv \
		--name "${TAG}_${spec}_s${SEED}" \
		--notes "ネットワーク構成の比較（ADR-0127）: ${spec}、seed ${SEED}、${DATA}${INIT_NET:+、init ${INIT_NET}}${FREEZE_FT:+、FT凍結}"
done

log_step "完了"
log_info "valid lossを比べる:"
echo "  column -t -s \$'\\t' training/runs/registry.tsv | grep '${TAG}_'"
