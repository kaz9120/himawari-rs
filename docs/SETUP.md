# 開発環境の構築

新しいマシンで開発を引き継ぐ手順。Linux と WSL2 を対象にする。
設計判断の経緯は [ADR-0081](adr/0081-portability.md)、
ネットの配布は [ADR-0080](adr/0080-net-release.md) を参照。

## 全体の流れ

```
1. WSL2 を入れる（Windowsの場合）
2. scripts/setup.sh        ツールチェインとビルド
3. gh auth login           GitHubの認証
4. 学習済みネットの取得     gh release download
5. scripts/fetch-dataset.sh 教師データ（学習を回す場合のみ）
```

所要は2までが15分、4が数分、5が回線次第（77GB）。

## 1. WSL2（Windowsの場合）

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

## 2. ツールチェインとビルド

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

## 3. GitHubの認証

```bash
gh auth login
```

ネットの取得（`gh release download`）とPRの作成に要る。

## 4. 学習済みネット

リポジトリには含めない（[ADR-0080](adr/0080-net-release.md)）。
GitHub Release から取る。

```bash
gh release list | grep net-v
gh release download net-v<N> -D data/nets/
```

現行の最強構成は [ROADMAP.md](ROADMAP.md) の「現行の最強構成」を見る。

取得したら動作を確かめる。

```bash
printf 'usi\nsetoption name EvalFile value data/nets/<ネット>\nisready\nquit\n' \
  | ./target/release/himawari
```

`info string EvalFile loaded: ...` に学習来歴が出れば正しく読めている。

## 5. 教師データ（学習を回す場合のみ）

約160GBの空きが要る（生データ77GB + 加工済み80GB）。
探索の開発だけなら不要。

```bash
scripts/fetch-dataset.sh all
```

`download` → `verify` → `prepare` を順に実行する。中断しても
`download` は再実行できる（妥当なサイズのファイルは飛ばす）。

`prepare` は検証データの切り出しと全体シャッフルを行う。
シャッフルは2パスのバケット法で、RAMに載らない規模でも通る
（[ADR-0065](adr/0065-large-scale-dataloader.md)）。

データの詳細は [DATASETS.md](DATASETS.md) を参照。

## マシンに合わせる設定

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

## SPRTの回し方

```bash
scripts/sprt.sh <baselineバイナリ> <candidateバイナリ> <名前>
```

条件は `env.sh` の既定（[ADR-0028](adr/0028-pruning-extensions.md)）を使う。
棋譜は `data/sprt/<名前>.jsonl` へ書く。

SPRTの前に機能検証を済ませる（[ADR-0074](adr/0074-feature-verification.md)）。
固定深さでノード数が変わらない変更は、SPRTへ持ち込んでも中立にしかならない。

## floodgate参加（Windows）

WSL2内のバイナリをWindowsのGUI（ShogiGUI・将棋所）から直接叩くのは
面倒なため、GitHub Releaseのバイナリを使う。

```powershell
gh release download v<バージョン> -p "*windows-x64-avx2.zip"
```

`release.yml` がタグpushのたびにAVX2とSSE4.2の2種類をビルドしている。
CPUがAVX2に対応していればavx2版を使う。

評価関数は別途 `net-v<N>` から取り、GUIのエンジン設定で `EvalFile` に
絶対パスを指定する。

## macOSで開発する場合

`scripts/setup.sh` はLinux専用である。macOSでは個別に入れる。

```bash
brew install gh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
python3 -m venv .venv && source .venv/bin/activate
pip install -r training/requirements.txt
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

学習にMPSを使う場合は `--device mps` を指定する
（[ADR-0064](adr/0064-dense-ft-gradient-mps.md)）。
