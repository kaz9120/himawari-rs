#!/bin/sh
# このブランチで足したコミットを、引数の書式で出す（Stopフックの件名・本文の検査用）。
#
# 基点はローカルの main と origin/main の両方で、どちらかに含まれるコミットを外す。
# クラウドのcloneは origin/main が古いまま HEAD が main の先端にあり、開発機は
# ローカルの main が古いまま origin/main から切る。片方だけを基点にすると、
# マージ済みのコミットを「このブランチのコミット」として拾う（issue #579）。
#
# `--branches=main` は末尾に /* を補って main 自体に一致しないので、refを列挙して渡す。
base=$(git for-each-ref --format='%(refname)' refs/heads/main refs/remotes/origin/main)
[ -z "$base" ] && exit 0
# shellcheck disable=SC2086
git log HEAD --not $base --format="$1"
