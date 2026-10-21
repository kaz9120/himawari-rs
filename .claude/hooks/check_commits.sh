#!/bin/sh
# Stopフックで、このブランチのコミットを検査する（ADR-0181）。
#
#   check_commits.sh parens   本文の入れ子の半角括弧（release-pleaseが黙って止まる）
#   check_commits.sh subject  件名の書き方をyomiyasuで見る（release-pleaseがCHANGELOGへ載せる）
#
# 違反を見つけたら終了コード2でセッションの終わりを止め、理由をエージェントへ返す。
# 検査そのものが動かなかったときは、警告を出して0で抜ける。検査の不調で
# セッションを終われなくしない。ただし黙っては通さない（issue #764）。
#
# 相対パスはリポジトリの根を基準にする。cwdがずれても同じ対象を見る。

mode=$1
root=${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null)}
warn() {
	echo "コミットの検査（$mode）を実行できなかった: $1" >&2
	exit 0
}
[ -n "$root" ] || warn 'リポジトリの根が分からない（CLAUDE_PROJECT_DIR が空で、gitの外にいる）'
cd "$root" || warn "$root へ移れない"

# 引数の書式でこのブランチのコミットを出す。gitが失敗したら警告を出して1を返す。
# 呼び出し側はコマンド置換の中なので、warn の exit では外側が止まらない
commits() {
	errf=$(mktemp) || return 1
	if sh .claude/hooks/branch_commits.sh "$1" 2>"$errf"; then
		rm -f "$errf"
		return 0
	fi
	echo "コミットの検査（$mode）を実行できなかった: branch_commits.sh が失敗した: $(cat "$errf")" >&2
	rm -f "$errf"
	return 1
}

case $mode in
parens)
	body=$(commits %B) || exit 0
	if printf '%s\n' "$body" | grep -qE '\([^()]*\('; then
		echo 'このブランチのコミット本文に入れ子の半角括弧がある。release-pleaseのパーサが失敗し、その回のリリースが黙って止まる。git commit --amend で直す（CLAUDE.md「コミット規律」）' >&2
		exit 2
	fi
	;;
subject)
	subjects=$(commits %s) || exit 0
	[ -z "$subjects" ] && exit 0
	[ -f hmwr/doclint.py ] || warn 'hmwr/doclint.py がない'
	out=$(printf '%s\n' "$subjects" | python3 hmwr/doclint.py - 2>&1)
	code=$?
	case $code in
	0) ;;
	1)
		printf '%s\n' "$out" >&2
		echo 'コミット件名がyomiyasuに引っかかっている。件名はrelease-please経由でCHANGELOGへ載り、後から直せない。git commit --amend で直す（CLAUDE.md「コミット規律」）' >&2
		exit 2
		;;
	*) warn "yomiyasuが終了コード $code で落ちた: $out" ;;
	esac
	;;
*)
	warn "知らない検査: $mode（parens か subject）"
	;;
esac
exit 0
