# himawari-rs

Rustで書くコンピュータ将棋エンジン。USIプロトコルに対応し、将棋所やShogiGUIで
対局できる。評価関数（NNUE）の学習から探索まで自前で実装している。

2027年5月の世界コンピュータ将棋選手権への参加を目標にしている。

- floodgateレート**3484**（562局、296勝266敗。2026-08-14時点）
- 単一バイナリで動く。実行時の外部依存はなし
- 評価関数は29.9億局面で学習した純粋HalfKP 256x2-32-32

## 使う

エンジン本体と評価関数ファイルの2つが要る。評価関数は
[Releases](../../releases)の `net-v*` から取得する。番号が最も大きいものが
最新で、古い番号のネットはそのぶん弱い。

```sh
cargo build --release                    # target/release/himawari ができる
gh release download net-v4 -p '*.hmwr'   # 評価関数（2026-08-08時点の最新）
```

定跡は任意で、`book-v*` から取得できる。

`target/release/himawari` をGUIへエンジンとして登録し、`EvalFile` に
評価関数のパスを設定する。`EvalFile` を設定しないと起動時にエラーで
終了する（気づかず弱いまま対局する事故を防ぐため）。

### 主なUSIオプション

| オプション | 既定 | 説明 |
|---|---|---|
| `EvalFile` | （空） | 評価関数のパス。必須 |
| `USI_Hash` | 256 | 置換表[MB] |
| `Threads` | 1 | 探索スレッド数 |
| `USI_Ponder` | false | 相手番の思考 |
| `BookFile` | （空） | 定跡ファイルのパス |
| `MinimumThinkingTime` | 2000 | 最小思考時間[ms] |
| `ResignValue` | 99999 | 投了する評価値の閾値（既定は無効） |
| `MultiPV` | 1 | 検討モードのライン数 |

全オプションは `usi` コマンドの出力を参照。

### つまずきやすいところ

起動直後に終了するときは、`EvalFile` が未設定かパスが違う。標準エラーへ理由を
出して終了する。

評価関数の読み込みで次のエラーが出ることがある。

```
info string error: EvalFile読み込み失敗: FT重み194がi8に収まらない。--ft-clipを付けて学習したネットが要る（ADR-0138）
```

既定のビルドはFT重みをi8で持つ（[ADR-0138](docs/adr/0138-ft-i8-quantization.md)）。
`net-v2` 以前のネットは重みが範囲に収まらず読めない。`net-v3` 以降を使うか、
`HIMAWARI_FT_I8=0` を付けてビルドする。

短い持ち時間で時間を使いすぎるときは、`MinimumThinkingTime` を小さくする。
既定の2000msは300秒＋10秒加算のような実戦の持ち時間を想定している。

ビルドが通らないときはツールチェインを確認する。SIMDに `std::simd` を使うため
安定版では通らず、`rust-toolchain.toml` が固定しているnightlyが要る。

## ビルド

```sh
cargo build --release
```

計測や対局に使うビルドは `-C target-cpu=native` を付ける
（[ADR-0003](docs/adr/0003-toolchain.md)）。

