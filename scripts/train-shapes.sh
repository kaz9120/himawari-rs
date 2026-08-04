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
  SHAPE_DISTILL_NET 表現蒸留の教師にする.hmwr（ADR-0132）。太いFTの出力を
                    的にして、細いFTへ表現を写す。読むのはFTだけ
  SHAPE_LAMBDA_DISTILL
                    蒸留損失の重み。既定は train.py の 0.0。0.01以下から振る
  SHAPE_EFFECT_HEAD 利き予測ヘッドを付けてFTを事前学習する（ADR-0133）。
                    linear（線形1層）か mlp（中間256の2層）。SHAPE_LAMBDA_EFFECT
                    と対で渡す
  SHAPE_PEAK_LR     ピーク学習率。既定は train.py の 0.001。事前学習した表現が
                    序盤で壊れるのを避けたいとき下げる（ADR-0133）
  SHAPE_LAMBDA_VALUE
                    評価値損失の重み。0にすると評価値を切り、利き予測だけで
                    FTを事前学習する（ADR-0133の第1段階）
  SHAPE_LAMBDA_EFFECT
                    利き損失の重み。λ×利き損失÷value損失 の割合で決める
  SHAPE_GENERATE    教師データの代わりに局面をその場で作る（ADR-0133）。値は
                    1エポックあたりの局面数。SHAPE_TRAIN_DATA・SHAPE_VALID_DATA
                    は使わない。生成した局面は使い捨てなので検証集合が要らない
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
DISTILL_NET="${SHAPE_DISTILL_NET:-}"
LAMBDA_DISTILL="${SHAPE_LAMBDA_DISTILL:-}"
DISTILL_ARGS=()
if [[ -n "$DISTILL_NET" ]]; then
	[[ -f "$DISTILL_NET" ]] || die "蒸留の教師がない: ${DISTILL_NET}"
	DISTILL_ARGS+=(--distill-net "$DISTILL_NET")
	if [[ -n "$LAMBDA_DISTILL" ]]; then
		DISTILL_ARGS+=(--lambda-distill "$LAMBDA_DISTILL")
	fi
fi
PEAK_LR="${SHAPE_PEAK_LR:-}"
LAMBDA_VALUE="${SHAPE_LAMBDA_VALUE:-}"
EFFECT_HEAD="${SHAPE_EFFECT_HEAD:-}"
LAMBDA_EFFECT="${SHAPE_LAMBDA_EFFECT:-}"
EFFECT_ARGS=()
if [[ -n "$EFFECT_HEAD" ]]; then
	[[ -n "$LAMBDA_EFFECT" ]] || die "SHAPE_EFFECT_HEAD には SHAPE_LAMBDA_EFFECT が要る"
	EFFECT_ARGS+=(--effect-head "$EFFECT_HEAD" --lambda-effect "$LAMBDA_EFFECT")
fi
if [[ -n "$LAMBDA_VALUE" ]]; then
	EFFECT_ARGS+=(--lambda-value "$LAMBDA_VALUE")
fi
if [[ -n "$PEAK_LR" ]]; then
	EFFECT_ARGS+=(--peak-lr "$PEAK_LR")
fi
GENERATE="${SHAPE_GENERATE:-}"
DATA_ARGS=()
if [[ -n "$GENERATE" ]]; then
	# 局面をその場で作るので教師データを読まない。使い捨ての生成では
	# 訓練損失がそのまま未見データの損失になり、検証集合も要らない
	DATA_ARGS+=(--generate "$GENERATE")
	SRC_DESC="生成${GENERATE}局面"
else
	[[ -f "$DATA" ]] || die "学習データがない: ${DATA}"
	[[ -f "$VALID" ]] || die "検証データがない: ${VALID}"
	DATA_ARGS+=(--data "$DATA" --valid "$VALID")
	SRC_DESC="$DATA"
fi
mkdir -p training/runs/net_shape data/nets
# wheelの名前は構成によらず同じなので、同じ場所へ上書きされる
WHEEL_DIR="${REPO_ROOT}/target/wheels-shape"

log_step "構成ごとの学習（${#@}件）"
if [[ -n "$GENERATE" ]]; then
	log_info "学習データ: 生成（1エポック${GENERATE}局面）"
else
	log_info "学習データ: ${DATA}"
	log_info "検証データ: ${VALID}"
fi
log_info "デバイス: ${DEVICE}、札: ${TAG}、種: ${SEED}"
if [[ -n "$INIT_NET" ]]; then
	log_info "初期値: ${INIT_NET}${FREEZE_FT:+（FT凍結）}"
fi
if [[ -n "$DISTILL_NET" ]]; then
	log_info "蒸留の教師: ${DISTILL_NET}${LAMBDA_DISTILL:+（λ=${LAMBDA_DISTILL}）}"
fi
if [[ -n "$EFFECT_HEAD" ]]; then
	log_info "評価値の重み: ${LAMBDA_VALUE:-既定}"
	log_info "利き予測: ${EFFECT_HEAD}ヘッド（λ=${LAMBDA_EFFECT}）"
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
		"${DISTILL_ARGS[@]+"${DISTILL_ARGS[@]}"}" \
		"${EFFECT_ARGS[@]+"${EFFECT_ARGS[@]}"}" \
		"${DATA_ARGS[@]}" \
		--out "data/nets/${TAG}-${spec}-s${SEED}.hmwr" \
		--batch-loader --dense-ft --factorized \
		--device "$DEVICE" \
		--seed "$SEED" \
		--log-file "training/runs/net_shape/${TAG}-${spec}-s${SEED}.tsv" \
		--registry training/runs/registry.tsv \
		--name "${TAG}_${spec}_s${SEED}" \
		--notes "ネットワーク構成の比較（ADR-0127）: ${spec}、seed ${SEED}、${SRC_DESC}${INIT_NET:+、init ${INIT_NET}}${FREEZE_FT:+、FT凍結}${DISTILL_NET:+、蒸留 ${DISTILL_NET} λ=${LAMBDA_DISTILL:-既定}}${EFFECT_HEAD:+、利き ${EFFECT_HEAD} λ=${LAMBDA_EFFECT}}"
done

log_step "完了"
log_info "valid lossを比べる:"
echo "  column -t -s \$'\\t' training/runs/registry.tsv | grep '${TAG}_'"
