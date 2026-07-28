#!/usr/bin/env bash
# 定跡をGitHub Releaseとして配布する（ADR-0082）。
#
# 定跡は非決定的に生成されるため（ADR-0063）、コマンドを残しても
# 同じものは再現できない。成果物そのものを保存する。
set -euo pipefail

usage() {
	cat <<'USAGE'
使い方:
  scripts/release-book.sh <dbファイル> <バージョン番号> [genログ] [追記ノート]

例:
  scripts/release-book.sh data/book/mini.db 1 data/book/gen.log \
    "net-v1（halfkp_1900M_fact）で生成"

genログを渡すと、生成条件（ply/width/depth/threads）と使用ネットの
学習来歴をリリースノートへ自動で載せる。省略すると局面数だけになる。

タグは book-v<バージョン番号> になる。エンジン本体（v0.7.x）や
ネット（net-v<N>）とは別系統で、--latest=false で作る。
USAGE
}

if [[ $# -lt 2 ]]; then
	usage
	exit 1
fi

DB_PATH="$1"
VERSION="$2"
GEN_LOG="${3:-}"
EXTRA_NOTE="${4:-}"

if [[ ! -f "$DB_PATH" ]]; then
	echo "エラー: ファイルがない: $DB_PATH" >&2
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

TAG="book-v${VERSION}"

if gh release view "$TAG" >/dev/null 2>&1; then
	echo "エラー: $TAG は既にある。番号を上げる" >&2
	exit 1
fi

# 形式の確認。db2016互換のヘッダで始まる
if ! head -1 "$DB_PATH" | grep -q '^#YANEURAOU-DB'; then
	echo "エラー: db2016形式のヘッダがない: $DB_PATH" >&2
	exit 1
fi

POSITIONS=$(grep -c '^sfen' "$DB_PATH")
SIZE=$(du -h "$DB_PATH" | cut -f1)
ASSET_NAME="$(basename "$DB_PATH")"

# genログから生成条件を拾う（ADR-0082でbook genが出すようにした）
BOOKGEN=""
EVALFILE=""
ELAPSED=""
if [[ -n "$GEN_LOG" && -f "$GEN_LOG" ]]; then
	BOOKGEN=$(grep -m1 '^BookGen:' "$GEN_LOG" || true)
	EVALFILE=$(grep -m1 '^EvalFile:' "$GEN_LOG" || true)
	ELAPSED=$(grep -oE '（[0-9]+秒）' "$GEN_LOG" | tail -1 || true)
fi

NOTES_FILE="$(mktemp)"
{
	echo "## 定跡"
	echo
	echo "| 項目 | 値 |"
	echo "|---|---|"
	echo "| アセット | \`${ASSET_NAME}\` |"
	echo "| 局面数 | ${POSITIONS} |"
	echo "| サイズ | ${SIZE} |"
	echo "| 形式 | やねうら王 db2016 互換（ADR-0063） |"
	if [[ -n "$ELAPSED" ]]; then
		echo "| 生成所要 | ${ELAPSED//[（）]/} |"
	fi
	if [[ -n "$BOOKGEN" ]]; then
		echo
		echo "## 生成条件"
		echo
		echo '```'
		echo "${BOOKGEN#BookGen: }"
		echo '```'
	fi
	if [[ -n "$EVALFILE" ]]; then
		echo
		echo "## 生成に使った評価関数"
		echo
		echo '```'
		echo "${EVALFILE#EvalFile: }"
		echo '```'
	fi
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
	echo "gh release download ${TAG} -D data/book/"
	echo '```'
	echo
	echo "USIオプション \`BookFile\` にパスを指定する。既定は定跡なし。"
	echo "\`BookDepth\` で定跡を引く手数の上限を決める（既定24）。"
	echo
	echo "生成は非決定的である（ADR-0063）。同じコマンドでも内容が変わるため、"
	echo "再現ではなくこの成果物を使う。"
} >"$NOTES_FILE"

echo "タグ: $TAG"
echo "アセット: $ASSET_NAME (${POSITIONS}局面, $SIZE)"
[[ -n "$BOOKGEN" ]] && echo "$BOOKGEN"
echo

gh release create "$TAG" "$DB_PATH" \
	--title "$TAG: ${ASSET_NAME} (${POSITIONS}局面)" \
	--notes-file "$NOTES_FILE" \
	--latest=false

rm -f "$NOTES_FILE"

echo
echo "作成した: $(gh release view "$TAG" --json url --jq .url)"
