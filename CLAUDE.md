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

## コミット規律

- 1コミット=単一の論理変更
- コミット前に `cargo fmt` を必須とする（CIが `rustfmt --check` を強制）

## ビルド

計測・対局は `-C target-cpu=native` で行う（[ADR-0003](docs/adr/0003-toolchain.md)）。

```
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

## data/ の配置（[ADR-0053](docs/adr/0053-docs-structure.md)）

すべてgitignore対象。`data/raw/<データセット名>/` に生データ、
`data/train/` に加工済みpsv、`data/nets/` に学習済みネット、
`data/sprt/` にselfplayの棋譜ログ（jsonl）を置く。
チェックポイント（*.pt）は `training/checkpoints/` に置く。
ネットのファイル名は実験名を含める（例: `halfkp_180M.hmwr.best`）。
