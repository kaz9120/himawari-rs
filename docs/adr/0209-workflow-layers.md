# 0209: 仕事を4つの層へ振り分け、実験をIssueのキューで無人実行する

- Status: proposed
- Date: 2026-09-17
- 関連ADR: [0070](0070-pr-based-workflow.md), [0123](0123-stop-and-resume.md), [0149](0149-experiment-runner.md), [0152](0152-floodgate-cycle.md), [0175](0175-sprt-until-decision.md), [0181](0181-agent-surface.md), [0189](0189-artifact-retention.md), [0207](0207-roadmap-focus-eval.md), [0208](0208-hmwr-resource-verbs.md)

## Context

`micro` Issueのサイクルが止まっていた。2026-08-30は作成14件・消化10件
だったが、2026-09-04を最後に作成も消化も0件である（2026-09-17に集計）。
オーナーの診断は「明確な開発プロセスを作れていない。もっと決定論的に
表現できれば起きない」だった。

決定論的でない箇所を数えると5つあった。

| 箇所 | いまの書き方 | 起きること |
|---|---|---|
| セッションの入口 | ROADMAPの「次に着手する」を読む | 終わった実験の後処理や開いたIssueを、誰も見に行かない |
| `micro` の発動 | 「待ち時間が生まれたとき」 | 待ち時間の使い方がセッションごとに変わる |
| Issueの発見 | 「気になりを見つけたら」 | 気づくかどうかに依存し、探しに行く手順がない |
| 定期の運用 | 「月1回を目安」「機を見て」 | 最後にいつ実行したかを誰も持っていない |
| 実験の実行 | 使い捨てのチェーンスクリプト | 前の実験を `pgrep` で待つ手製のキューが13本ある。途中で止まると最初からやり直しになる |

共通の原因は、すべてを対話セッションの中で解こうとしていることである。
オーナーの方針は2つある（2026-09-17）。スクリプトで書ける部分はLLMに
頼らない。定期的に実行できる独立した仕事は、Claude Code Routinesのような
仕組みに載せる。ゴールは、仕事の置き場を整理し、置き場どうしをつなぐ
状態の持ち方を設計することである。

制約が1つある。`data/`（教師・ネット・棋譜・ログ）はgitignoreで、開発機に
しかない。GitHub ActionsとRoutinesはリポジトリをcloneして動くので、
`data/` と開発機の計算資源に触れない。

## 選択肢と比較

### 案A: 対話セッションの入口で、次の仕事を判定するコマンドを作る

`hmwr next` が状態を読み、優先順位表から次の仕事を返す。作るものは少ない。
しかしセッションが開かれない限り何も進まず、判定の後はLLMが実行する。
スクリプトで済む仕事までLLMを通る。

### 案B: 仕事を実行の場所で振り分け、状態をGitHubに集める

判断が要るか、`data/` か開発機が要るか、の2問で置き場を決める。置き場は
4つになる。状態はどの置き場からも届くGitHubに置く。

### 案C: 専用の状態ファイルやボードを新設する

GitHub Projectsや、リポジトリ内の状態ファイルで進捗を持つ。見通しは良い。
しかし状態の置き場が増え、IssueやADRのStatusと二重管理になる。

## Decision

**案Bを採る**。決め手は `data/` の制約である。置き場を決める問いが実行
環境で決まるので、振り分けに判断が入らない。

### 振り分けの規則

| | リポジトリだけで完結する | `data/` か開発機が要る |
|---|---|---|
| 判断が要らない | GitHub Actions | 開発機の定期実行（launchdと `hmwr`） |
| 判断が要る | Claude Code Routines | 対話セッション、または開発機の `claude -p` |

新しい仕事が生まれたら、この表で置き場を決める。**判断が要らない仕事を
LLMに渡さない**。

### 各層の仕事

**GitHub Actions**

- CIとリリース（既存）
- 文書の整合検査。ADR本文のStatusと索引の一致。ROADMAPの候補が
  accepted・rejectedのADRを指していないこと。proposedが30日以上動いて
  いないこと。`experiments/` のspecの検証（下に書く）。PRでは落とし、
  週次の実行では違反をIssueへ起票する
- floodgateのレートの定点記録。週次で取得し、追跡用のIssueへ追記する。
  日々の物差しをレートに置いたためである（[ADR-0207](0207-roadmap-focus-eval.md)）

**Claude Code Routines**

- `micro` の消化。毎晩、`cloud` ラベルのIssueを古い順に1件取り、PRを出す
- 巡回。週次で、固定の観点でコードと文書を読み、`micro` Issueを切る。
  観点は、文書と実装の食い違い、`hmwr --help` にない頻用の手順、重複コード、
  CIの所要時間、TODOコメントとする。観点の一覧はmicro-improvementsスキルが持つ
