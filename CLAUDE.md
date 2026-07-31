# エージェント向け作業規約

himawari-rsで作業するエージェントの規約。詳細は各文書へリンクする。
文書の役割分担は [ADR-0053](docs/adr/0053-docs-structure.md) を正とする。

## 前提: オートパイロットで進める

このリポジトリはエージェントが主体で進める。オーナーが求めるのは
**判断の記録が残ることと、棋力が上がること**の2つで、経過の逐一の
確認ではない（2026-07-29オーナー指示）。

- 方向を選ぶ場面で止まらない。根拠を書いて選び、外れたら棄却として
  記録する。棄却の記録も成果物である
- CIが緑のPRは自分でマージする。マージ条件は下の「開発フロー」の表に
  従う。棋力向上はSPRTでH1採択も要る
- ADRのacceptedもオーナーの返事を待たずに進めてよい。判断の根拠が
  ADRに残っていることが条件になる
- 確認するのは、破壊的で戻せない操作だけ。force push、履歴の書き換え、
  `data/` の削除など

止まって聞くより、測って決めて記録するほうが速い。効かなかった案の
記録は次の判断材料になるので、必ず残す（[ADR-0102](docs/adr/0102-move-horizon.md)・
[ADR-0103](docs/adr/0103-root-score-gap.md)が例）。

## 文書の役割分担

| 文書 | 軸 | 持つ情報 |
|---|---|---|
| [docs/ROADMAP.md](docs/ROADMAP.md) | 現在地と方向 | 今どこにいて、次にどちらへ向かうか |
| [docs/RESULTS.md](docs/RESULTS.md) | 時系列 | 計測・検証の1行ログ（append-only） |
| [docs/adr/](docs/adr/README.md) | 決定 | 設計判断と経緯 |
| [docs/IDEAS.md](docs/IDEAS.md) | 候補 | 未着手の案を1案1行で |
| [docs/DATASETS.md](docs/DATASETS.md) | 資産 | データの所在と前処理 |
| [README.md](README.md) | 入口 | 人間向けの概要・手順 |

同じ情報を2文書に書かない。参照はリンクで行う。

**案は IDEAS → ADR → 完了 の順に動く。** IDEAS.mdは候補の在庫で、着手を決めたら
ADRを起こしてIDEAS.mdから消す。完了・棄却した案もIDEAS.mdには残さない。
**ROADMAPは候補を列挙しない。** 「今どこにいて、次にどちらへ向かうか」だけを書き、
具体的な候補はIDEAS.mdへのリンクで示す。この3文書の役割が重なったら、
ROADMAPから候補を削ってIDEAS.mdへ寄せる。

**過去の意思決定はADRが持つ。** ROADMAPの過去の結論が後の測定で覆ったら、
訂正を追記して現在地を更新する。経緯を追うための記録はROADMAPの「記録」節へ
置き、現在の判断に要る結論は「次の方向」へ上げる。

RESULTS.mdだけはappend-onlyで、訂正も追記で行い過去の行を書き換えない。
時系列の記録が「いつ何を知っていたか」を示すためである。

## ADRプロセス

設計判断はすべてADRに記録する（[ADR-0001](docs/adr/0001-adr-process.md)）。
proposedで起草し、オーナーLGTMでacceptedにする。1アイデア1ADR。

## SPRTゲート（[ADR-0028](docs/adr/0028-pruning-extensions.md)）

- H1採択した変更だけをmainに取り込む
- 単発の変更は1機能=1SPRT。**参照実装への追従は1群=1SPRT**
  （[ADR-0109](docs/adr/0109-reference-parity.md)）
- 既定条件: `--tc 10+0.1 --concurrency 8 --adjudicate 2000,8`、
  elo0=0、elo1=5、α=β=0.05
- 結果（対局数、W-D-L、Elo±CI、LLR）をコミットメッセージと
  RESULTS.mdに記録する

**既定条件で測れない変更がある。** 時間管理は参照実装の既定値が実戦の持ち時間
（floodgateの300+10）を前提にしており、10+0.1では床が配分を支配する。条件を
変えて測るか、非劣性検定（elo0=-5、elo1=0）へ落とす。条件を変えたら、なぜ既定で
測れないかをADRに書く（[ADR-0116](docs/adr/0116-g7-timeman.md)が例）。

