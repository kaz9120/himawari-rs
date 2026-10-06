# 巡回（週次）

[共通の約束](README.md)を先に読む。直さない。見つけてIssueに切るだけである。

## 観点

観点の正はmicro-improvementsスキルの「巡回の観点」にある。1回の巡回で
全部を見なくてよい。直近の `micro` Issueの題を見て、しばらく見ていない観点から
始める。

```
gh api 'repos/{owner}/{repo}/issues?state=all&labels=micro&sort=created&direction=desc&per_page=20' \
  --jq '.[] | "\(.number)\t\(.created_at[0:10])\t\(.title)"'
```

## 手順

1. 観点ごとに、食い違いや粗を探す。**推測で切らない**。ファイルと行を確かめ、
   文書と実装の食い違いなら両方を読む
2. 同じ内容のIssueが開いていないかを確かめる
3. 1件ずつ `micro` Issueに切る。題は直す対象が分かる形にする。本文には、
   気になる点（ファイルと行）、直し方の案、判断軸（micro-improvementsスキルの
   4軸のどれか）を書く
4. 実行の場所のラベルを付ける。リポジトリだけで直せて計測が要らないなら
   `cloud`、計測か `data/` か設計判断が要るなら `local`
5. 開いている `micro` Issueのうち、実行の場所のラベルが無いものを同じ基準で
   仕分ける。手で切ったIssueはラベルが漏れやすい。漏れると、夜間のRoutineにも
   対話セッションにも拾われない

   ```
   gh api 'repos/{owner}/{repo}/issues?state=open&labels=micro&per_page=100' \
     --jq '.[] | select([.labels[].name] | (index("cloud") or index("local")) | not)
                 | "\(.number)\t\(.title)"'
   ```

1回の巡回で切るのは5件までにする。多すぎると消化が追いつかず、読まれなくなる。
何も見つからなければ、何もせず終わる。
