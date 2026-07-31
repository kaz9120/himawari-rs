# himawari-rs

Rustで書くコンピュータ将棋エンジン（USIプロトコル対応）。

評価関数はNNUE、探索はalpha-beta＋Lazy SMPで、自己対局による教師データ生成と
学習器まで自前で実装する。探索部は[やねうら王](https://github.com/yaneurao/YaneuraOu)を
参照実装として機能差分を埋めた（[ADR-0109](docs/adr/0109-reference-parity.md)）。
参考: [Stockfish](https://github.com/official-stockfish/stockfish)、
[Chess Programming Wiki](https://www.chessprogramming.org/Main_Page)

## ドキュメント構成

GitHub Issuesは使わない。状況・設計・手順のすべてをリポジトリ内の文書で管理する。
**文書は「誰がいつ読むか」で決め、読み手のいない文書は作らない。**

| 文書 | 読むとき |
|---|---|
| [docs/ROADMAP.md](docs/ROADMAP.md) | 次に何をするか知りたいとき（現行構成・方向・候補） |
| [docs/adr/README.md](docs/adr/README.md) | なぜそうしたか知りたいとき（設計判断の索引） |
| [docs/DATASETS.md](docs/DATASETS.md) | 教師データを扱うとき |
| [CHANGELOG.md](CHANGELOG.md) | 何が入ったか見るとき（release-pleaseが生成） |
| [CLAUDE.md](CLAUDE.md) | エージェントが作業するとき（規約） |
| [.claude/skills/](.claude/skills/) | 特定の作業手順が要るとき |

## 開発プロセス

設計判断はすべてADRとして記録し、実装より先に書く。**過去の意思決定はADRが
持ち、ROADMAPとROADMAPの候補には今と未来の判断に要る情報だけを置く。**

棋力が変わる変更はSPRTでH1採択したものだけをmainへ入れる
（[ADR-0028](docs/adr/0028-pruning-extensions.md)）。単発の変更は1機能=1SPRT、
参照実装への追従は1群=1SPRTで測る（[ADR-0109](docs/adr/0109-reference-parity.md)）。
SPRTの前に機能検証（固定深さでのノード数の比較）と発動率の計測を行う
（[ADR-0074](docs/adr/0074-feature-verification.md)）。

フェーズ管理は[ADR-0068](docs/adr/0068-sprt-driven-versioning.md)で終え、
現在はSPRT採択を単位に進めている。

## 環境構築

新しいマシンで開発を引き継ぐ手順。設計判断の経緯は
[ADR-0081](docs/adr/0081-portability.md)、ネットの配布は
[ADR-0080](docs/adr/0080-net-release.md) にある。

### 全体の流れ

```
1. WSL2 を入れる（Windowsの場合）
2. scripts/setup.sh        ツールチェインとビルド
3. gh auth login           GitHubの認証
4. 学習済みネットの取得     gh release download net-v<N>
5. 定跡の取得（任意）       gh release download book-v<N>
6. scripts/fetch-dataset.sh 教師データ（学習を回す場合のみ）
```

所要は2までが15分、4が数分、5が回線次第（77GB）。

### 1. WSL2（Windowsの場合）

管理者権限のPowerShellで実行する。

```powershell
wsl --install -d Ubuntu-24.04
```

再起動後、Ubuntuを起動してユーザーを作る。

リポジトリはWSL2のファイルシステム内（`~/` 配下）に置く。
`/mnt/c/` 配下はファイルI/Oが桁で遅く、ビルドと教師データの
読み込みが実用にならない。

### 24時間稼働の設定

Windows側で3つ設定する。

電源オプションでスリープと休止を無効にする。ノートPCやタブレットは
「電源接続時」の設定を変える。

Windows Update のアクティブ時間を設定するか、更新を一時停止する。
自動再起動で長時間のSPRTや学習が落ちる。

`%UserProfile%\.wslconfig` でWSL2への割当を決める。

```ini
[wsl2]
memory=24GB
processors=10
swap=8GB
```

`processors` は論理プロセッサ数から余裕を引いた値にする。
`memory` は物理RAMの7割程度が目安。教師データはmmapで読むため、
ページキャッシュに使える余地を残す。

### 2. ツールチェインとビルド

```bash
git clone https://github.com/kaz9120/himawari-rs.git
cd himawari-rs
scripts/setup.sh
```

入るもの: build-essential、git、gh CLI、rustup と
`rust-toolchain.toml` が指すnightly、Python3 と PyTorch（`.venv` 配下）。

PyTorchが不要なら `SKIP_PYTHON=1 scripts/setup.sh` で飛ばせる。
探索の開発だけならPythonは要らない。

最後にリリースビルドとテストが走る。テストが105件通れば完了。

### 3. GitHubの認証

```bash
gh auth login
```

ネットの取得（`gh release download`）とPRの作成に要る。

### 4. 学習済みネット

リポジトリには含めない（[ADR-0080](docs/adr/0080-net-release.md)）。
GitHub Release から取る。

```bash
gh release list | grep net-v
gh release download net-v<N> -D data/nets/
```

現行の最強構成は [docs/ROADMAP.md](docs/ROADMAP.md) の「現行の最強構成」を見る。

取得したら動作を確かめる。

```bash
printf 'usi\nsetoption name EvalFile value data/nets/<ネット>\nisready\nquit\n' \
  | ./target/release/himawari
```

`info string EvalFile loaded: ...` に学習来歴が出れば正しく読めている。

### 5. 定跡（任意）

リポジトリには含めない（[ADR-0082](docs/adr/0082-book-release.md)）。
生成が非決定的なため再現できず、成果物をGitHub Releaseで配る。

```bash
gh release list | grep book-v
gh release download book-v<N> -D data/book/
```

USIオプション `BookFile` にパスを指定する。既定は定跡なしで、
指定しなければ使われない。

### 6. 教師データ（学習を回す場合のみ）

約160GBの空きが要る（生データ77GB + 加工済み80GB）。
探索の開発だけなら不要。

```bash
scripts/fetch-dataset.sh all
```

`download` → `verify` → `prepare` を順に実行する。中断しても
`download` は再実行できる（妥当なサイズのファイルは飛ばす）。

`prepare` は検証データの切り出しと全体シャッフルを行う。
シャッフルは2パスのバケット法で、RAMに載らない規模でも通る
（[ADR-0065](docs/adr/0065-large-scale-dataloader.md)）。

データの詳細は [docs/DATASETS.md](docs/DATASETS.md) を参照。

### マシンに合わせる設定

`scripts/env.sh` が自動で決める。

| 項目 | 決め方 |
|---|---|
| SPRTの並列度 | 物理コア数-1、ただし8が上限（ADR-0028の既定に合わせる） |
| 評価関数 | `data/nets/halfkp_1900M_fact.hmwr.best` |
| 持ち時間 | 10+0.1（ADR-0028の既定） |

現在の値は次で確認できる。

```bash
bash -c 'source scripts/env.sh && env_summary'
```

環境変数で上書きできる。

```bash
SPRT_CONCURRENCY=4 scripts/sprt.sh base cand name
```

### 並列度は実機で確かめる

物理コア数から機械的に決めているが、コアの性質が揃っていないマシンでは
調整が要る。高性能コアと効率コアが混在する構成（Intel の P/E コア、
Apple Silicon）では、効率コアに載った対局だけ探索が浅くなり、
測定がぶれる。

判断の目安は、同じ局面・同じ深さで測ったNPSが並列度を上げても
極端に落ちないことである。

```bash
# 1スレッドのNPS
printf 'usi\nsetoption name EvalFile value <ネット>\nisready\nusinewgame\nposition startpos\ngo depth 14\n' \
  | ./target/release/himawari | grep 'depth 14 '
```

### SPRTの回し方

```bash
scripts/sprt.sh <baselineバイナリ> <candidateバイナリ> <名前>
```

条件は `env.sh` の既定（[ADR-0028](docs/adr/0028-pruning-extensions.md)）を使う。
棋譜は `data/sprt/<名前>.jsonl` へ書く。

SPRTの前に機能検証を済ませる（[ADR-0074](docs/adr/0074-feature-verification.md)）。
固定深さでノード数が変わらない変更は、SPRTへ持ち込んでも中立にしかならない。

### floodgate参加（Windows）

WSL2内のバイナリをWindowsのGUI（ShogiGUI・将棋所）から直接叩くのは
面倒なため、GitHub Releaseのバイナリを使う。

```powershell
gh release download v<バージョン> -p "*windows-x64-avx2.zip"
```

`release.yml` がタグpushのたびにAVX2とSSE4.2の2種類をビルドしている。
CPUがAVX2に対応していればavx2版を使う。

評価関数は別途 `net-v<N>` から取り、GUIのエンジン設定で `EvalFile` に
絶対パスを指定する。

### macOSで開発する場合

`scripts/setup.sh` はLinux専用である。macOSでは個別に入れる。

```bash
brew install gh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
python3 -m venv .venv && source .venv/bin/activate
pip install -r training/requirements.txt
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

学習にMPSを使う場合は `--device mps` を指定する
（[ADR-0064](docs/adr/0064-dense-ft-gradient-mps.md)）。

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

## 謝辞

探索部は[やねうら王](https://github.com/yaneurao/YaneuraOu)を参照実装とし、
機能差分を埋める形で開発している（[ADR-0109](docs/adr/0109-reference-parity.md)）。
やねうら王は[Stockfish](https://github.com/official-stockfish/Stockfish)の
探索技法を将棋へ移植したもので、本エンジンはその系譜に連なる。
移植したファイルには冒頭に由来を書いている。

## License

GPL-3.0-or-later

Copyright (C) 2026 Kazumasa Yamamoto

v0.16.2まではMITで配布した。GPLv3への変更の経緯は
[ADR-0108](docs/adr/0108-license-gplv3.md)にある。既存のタグはMITのまま
変わらない。

学習済みネットと定跡データはプログラムの出力物であり、このライセンスの
対象外とする。配布条件は[ADR-0080](docs/adr/0080-net-release.md)と
[ADR-0082](docs/adr/0082-book-release.md)を参照。