SPRTの前に機能検証を行う（[ADR-0074](docs/adr/0074-feature-verification.md)）。
固定深さで3局面以上のノード数を変更前後で比べ、変わることを確かめる。
全局面で一致したら探索に影響していない。枝刈り・延長は発動率も測り、
0.1%を下回るならSPRTにかけない。

他エンジンから移すときは、**その値が何に支えられているかまで移す。** 確かめる
のは3つある。係数のスケールが本エンジンで成り立つか、発動頻度の設計点が同じか、
その値を守っている別の仕組みがないか。3つ目を落として失敗した例が2件ある
（[ADR-0116](docs/adr/0116-g7-timeman.md)・[ADR-0118](docs/adr/0118-g9-aspiration.md)）。
探索定数を足すADRには、出典・前提・成立の根拠を書く。

## 開発フロー（[ADR-0070](docs/adr/0070-pr-based-workflow.md)）

変更はすべてPR経由で入れる。mainへ直接pushしない。
PRテンプレートで種別を選び、種別ごとにマージ条件とバージョンが決まる。

| 種別 | 対象 | マージ条件 |
|---|---|---|
| 棋力向上 | 探索・評価関数・時間管理など、強さが変わる変更 | CIが緑、かつSPRTでH1採択 |
| その他 | リファクタ、文書、ツール、CI、依存更新 | CIが緑 |

迷う変更は「その他」に倒す。バージョンとタグはrelease-pleaseが作る
（[ADR-0071](docs/adr/0071-release-please.md)）。`Cargo.toml` は触らない。

ブランチはorigin/mainから切り、1ブランチ1PRとする。マージ済みブランチは
再利用しない。名前は着手時に `<型>-adrNNNN-<slug>` で決め、途中で改名
しない。rebaseで競合したら解決を終えてから次の操作へ移る（競合の途中で
ビルドや計測をしない）。詳細は[ADR-0070](docs/adr/0070-pr-based-workflow.md)
の「ブランチ運用」にある。

PRの作成はテンプレートを明示して行う。

```
gh pr create --template strength.md   # 棋力向上
gh pr create --template chore.md      # その他
```

## バージョニング（[ADR-0068](docs/adr/0068-sprt-driven-versioning.md)・[ADR-0071](docs/adr/0071-release-please.md)）

コミットの型からrelease-pleaseが算出する。

| 型 | 対象 | bump |
|---|---|---|
| `feat` | SPRTでH1採択した変更 | MINOR |
| `fix` | 棋力に影響しないコードの変更 | PATCH |
| `docs` | 文書のみの変更 | なし |
| `chore` | CI・設定・テスト・依存更新 | なし |

判断はバイナリが変わるか、棋力が変わるかの2問で決まる。
MAJOR（選手権への参加。次回2027年5月を1.0.0）は
`Release-As: 1.0.0` トレーラで明示する。

## コミット規律

- 1コミット=単一の論理変更
- 件名はConventional Commitsの型で始める（`feat:` / `fix:` / `docs:` /
  `chore:`）。本文は日本語の平叙文のままでよい
- 棋力向上のコミットは件名にEloを入れる。CHANGELOGに数値が残る
  （例: `feat: razoringを導入する（+184.8 Elo、ADR-0057）`）
- コミット前に `cargo fmt` を必須とする（CIが `rustfmt --check` を強制）
- 棋力が変わる変更には `SPRT:` トレーラを付ける。書式は
  `SPRT: <Elo> [<CI下限>,<CI上限>] <対局数>games <H0|H1>`。
  `Co-Authored-By` と同じ位置に書く。RESULTS.mdへの転記元になる

## ビルド

計測・対局は `-C target-cpu=native` で行う（[ADR-0003](docs/adr/0003-toolchain.md)）。

```
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

## data/ の配置（[ADR-0053](docs/adr/0053-docs-structure.md)）

すべてgitignore対象。`data/raw/<データセット名>/` に生データ、
`data/train/` に加工済みpsv、`data/nets/` に学習済みネット、
`data/sprt/` にselfplayの棋譜ログ（jsonl）、`data/bin/` に
比較用に残すビルド済みバイナリを置く。
チェックポイント（*.pt）は `training/checkpoints/` に置く。
ネットのファイル名は実験名を含める（例: `halfkp_180M.hmwr.best`）。