- Stockfishの未採用PRの調査。週次で新着のPRを読み、本エンジンで成り立つ
  理由を書けるものだけをIssueにする

RoutinesのPRは、種別が「その他」ならCIが緑で自動マージする。オーナーの
確認は挟まない（2026-09-17オーナー判断）。Routinesのブランチは `claude/`
で始まる名前に固定されるので、[ADR-0070](0070-pr-based-workflow.md)の
命名規則の例外とする。棋力が変わる変更はRoutinesで扱わない。SPRTを
回せないからである。

**開発機の定期実行**

- 実験キューの実行（下に書く）
- floodgateの棋譜の回収（`hmwr kifu fetch`、[ADR-0152](0152-floodgate-cycle.md)）
- 成果物の掃除（`hmwr clean --apply`、月次、[ADR-0189](0189-artifact-retention.md)）

開発機は常時起動を前提にする。再起動はあり得るので、どの仕事も完了印を
見て続きから走れる形にする（[ADR-0123](0123-stop-and-resume.md)）。

**対話セッション**

- 実験の設計と事前登録（ADRとspec）
- 結果の読みと次の判断、棋力が変わる実装とSPRT
- `local` ラベルの `micro`（計測が要るもの）
- 棚卸しのような、オーナーとの議論

### 層をつなぐ状態はGitHubが持つ

新しい状態ファイルは作らない。

| 状態 | 置き場 |
|---|---|
| 仕事の待ち行列 | Issueとラベル |
| 成果物 | PR |
| 判断と経緯 | ADRとROADMAP |
| 実験の中身 | `experiments/*.toml`（gitに入れる） |
| ステップの進捗 | 開発機の完了印（`data/queue/`）。要約はIssueのコメントへ書き戻す |

ラベルは次のとおりにする。`micro` は実行の場所を `cloud` か `local` で
分ける。実験は `experiment` に、状態の `queued`・`running`・`done`・`failed` を
付ける。`micro` と実験はラベルで分かれるので、CLAUDE.mdの「細かいタスクと
ロードマップの実験は分けて管理する」は保たれる。

### 実験は1件1 Issueで、中身はspecに書く

Issueは順番と状態を持つ。オーナーも順番の入れ替えや取り消しをIssueから
行える。本文の先頭にspecのパスを書く（Issueフォームで固定する）。

中身をIssueの本文に書かないのは、本文が後から書き換えられるためである。
「対立仮説は着手時に決め、走行後に変えない」（CLAUDE.md）の担保には、
gitの履歴とレビューを通る場所が要る。

specは `experiments/adrNNNN-<slug>.toml` に置き、ADRと同じPRで入れる。
**実行器はorigin/mainにあるspecだけを実行する**。事前登録のPRをマージして
から走らせる運用はいまと同じで、それが機械で強制される。固定するもの
（seed・検証集合・データ量）は、specの値がそのまま使われる。

```toml
adr = "0210"
issue = 512

[[step]]
id = "mix"
run = "hmwr data mix ctrlmix20_300M --in train_300M_q1 --in tanuki_60M_q1 --seed 1"

[[step]]
id = "train"
run = "hmwr net train ctrlmix20_300M --data ctrlmix20_300M --valid valid_385M_q1 --seed 0"

[[step]]
id = "match-normal"
run = "hmwr match run adr0210-ctrlmix20-normal --base-net pairrank_300M_q1 --cand-net ctrlmix20_300M --stop pairs:500"
```

ステップは `hmwr` のコマンド1つにする。型付きのステップやDAGは作らない。
パラメータ化は `hmwr` の側に寄せる（[ADR-0208](0208-hmwr-resource-verbs.md)）。
確実に運用されるよう、2か所で検査する。

- CIが全specの全ステップを `--dry-run` で通す。引数の誤りと、廃止された
  コマンドはPRで落ちる
- 実行器は `hmwr` で始まらないステップを拒む

### 実験キューの実行器

`hmwr queue run` をlaunchdで常駐させる。

1. `experiment` と `queued` の付いたIssueを古い順に取る。同時に走るのは
   1実験だけにする。開発機の資源は1つだからである
2. specがorigin/mainにあることを確かめ、ラベルを `running` へ付け替える
3. ステップを順に実行し、終わるたびに `data/queue/<実験>/<id>.done` を書く。
   再起動の後は、印のあるステップを飛ばす。学習とSPRTの途中からの再開は、
   各コマンドの再開の口に任せる
