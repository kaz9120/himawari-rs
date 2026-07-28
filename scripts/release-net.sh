#!/usr/bin/env bash
# 学習済みネットをGitHub Releaseとして配布する（ADR-0080）。
#
# ネットはリポジトリで管理しない（data/ はgitignore）。学習を回した
# マシンからこのスクリプトで直接リリースを作る。CIは介さない。
# 成果物がそのマシンにしか存在しないためである。
set -euo pipefail

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
USAGE
}

if [[ $# -lt 2 ]]; then
	usage
	exit 1
fi

NET_PATH="$1"
VERSION="$2"
EXTRA_NOTE="${3:-}"

if [[ ! -f "$NET_PATH" ]]; then
	echo "エラー: ファイルがない: $NET_PATH" >&2
	exit 1
fi

if ! [[ "$VERSION" =~ ^[0-9]+$ ]]; then
	echo "エラー: バージョン番号は1以上の整数で指定する: $VERSION" >&2
	exit 1
fi

if ! command -v gh >/dev/null 2>&1; then
	echo "エラー: gh CLI が要る" >&2
	exit 1
fi

TAG="net-v${VERSION}"

if gh release view "$TAG" >/dev/null 2>&1; then
	echo "エラー: $TAG は既にある。番号を上げる" >&2
	exit 1
fi

# ヘッダからlineageを読む（ADR-0037の形式）。
#   magic 8B / version 4B / dims 12B / lineage長 4B / lineage / hash 8B / body
LLEN=$(od -An -tu4 -j24 -N4 "$NET_PATH" | tr -d ' ')
if [[ -z "$LLEN" || "$LLEN" -gt 4096 ]]; then
	echo "エラー: lineage長が読めない（Himawari NNUE形式か確認する）" >&2
	exit 1
fi
LINEAGE=$(dd if="$NET_PATH" bs=1 skip=28 count="$LLEN" 2>/dev/null)

ASSET_NAME="$(basename "$NET_PATH")"
ASSET_NAME="${ASSET_NAME%.best}"
SIZE=$(du -h "$NET_PATH" | cut -f1)

TMP_ASSET="$(mktemp -d)/${ASSET_NAME}"
cp "$NET_PATH" "$TMP_ASSET"

NOTES_FILE="$(mktemp)"
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

echo "タグ: $TAG"
echo "アセット: $ASSET_NAME ($SIZE)"
echo "lineage: $LINEAGE"
echo

gh release create "$TAG" "$TMP_ASSET" \
	--title "$TAG: $ASSET_NAME" \
	--notes-file "$NOTES_FILE" \
	--latest=false

rm -f "$NOTES_FILE"
rm -rf "$(dirname "$TMP_ASSET")"

echo
echo "作成した: $(gh release view "$TAG" --json url --jq .url)"
