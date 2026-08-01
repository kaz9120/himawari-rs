# himawari-rs

Rustで書くコンピュータ将棋エンジン。USIプロトコルに対応し、将棋所やShogiGUIで
対局できる。

評価関数（NNUE）の学習から探索まで自前で実装している。2027年5月の世界コンピュータ
将棋選手権への参加を目標にしている。

- **floodgateレート 3186**（2026-07の初参戦、30局19勝11敗）
- 単一バイナリで動く。実行時の外部依存はなし
- 評価関数は19.9億局面で学習した純粋HalfKP 256x2-32-32

## 使う

### 用意するもの

エンジン本体と評価関数ファイルの2つが要る。評価関数は
[Releases](../../releases)の `net-v*` から取得する。

```sh
cargo build --release                    # target/release/himawari ができる
gh release download net-v1 -p '*.hmwr'   # 評価関数
```

定跡は任意で、`book-v*` から取得できる。

### GUIへの登録

`target/release/himawari` をエンジンとして登録し、`EvalFile` に評価関数の
パスを設定する。**`EvalFile` を設定しないと起動時にエラーで終了する**
（気づかず弱いまま対局する事故を防ぐため）。

### 主なUSIオプション

| オプション | 既定 | 説明 |
|---|---|---|
| `EvalFile` | （空） | 評価関数のパス。**必須** |
| `USI_Hash` | 256 | 置換表[MB] |
| `Threads` | 1 | 探索スレッド数 |
| `USI_Ponder` | false | 相手番の思考 |
| `BookFile` | （空） | 定跡ファイルのパス |
| `MinimumThinkingTime` | 2000 | 最小思考時間[ms] |
| `ResignValue` | 99999 | 投了する評価値の閾値（既定は無効） |
| `MultiPV` | 1 | 検討モードのライン数 |

`MinimumThinkingTime` の既定値は300秒＋10秒加算のような実戦の持ち時間を
想定している。10秒程度の短い持ち時間で使うときは小さくする。

全オプションは `usi` コマンドの出力を参照。

## ビルド

```sh
cargo build --release
```

ツールチェインは `rust-toolchain.toml` で固定している（nightly）。SIMDに
`std::simd` を使うため安定版では通らない。

計測や対局に使うビルドは `-C target-cpu=native` を付ける。

```sh
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

評価関数のFT次元は既定が256で、featureで512に切り替えられる。512は評価精度で
上回るがNPSが0.65倍に落ちるため既定では使わない
（[ADR-0067](docs/adr/0067-ft-dimension-512.md)）。

```sh
cargo build --release --features himawari-engine/ft512
```

## 開発する

### 環境構築

```sh
scripts/setup.sh           # ツールチェインとビルド
gh auth login              # Releaseの取得に要る
scripts/fetch-dataset.sh   # 教師データ（学習を回す場合のみ、77GB）
```

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

開発用ツールは `cargo run --release -p himawari-tools --bin <name>` で動く
（`perft` / `tsume` / `makenet` / `selfplay` / `psv` / `book` /
`bench` / `verify` / `profile`）。`bench`・`verify`・`profile` は評価関数の
場所を環境変数 `EVAL_FILE` から読む。`source scripts/env.sh` で入る
（[ADR-0122](docs/adr/0122-tooling-language-split.md)）。

### 学習

PyTorchで学習する。教師データはPackedSfenValue形式で、`crates/py` のPyO3拡張が
特徴抽出を担う。19.9億局面の1エポックが約2時間で回る（449,000 samples/s）。
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

設計判断はすべてADRに記録し、実装より先に書く。**棋力が変わる変更はSPRTで
H1採択したものだけをmainへ入れる。** SPRTの前に機能検証（固定深さでのノード数の
比較）と発動率の計測を行う。

バージョンはrelease-pleaseがコミットの型から算出する。`feat` がMINOR、`fix` が
PATCHで、`feat` はSPRTを通った変更にだけ使う。

## workspace構成

| ディレクトリ | 内容 |
|---|---|
| `crates/core` | 盤面表現・指し手生成・SFEN入出力（探索非依存） |
| `crates/engine` | 探索・置換表・NNUE評価・時間管理 |
| `crates/usi` | USIプロトコル層 + エンジンバイナリ `himawari` |
| `crates/tools` | 開発用ツール（perft・tsume・selfplay・makenet・psv・book） |
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
