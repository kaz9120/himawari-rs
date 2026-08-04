#!/usr/bin/env bash
# 教師データの取得と前処理（DATASETS.md の手順をスクリプト化）。
#
# nodchip/shogi_hao_depth9 から381ファイル（約116GB・約29.9億局面）を
# 取得し、train_2990M.psv と valid_385M.psv を作る。
# 開発機を移すときはこれ1本で教師データを再現できる。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./env.sh
source "${SCRIPT_DIR}/env.sh"

RAW_DIR="${RAW_DIR:-${REPO_ROOT}/data/raw/hao_depth9}"
TRAIN_DIR="${TRAIN_DIR:-${REPO_ROOT}/data/train}"
JOBS="${JOBS:-4}"

BASE_URL="https://huggingface.co/datasets/nodchip/shogi_hao_depth9/resolve/main"
PREFIX="kifu.tag=train.depth=9.num_positions=1000000000"
# 第3グループ1695872823はADR-0135で追加した（19.9億→29.9億）
START_TIMES=(1695340981 1695606850 1695872823)
# 検証データの供給元。学習データからは除く（対局単位で分離するため）
VALID_START_TIME=1695340981
VALID_INDEX=023
VALID_COUNT=200000
# Entry not found は15バイトで返る。実ファイルは280〜350MBある
MIN_SIZE=$((100 * 1024 * 1024))

usage() {
	cat <<'USAGE'
使い方:
  scripts/fetch-dataset.sh download   HuggingFaceから381ファイルを取得する
  scripts/fetch-dataset.sh verify     取得済みファイルのサイズを検査する
  scripts/fetch-dataset.sh prepare    train_2990M.psv と valid_385M.psv を作る
  scripts/fetch-dataset.sh all        download → verify → prepare

環境変数:
  RAW_DIR    生データの置き場（既定 data/raw/hao_depth9）
  TRAIN_DIR  加工済みpsvの置き場（既定 data/train）
  JOBS       並列ダウンロード数（既定 4）

download は再実行できる。妥当なサイズのファイルは飛ばす。
prepare には psv ツールが要る（cargo build --release）。

所要の目安: download は回線次第、prepare のシャッフルは約3分。
必要な空き容量は raw 116GB + train 120GB で約236GB。
USAGE
}

file_name() {
	printf '%s.start_time=%s.thread_index=%s.bin' "$PREFIX" "$1" "$2"
}

url_of() {
	# HuggingFaceは = をエンコードしたパスで配る
	printf '%s/%s' "$BASE_URL" "$(printf '%s' "$1" | sed 's/=/%3D/g')"
}

fetch_one() {
	local name="$1" dest="$2"
	local path="${dest}/${name}"
	if [[ -f "$path" ]]; then
		local size
		size=$(wc -c <"$path")
		if [[ "$size" -ge "$MIN_SIZE" ]]; then
			return 0
		fi
		echo "再取得（サイズ不足 ${size}B）: $name" >&2
		rm -f "$path"
	fi
	curl -fsSL --retry 3 --retry-delay 5 -o "${path}.part" "$(url_of "$name")"
	mv "${path}.part" "$path"
	echo "取得: $name"
}
export -f fetch_one url_of
export BASE_URL MIN_SIZE

cmd_download() {
	mkdir -p "$RAW_DIR"
	local names=()
	for st in "${START_TIMES[@]}"; do
		for i in $(seq -f '%03g' 0 126); do
			names+=("$(file_name "$st" "$i")")
		done
	done
	log_info "対象 ${#names[@]} ファイル、並列 ${JOBS}、置き場 ${RAW_DIR}"
	printf '%s\n' "${names[@]}" \
		| xargs -P "$JOBS" -I{} bash -c 'fetch_one "$@"' _ {} "$RAW_DIR"
}

cmd_verify() {
	local bad=0 count=0
	for st in "${START_TIMES[@]}"; do
		for i in $(seq -f '%03g' 0 126); do
			local name path size
			name="$(file_name "$st" "$i")"
			path="${RAW_DIR}/${name}"
			count=$((count + 1))
			if [[ ! -f "$path" ]]; then
				log_warn "欠落: $name"
				bad=$((bad + 1))
				continue
			fi
			size=$(wc -c <"$path")
			if [[ "$size" -lt "$MIN_SIZE" ]]; then
				log_warn "サイズ不足 ${size}B: $name"
				bad=$((bad + 1))
			fi
		done
	done
	log_info "検査 ${count} ファイル、異常 ${bad} 件"
	[[ "$bad" -eq 0 ]]
}

cmd_prepare() {
	local psv="${REPO_ROOT}/target/release/psv"
	if [[ ! -x "$psv" ]]; then
		die "${psv} がない。先に cargo build --release を実行する"
	fi
	mkdir -p "$TRAIN_DIR"

	local valid_src
	valid_src="${RAW_DIR}/$(file_name "$VALID_START_TIME" "$VALID_INDEX")"
	log_info "検証データを切り出す（${VALID_COUNT}局面）"
	"$psv" head --in "$valid_src" --out "${TRAIN_DIR}/valid_385M.psv" --count "$VALID_COUNT"

	log_info "学習データをシャッフルする（検証データの供給元を除く253ファイル）"
	local files
	files=$(find "$RAW_DIR" -name '*.bin' \
		! -name "*start_time=${VALID_START_TIME}.thread_index=${VALID_INDEX}.bin" \
		| sort | paste -sd, -)
	"$psv" shuffle --in "$files" --out "${TRAIN_DIR}/train_2990M.psv" --seed 42

	echo
	"$psv" stats --in "${TRAIN_DIR}/train_2990M.psv" --limit 1 || true
	ls -lh "${TRAIN_DIR}/train_2990M.psv" "${TRAIN_DIR}/valid_385M.psv"
}

case "${1:-}" in
download) cmd_download ;;
verify) cmd_verify ;;
prepare) cmd_prepare ;;
all)
	cmd_download
	cmd_verify
	cmd_prepare
	;;
-h | --help)
	usage
	exit 0
	;;
*)
	usage
	exit 2
	;;
esac
