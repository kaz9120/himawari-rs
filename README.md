# himawari-rs

Rustで書くコンピュータ将棋エンジン（USIプロトコル対応）。

評価関数はNNUE、探索はalpha-beta＋Lazy SMPを最終形とし、
自己対局による教師データ生成と学習器まで自前で実装する。
参考: [Stockfish](https://github.com/official-stockfish/stockfish)、
[やねうら王](https://github.com/yaneurao/yaneuraou)、
[Chess Programming Wiki](https://www.chessprogramming.org/Main_Page)

## ドキュメント構成

このプロジェクトはGitHub Issuesを使わない。やりたいこと・設計・状況の
すべてをリポジトリ内の文書で管理する。

| 文書 | 役割 |
|---|---|
| [docs/ROADMAP.md](docs/ROADMAP.md) | 現在地・フェーズ進捗・残作業（状況はここが正） |
| [docs/RESULTS.md](docs/RESULTS.md) | 計測・検証結果の時系列ログ（append-only） |
| [docs/adr/README.md](docs/adr/README.md) | 設計判断の索引と未起草バックログ（設計はここが正） |
| docs/adr/NNNN-*.md | 個々の設計判断（ADR） |
| [docs/IDEAS.md](docs/IDEAS.md) | 改善アイデア帳 |
| [docs/DATASETS.md](docs/DATASETS.md) | 教師データの所在と前処理手順 |
| [CLAUDE.md](CLAUDE.md) | エージェント向けの作業規約（ゲート運用・コミット規律） |

## 開発プロセス

設計判断はすべてADRとして記録し、実装より先に書く。
「ADR群を書く→実装→検証」をフェーズ単位で繰り返し、
各フェーズの出口条件（perft一致、対局完走など）を通過してから次へ進む。

## ビルドと使い方

```
cargo build --release
```

`target/release/himawari` がUSIエンジン本体。将棋所やShogiGUIに
エンジン登録して対局できる。

FT次元は既定が256で、featureで512に切り替えられる（ADR-0067）。
512は評価精度で上回るがNPSが0.65倍に落ちるため、既定では使わない。

```
cargo build --release --features himawari-engine/ft512
```

開発用ツールは次のとおり。

```
cargo run --release -p himawari-tools --bin perft -- 5   # perft
cargo run --release -p himawari-tools --bin tsume        # 詰将棋スモーク
cargo run --release -p himawari-tools --bin makenet      # 乱数NNUEネット生成
cargo test --workspace                                   # テスト（debug）
cargo test --workspace --release                         # perft既知値の照合込み
```

自己対局・SPRT検定は `selfplay`（使い方は
`crates/tools/src/bin/selfplay/main.rs` 冒頭のコメント）。

ツールチェインは `rust-toolchain.toml` で固定している（nightly）。

## 学習

学習器はPyTorch（ADR-0040）。PSVデコードと特徴抽出はRustの
PyO3拡張モジュール経由で呼ぶ（ADR-0043）。教師データの
詳細は [docs/DATASETS.md](docs/DATASETS.md) を参照。

```bash
# 初回セットアップ（maturin developはvirtualenvを要求するのでbuild+installを使う）
pip install torch tensorboard maturin
cd crates/py && maturin build --release && cd ../..
pip install --force-reinstall --no-deps target/wheels/himawari-*.whl

# 教師データの前処理。shuffleは2パスのバケット法で、RAMに載らない規模も通る
cargo run --release --bin psv -- shuffle --in data/raw/hao_depth9/000.bin,... --out data/train/train.psv
cargo run --release --bin psv -- head --in data/raw/hao_depth9/023.bin --out data/train/valid.psv --count 200000

# 学習（純粋HalfKP + factorizer、MPS）
cd training
python3 train.py --data ../data/train/train.psv --valid ../data/train/valid.psv \
  --out ../data/nets/net.hmwr --epochs 1 --batch 16384 \
  --peak-lr 1e-3 --warmup-steps 100 --lambda 0.7 \
  --device mps --dense-ft --batch-loader --factorized

# 棋力検証（SPRT）
cargo run --release --bin selfplay -- \
  --baseline target/release/himawari \
  --candidate target/release/himawari \
  --copt "EvalFile=data/nets/net.hmwr.best" \
  --tc 10+0.1 --concurrency 8 \
  --openings openings/start_sfens_ply24.txt
```

学習の主なフラグは次のとおり。

| フラグ | 意味 |
|---|---|
| `--device mps` `--dense-ft` | FT勾配をdenseにしてGPUで回す（ADR-0064）。現行の5.5倍 |
| `--batch-loader` | バッチ一括抽出のローダを使う（ADR-0065） |
| `--factorized` | 学習時だけ駒単独の仮想特徴を併用する（ADR-0066）。+28.1 Elo |
| `--mmap` | 学習データをmmapで開く。RAMに載らない規模で使う |

学習側のFT次元はPyO3モジュールが公開する定数から読む。512で学習する
ときは `maturin build --release --features ft512` で作ったモジュールを
入れる（ADR-0067）。

## workspace構成

| ディレクトリ | 内容 |
|---|---|
| `crates/core` | 盤面表現・指し手生成・SFEN入出力（探索非依存） |
| `crates/engine` | 探索・置換表・NNUE評価・時間管理 |
| `crates/usi` | USIプロトコル層 + エンジンバイナリ `himawari` |
| `crates/tools` | 開発用ツール（perft・tsume・selfplay・makenet・psv・train） |
| `crates/py` | PyO3拡張モジュール（特徴抽出・.hmwr I/OをPythonに公開） |
| `training/` | PyTorch学習器（モデル定義・データセット・学習ループ・量子化） |

## License

MIT
