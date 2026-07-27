# エージェント向け作業規約

himawari-rsで作業するエージェントの規約。詳細は各文書へリンクする。
文書の役割分担は [ADR-0053](docs/adr/0053-docs-structure.md) を正とする。

## 文書の役割分担

| 文書 | 軸 | 持つ情報 |
|---|---|---|
| [docs/ROADMAP.md](docs/ROADMAP.md) | 未来 | 現在地、フェーズ表、直近の残作業 |
| [docs/RESULTS.md](docs/RESULTS.md) | 時系列 | 計測・検証の1行ログ（append-only） |
| [docs/adr/](docs/adr/README.md) | 決定 | 設計判断と経緯 |
| [docs/IDEAS.md](docs/IDEAS.md) | 候補 | 1案1行の受け皿 |
| [docs/DATASETS.md](docs/DATASETS.md) | 資産 | データの所在と前処理 |
| [README.md](README.md) | 入口 | 人間向けの概要・手順 |

同じ情報を2文書に書かない。参照はリンクで行う。RESULTS.mdはappend-only。
訂正は訂正行の追記で行い、過去の行を書き換えない。

## ADRプロセス

設計判断はすべてADRに記録する（[ADR-0001](docs/adr/0001-adr-process.md)）。
proposedで起草し、オーナーLGTMでacceptedにする。1アイデア1ADR。

## SPRTゲート（[ADR-0028](docs/adr/0028-pruning-extensions.md)）

- 1機能=1SPRT。H1採択した変更だけをmainに取り込む
- 既定条件: `--tc 10+0.1 --concurrency 8 --adjudicate 2000,8`、
  elo0=0、elo1=5、α=β=0.05
- 結果（対局数、W-D-L、Elo±CI、LLR）をコミットメッセージと
  RESULTS.mdに記録する

SPRTの前に機能検証を行う（[ADR-0074](docs/adr/0074-feature-verification.md)）。
固定深さで3局面以上のノード数を変更前後で比べ、変わることを確かめる。
全局面で一致したら探索に影響していない。枝刈り・延長は発動率も測り、
0.1%を下回るならSPRTにかけない。他エンジンの係数を借りるときは、
その出典が前提とするスケールが本エンジンで成り立つかを実測する。
探索定数を足すADRには、出典・出典のスケール前提・成立の根拠を書く。

## 開発フロー（[ADR-0070](docs/adr/0070-pr-based-workflow.md)）

変更はすべてPR経由で入れる。mainへ直接pushしない。
PRテンプレートで種別を選び、種別ごとにマージ条件とバージョンが決まる。

| 種別 | 対象 | マージ条件 |
|---|---|---|
| 棋力向上 | 探索・評価関数・時間管理など、強さが変わる変更 | CIが緑、かつSPRTでH1採択 |
| その他 | リファクタ、文書、ツール、CI、依存更新 | CIが緑 |

迷う変更は「その他」に倒す。バージョンとタグはrelease-pleaseが作る
（[ADR-0071](docs/adr/0071-release-please.md)）。`Cargo.toml` は触らない。

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