4. 全ステップが終わったら、結果の記録（次節）を起動し、ラベルを `done` に
   する。失敗したらログの末尾をコメントし、`failed` にして次の実験へ進む
5. `hmwr queue pause` は停止ファイルを置く。対話セッションがNPSを測るときの
   ように、開発機を空けたいときに使う。GitHubに届かないときは待って
   再試行する

### 結果の記録は `claude -p` で無人にする

道具が材料を揃え、LLMは書くだけにする。

1. `hmwr exp report <実験>` が数値の表をMarkdownで出す。`.result`・棋譜・
   学習のメタデータから作り、LLMを通さない。数値の転記の誤りが構造的に消える
2. 実行器が `claude -p` を起動し、recording-experimentスキル（新設）を
   実行させる。入力はspec・reportの出力・ADRの事前登録の節である。
   出力はPR1本で、ADRへの結果の追記・Statusと索引の更新・ROADMAPの候補の
   整理を含む
3. 事前登録に「各結果が何を意味するか」が書いてあるので、読みの大半は照合で
   済む。登録外の発見は「登録外」と明記して書き、次の判断は対話セッションへ渡す

**無人にする範囲は、当面は結果の記録（docsのPR）までとする**。評価関数の
差し替えや `feat:` のPRのような採用の操作は、対話セッションに残す。記録の
質を数件見てから広げる。

### セッションの入口

SessionStartのhookで、開いている `micro` の件数と実験Issueの状態を表示する。
優先順位の判定は要らない。仕事はそれぞれの層で進んでおり、セッションは
自分の層の仕事だけを見ればよい。

### 進める順番

1. [ADR-0208](0208-hmwr-resource-verbs.md)の部品の被覆と `match` の統合
2. `hmwr queue` とlaunchd。最初の実験は教師系列の混合の統制実験にする
3. `hmwr exp report` とrecording-experimentスキル
4. Actionsの文書整合検査と、Routinesの3本

1〜3が開発機の線、4がクラウドの線で、独立に進められる。

## Consequences

- セッションが閉じていても開発機が空かなくなる。学習3時間と対局2時間の
  実験なら、1日に4件を消化できる
- 事前登録が機械で強制される。specがmainになければ走らない
- `micro` のサイクルがセッションの気分から切り離される。消化はRoutinesが、
  発見は巡回とActionsの検査が担う
- 確かめていないことが3つある。Routinesのクラウド環境でRustとtextlintの
  ビルドが現実的な時間で通るか。`hmwr net train` が再開の口を配線して
  いるか。Routinesと `claude -p` の使用量が、サブスクリプションの上限に
  収まるか。Routinesはresearch previewでもある。毎晩1本と週次2本から始め、
  使用量を見て増減する
- 無人で走る範囲が広がるので、誤った操作の影響も広がる。mainへの直接pushの
  拒否、`data/` の削除の確認（[ADR-0181](0181-agent-surface.md)）は
  `claude -p` にも同じ設定で効かせる
- CLAUDE.mdの「オートパイロット」とmicro-improvementsスキルの発動条件を、
  実装が入る段で書き換える
- 見直しの引き金は2つある。`queued` の実験が3日以上動かないとき。
  RoutinesのPRでCIの失敗が続くとき

## 実装の記録

### 学習の完了印と再開（2026-09-18）

Consequencesに挙げた「`hmwr net train` が再開の口を配線しているか」を確かめた。
学習器は `--resume` を持ち、エポック内の位置まで戻せる
（[ADR-0159](0159-ft-width-1024.md)）。しかし `hmwr net train` は渡して
いなかった。再起動の後に同じステップを呼び直すと、最初から学習し直す形だった。

配線した。最終ネットが同じ条件で書かれていれば、何もせず成功で終わる。
`latest.ckpt` が残っていれば、そこから再開する。学習の条件は
`training/checkpoints/<名前>/train.cond` に控え、再開のたびに比べる。同じ名前で
条件が違うと止まる。対局の `.cond`（[ADR-0208](0208-hmwr-resource-verbs.md)の
段取り2）と同じ仕組みで、実装は `hmwr/conditions.py` に寄せた。

再開の再現性は、回帰だけのレシピでは確かめてある。ADR-0159の時点で、
中断して再開した学習が連続実行と全地点でビット一致した。ランキング損失の
兄弟群（[ADR-0185](0185-sibling-ranking-loss.md)）は、その後に足したもので、
再開したときの読み出し位置は確かめていない。実験の群で再開が起きたら、
結果の記録にその旨を書く。