```sh
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

ネットワークの次元は環境変数 `HIMAWARI_ARCH` でビルド時に切り替える。書式は
`<FT>x<L1>[x<L2>[x<L3>]]` で、既定は `256x32x32` である
（[ADR-0127](docs/adr/0127-net-shape-bench.md)）。

```sh
HIMAWARI_ARCH=1024x32x32 cargo build --release
```

FTを太らせると評価精度は上がるが、NPSが落ちて時間制では取り返せない。
FT512は−72.8 Elo（[ADR-0067](docs/adr/0067-ft-dimension-512.md)）、FT1024は
ノード数固定で+70.6 Eloに対し10+0.1で−21.0だった
（[ADR-0159](docs/adr/0159-ft-width-1024.md)）。既定を256にしているのは
このためである。

## 開発する

### 環境構築

```sh
scripts/setup.sh               # ツールチェインとビルド
gh auth login                  # Releaseの取得に要る
scripts/fetch-dataset.sh all   # 教師データ（学習を回す場合のみ）
```

教師データは生データ116GBと加工後120GBで、空きが236GB要る。`download` /
`verify` / `prepare` に分けて実行することもできる（`-h` で確認）。

WindowsではWSL2上で動かす。macOSでも開発できる（Apple Siliconで確認している）。
判断の経緯は[ADR-0081](docs/adr/0081-portability.md)にある。

### テストとベンチ

```sh
cargo test --workspace            # テスト（debug）
cargo test --workspace --release  # perft既知値の照合込み
cargo run --release -p himawari-tools --bin bench -- <base> <cand>   # NPS計測
cargo run --release -p himawari-tools --bin verify -- <base> <cand>  # 探索の変化を固定深さで比較
scripts/sprt-run.sh <base> <cand> <名前>  # SPRTで棋力を検定
```

開発用ツールは `cargo run --release -p himawari-tools --bin <name>` で動く。

| ツール | 用途 |
|---|---|
| `perft` / `tsume` | 指し手生成と詰み探索の検証 |
| `bench` / `verify` / `profile` | NPS計測・探索の差分比較・プロファイル |
| `selfplay` / `league` | SPRTの対局と総当たりリーグ戦 |
| `gensfen` / `psv` | 教師データの生成と加工 |
| `makenet` | 評価関数ファイルの生成 |
| `book` | 定跡の生成と統計 |
| `kifu` | floodgate棋譜の分析（[ADR-0152](docs/adr/0152-floodgate-cycle.md)） |

`bench`・`verify`・`profile` は評価関数の場所を環境変数 `EVAL_FILE` から読む。
`source scripts/env.sh` で入る（[ADR-0122](docs/adr/0122-tooling-language-split.md)）。

### 学習

PyTorchで学習する。教師データはPackedSfenValue形式で、`crates/py` のPyO3拡張が
特徴抽出を担う。29.9億局面の1エポックが約8.6時間で回る（96,000局面/秒）。
データの所在と前処理は[docs/DATASETS.md](docs/DATASETS.md)にある。

### 開発の進め方

GitHub Issuesは使わない。状況・設計・手順のすべてをリポジトリ内の文書で管理する。

| 文書 | 読むとき |
|---|---|
| [docs/ROADMAP.md](docs/ROADMAP.md) | 次に何をするか知りたいとき |
| [docs/adr/README.md](docs/adr/README.md) | なぜそうしたか知りたいとき |
| [docs/DATASETS.md](docs/DATASETS.md) | 教師データを扱うとき |
| [CHANGELOG.md](CHANGELOG.md) | 何が入ったか見るとき |
| [CLAUDE.md](CLAUDE.md) | エージェントが作業するとき |

設計判断はすべてADRに記録し、実装より先に書く。棋力が変わる変更はSPRTで
H1採択したものだけをmainへ入れる。SPRTの前に機能検証（固定深さでのノード数の
比較）と発動率の計測を行う。

バージョンはrelease-pleaseがコミットの型から算出する。`feat` がMINOR、`fix` が
PATCHで、`feat` はSPRTを通った変更にだけ使う。

## workspace構成

| ディレクトリ | 内容 |
|---|---|
| `crates/core` | 盤面表現・指し手生成・SFEN入出力（探索非依存） |
| `crates/engine` | 探索・置換表・NNUE評価・時間管理 |
| `crates/usi` | USIプロトコル層 + エンジンバイナリ `himawari` |
| `crates/tools` | 開発用ツール |
| `crates/py` | PyO3拡張（特徴抽出・.hmwr I/OをPythonに公開） |
| `training/` | PyTorch学習器 |

## 謝辞

探索部は[やねうら王](https://github.com/yaneurao/YaneuraOu)を参照実装として
機能差分を埋めている（[ADR-0109](docs/adr/0109-reference-parity.md)）。やねうら王は
[Stockfish](https://github.com/official-stockfish/Stockfish)の探索技法を将棋へ
移植したもので、本エンジンはその系譜に連なる。移植したファイルには冒頭に由来を
書いている。

[Chess Programming Wiki](https://www.chessprogramming.org/Main_Page)も参考にした。

## License

GPL-3.0-or-later

Copyright (C) 2026 Kazumasa Yamamoto

v0.16.2まではMITで配布した。GPLv3への変更の経緯は
[ADR-0108](docs/adr/0108-license-gplv3.md)にある。既存のタグはMITのまま変わらない。

学習済みネットと定跡データはプログラムの出力物であり、このライセンスの対象外と
する。配布条件は[ADR-0080](docs/adr/0080-net-release.md)と
[ADR-0082](docs/adr/0082-book-release.md)を参照。
