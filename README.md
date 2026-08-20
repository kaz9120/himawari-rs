<!--
この文書の読み手と範囲はADR-0182で決めた。読み手は2人いる。エンジンを対局に
使う人と、このリポジトリで開発する人である。棋力・ネットの世代・Elo・既定の
構成値のように変わり続ける事実はここへ書かず、正の場所（Releases・
docs/ROADMAP.md・docs/adr/・コード）を指す。
-->

# himawari-rs

Rustで書くコンピュータ将棋エンジン。USIプロトコルに対応し、将棋所やShogiGUIで
対局できる。探索・評価関数（NNUE）・学習器を自前で実装している。実行時の外部
依存はなく、エンジン本体と評価関数ファイルの2つで動く。

2027年5月の世界コンピュータ将棋選手権への参加を目標にしている。floodgateで
継続して対局しており、現在の棋力と開発の現在地は
[docs/ROADMAP.md](docs/ROADMAP.md)にある。

- 対局に使う人は「[使う](#使う)」を読む。ビルドは要らない
- コードを触る人は「[開発する](#開発する)」を読む

## 使う

### 用意する

エンジン本体と評価関数ファイルを[Releases](../../releases)から取る。

エンジン本体は `v*` タグの最新から、環境に合うzipを選ぶ。

| ファイル | 環境 |
|---|---|
| `himawari-<版>-windows-x64-avx2.zip` | Windows。AVX2を持つCPU |
| `himawari-<版>-windows-x64-sse42.zip` | Windows。AVX2を持たないCPU |
| `himawari-<版>-linux-x64-avx2.zip` | Linux。AVX2を持つCPU |
| `himawari-<版>-linux-x64-sse42.zip` | Linux。AVX2を持たないCPU |
| `himawari-<版>-macos-arm64.zip` | macOS（Apple Silicon） |

評価関数は `net-v*` タグのうち、番号のいちばん大きいものから `.hmwr` を取る。
古い番号のネットはそのぶん弱い。

コマンドで取るなら次のようにする。

```sh
gh release list                              # v* と net-v* の最新を確認する
gh release download <最新のv*> -p '*macos-arm64.zip'
gh release download <最新のnet-v*> -p '*.hmwr'
```

**エンジンと評価関数は最新どうしを組む**。世代の離れた組はファイル形式の版が
合わず、読み込みでエラーになる。

定跡は任意で、`book-v*` から取れる。USIオプション `BookFile` にパスを渡す。

### GUIへ登録する

解凍した `himawari`（Windowsは `himawari.exe`）をGUIへエンジンとして登録し、
USIオプション `EvalFile` に評価関数のパスを設定する。

**`EvalFile` の設定を忘れない**。未設定でも起動するが、駒割だけで指すため
極端に弱くなる。

### 主なUSIオプション

| オプション | 既定 | 説明 |
|---|---|---|
| `EvalFile` | （空） | 評価関数のパス。必須 |
| `BookFile` | （空） | 定跡ファイルのパス |
| `USI_Hash` | 256 | 置換表[MB] |
| `Threads` | 1 | 探索スレッド数 |
| `USI_Ponder` | false | 相手番の思考 |
| `MinimumThinkingTime` | 2000 | 最小思考時間[ms] |
| `MultiPV` | 1 | 検討モードのライン数 |

全オプションと値域は、エンジンへ `usi` と入力したときの出力が正になる。

### うまく動かないとき

エンジンの異常はGUIのログへ `info string error:` の行で出る。まずそこを読む。

起動直後に終了するときは、`EvalFile` のパスが違う。エンジンはパスと現在位置を
併記して終了するので、手元のファイルと突き合わせる。

```
info string error: EvalFileを開けません: No such file or directory (os error 2)
info string   path = "/tmp/net.hmwr" (14文字 14バイト)
info string   cwd  = "/home/user/shogi"
```

`EvalFile読み込み失敗` で終了するときは、エンジンと評価関数の組が合っていない。
両方を最新にすると直る。

短い持ち時間で時間を使いすぎるときは、`MinimumThinkingTime` を小さくする。
既定の2000msは、300秒＋10秒加算のような実戦の持ち時間を前提にしている。

## 開発する

### 環境構築

```sh
scripts/setup.sh               # ツールチェイン・Python・ビルド・テスト
export PATH="$PWD/bin:$PATH"   # hmwr コマンドへパスを通す
gh auth login                  # Releaseの取得に要る
hmwr data fetch all            # 教師データ（学習を回す場合のみ）
```

`scripts/setup.sh` はLinuxとWSL2向けである。macOSでも開発できるが、ツールは
個別に入れる（Apple Siliconで確認している）。

教師データは生データと加工後の合計で220GBを超える。内訳と前処理は
[docs/DATASETS.md](docs/DATASETS.md)にある。`hmwr data fetch` は
download・verify・prepareへ分けて実行もできる。

### ビルド

SIMDに `std::simd` を使うため、安定版のRustでは通らない。`rust-toolchain.toml`
が固定するnightlyを、rustupが自動で入れる。

```sh
cargo build --release                                    # target/release/himawari
RUSTFLAGS="-C target-cpu=native" cargo build --release   # 計測・対局用
```

計測と対局には `-C target-cpu=native` を付ける。配布用の単体ビルドはPGOで作り、
`hmwr build pgo` が手順を持つ。Releasesのバイナリも同じ手順で作っている。

ネットワークの次元は環境変数 `HIMAWARI_ARCH` でビルド時に切り替える。書式は
`<FT>x<L1>[x<L2>[x<L3>]]` で、既定値は `crates/engine/build.rs` の
`DEFAULT_ARCH` にある。

```sh
HIMAWARI_ARCH=256x32x32 cargo build --release
```

**バイナリと評価ファイルは対で使う**。次元やファイル形式の版が食い違うと
読み込みで落ちる。既定のビルドへ渡すネットは
[docs/ROADMAP.md](docs/ROADMAP.md)の現行構成にある。

### 日常操作

`hmwr` コマンドが入口になる。ビルド・測定・学習・データ処理・文書のlintが
ここから動く。

```sh
hmwr --help                         全体を見る
hmwr --dry-run <...>                走るはずのコマンドを表示する
hmwr env                            並列度・評価関数・持ち時間の既定
hmwr sprt run <名前>                ペア作成→機能検証→SPRT起動
hmwr sprt show <名前>               途中経過・結果
hmwr verify <名前>                  固定深さで探索の変化を比べる
hmwr bench <base> <cand>            NPSを交互に測る
hmwr net train <名前> --data <psv>  ネットを学習する
hmwr doc lint                       日本語文書のlint
```

オプションはフラグで渡す。ログの置き場（`data/logs/<領域>-<名前>.log`）は
CLIが決めるので、リダイレクト先を書かない。

### テストと検査

CIと同じ検査をローカルで通せる。

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace            # テスト（debug）
cargo test --workspace --release  # perft既知値の照合込み
python3 -m pytest tests -q        # hmwr のテスト
hmwr doc lint                     # 日本語文書のlint
```

開発用ツールは `cargo run --release -p himawari-tools --bin <name>` でも動く。

| ツール | 用途 |
|---|---|
| `perft` / `tsume` | 指し手生成と詰み探索の検証 |
| `bench` / `verify` / `profile` | NPS計測・探索の差分比較・プロファイル |
| `selfplay` / `league` | SPRTの対局と総当たりリーグ戦 |
| `gensfen` / `psv` | 教師データの生成と加工 |
| `makenet` | 評価関数ファイルの生成 |
| `book` | 定跡の生成と統計 |
| `kifu` | floodgate棋譜の回収と分析 |

`bench`・`verify`・`profile` は評価関数の場所を環境変数 `EVAL_FILE` から読む。
`hmwr` 経由で呼ぶときは指定が要らない。

### 学習

PyTorchで学習する。教師データはPackedSfenValue形式で、`crates/py` のPyO3拡張が
特徴抽出を担う。入口は `hmwr net train` である。データの所在と前処理は
[docs/DATASETS.md](docs/DATASETS.md)にある。

### 進め方

GitHub Issuesは使わない。状況・設計・手順のすべてをリポジトリ内の文書で管理する。

| 文書 | 読むとき |
|---|---|
| [docs/ROADMAP.md](docs/ROADMAP.md) | 次に何をするか知りたいとき |
| [docs/adr/README.md](docs/adr/README.md) | なぜそうしたか知りたいとき |
| [docs/DATASETS.md](docs/DATASETS.md) | 教師データを扱うとき |
| [CHANGELOG.md](CHANGELOG.md) | 何が入ったか見るとき |
| [CLAUDE.md](CLAUDE.md) | 作業の規約を確認するとき |

設計判断はすべてADRに記録し、実装より先に書く。棋力が変わる変更は、SPRTで
H1採択したものだけをmainへ入れる。変更はPR経由で入れ、バージョンとCHANGELOGは
release-pleaseがコミットの型から作る。

## workspace構成

| ディレクトリ | 内容 |
|---|---|
| `crates/core` | 盤面表現・指し手生成・SFEN入出力（探索非依存） |
| `crates/engine` | 探索・置換表・NNUE評価・時間管理 |
| `crates/usi` | USIプロトコル層 + エンジンバイナリ `himawari` |
| `crates/tools` | 開発用ツール |
| `crates/py` | PyO3拡張（特徴抽出・.hmwr I/OをPythonに公開） |
| `training/` | PyTorch学習器 |
| `hmwr/` | 開発コマンド `hmwr` の実装 |

## 謝辞

探索部は[やねうら王](https://github.com/yaneurao/YaneuraOu)を参照実装として
機能差分を埋めている。やねうら王は
[Stockfish](https://github.com/official-stockfish/Stockfish)の探索技法を将棋へ
移植したもので、本エンジンはその系譜に連なる。移植したファイルには冒頭に由来を
書いている。

[Chess Programming Wiki](https://www.chessprogramming.org/Main_Page)も参考にした。

## License

GPL-3.0-or-later

Copyright (C) 2026 Kazumasa Yamamoto

v0.16.2まではMITで配布した。既存のタグはMITのまま変わらない。

学習済みネットと定跡データはプログラムの出力物であり、このライセンスの対象外と
する。
