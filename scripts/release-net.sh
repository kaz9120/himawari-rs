#!/usr/bin/env bash
# 学習済みネットをGitHub Releaseとして配布する（ADR-0080）。
#
# ネットはリポジトリで管理しない（data/ はgitignore）。学習を回した
# マシンからこのスクリプトで直接リリースを作る。CIは介さない。
# 成果物がそのマシンにしか存在しないためである。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./env.sh
source "${SCRIPT_DIR}/env.sh"

usage() {
	cat <<'USAGE'
使い方:
  scripts/release-net.sh <netファイル> <バージョン番号> [追記ノート]

例:
  scripts/release-net.sh data/nets/halfkp_1900M_fact.hmwr.best 1 \
    "対halfkp_370Mで+243.8 Elo（180局、H1採択）"

タグは net-v<バージョン番号> になる。エンジン本体のタグ（v0.7.x）とは
別系統で、release-pleaseの動作には干渉しない。

--latest=false で作るため、リリース一覧の「Latest」はエンジン本体の
ままになる。利用者が最初に見るのはエンジンであるべきである。

アセット名は入力ファイルから .best を外したものになる。
既定では作らない。走るはずのコマンドとリリースノートを出して終わる。
実際に作るには --apply を付ける。リリースは消しても「あった」ことが残る
ため、明示したときだけ作る（ADR-0122）。

USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
	usage
	exit 0
fi

# --apply を抜いてから位置引数を読む。既定は予行演習（ADR-0122）
release_take_apply "$@"
set -- ${RELEASE_ARGS[@]+"${RELEASE_ARGS[@]}"}

if [[ $# -lt 2 ]]; then
	usage
	exit 2
fi

NET_PATH="$1"
VERSION="$2"
EXTRA_NOTE="${3:-}"

require_file "$NET_PATH" "ネットファイル"
release_validate_version "$VERSION"

TAG="net-v${VERSION}"
release_check_prereqs "$TAG"

# ヘッダからlineageを読む（ADR-0037の形式）。
#   magic 8B / version 4B / dims 12B / lineage長 4B / lineage / hash 8B / body
LLEN=$(od -An -tu4 -j24 -N4 "$NET_PATH" | tr -d ' ')
if [[ -z "$LLEN" || "$LLEN" -gt 4096 ]]; then
	die "lineage長が読めない（Himawari NNUE形式か確認する）"
fi
LINEAGE=$(dd if="$NET_PATH" bs=1 skip=28 count="$LLEN" 2>/dev/null)

ASSET_NAME="$(basename "$NET_PATH")"
ASSET_NAME="${ASSET_NAME%.best}"
SIZE=$(release_file_size "$NET_PATH")

TMP_ASSET_DIR="$(mktemp -d)"
TMP_ASSET="${TMP_ASSET_DIR}/${ASSET_NAME}"
cp "$NET_PATH" "$TMP_ASSET"

NOTES_FILE="$(mktemp)"
trap 'rm -rf "$TMP_ASSET_DIR"; rm -f "$NOTES_FILE"' EXIT
{
	echo "## 学習来歴"
	echo
	echo '```'
	echo "$LINEAGE"
	echo '```'
	echo
	echo "## ファイル"
	echo
	echo "| 項目 | 値 |"
	echo "|---|---|"
	echo "| アセット | \`${ASSET_NAME}\` |"
	echo "| サイズ | ${SIZE} |"
	echo "| 形式 | Himawari NNUE（ADR-0037） |"
	if [[ -n "$EXTRA_NOTE" ]]; then
		echo
		echo "## 補足"
		echo
		echo "$EXTRA_NOTE"
	fi
	echo
	echo "## 使い方"
	echo
	echo '```'
	echo "gh release download ${TAG} -p '${ASSET_NAME}' -D data/nets/"
	echo '```'
	echo
	echo "USIオプション \`EvalFile\` にパスを指定する。"
	echo
	echo "計測の詳細は [RESULTS.md](../blob/main/docs/RESULTS.md) を参照。"
} >"$NOTES_FILE"

log_info "タグ: $TAG"
log_info "アセット: $ASSET_NAME ($SIZE)"
log_info "lineage: $LINEAGE"

release_create "$TAG" "$TAG: $ASSET_NAME" "$NOTES_FILE" "$TMP_ASSET"
