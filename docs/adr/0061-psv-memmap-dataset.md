# 0061: 学習データの事前シャッフルを廃止し、読み込みをRAM常駐＋forkに統一する

- Status: superseded
- Date: 2026-07-25
- 関連ADR: [0040](0040-training-infra-v2.md), [0043](0043-pyo3-bridge.md), [0038](0038-training-data-format.md), [0065](0065-large-scale-dataloader.md)（本ADRを置き換え）

## Context

学習データを180M局面から370M局面へ増やしたところ、前処理とデータ
ローダの両方が詰まった。データは14.8GBで、作業機のRAMは48GBである。

詰まりは3か所にあった。

第1に、`training/dataset.py` の `PsvDataset` は docstring に
「Memory-mapped PSV dataset」と書きながら、実装は `np.fromfile` に
よる全ロードだった。第2に、`psv shuffle` は入力を `Vec<u8>` に
`read_to_end` する（`crates/tools/src/bin/psv.rs:157`）。第3に、
DataLoaderは `shuffle=True`（`training/train.py`）で、データセット
全体から一様にサンプリングする。

第3の性質から、事前シャッフルは全ロード前提の設計に由来する冗長な
工程だと分かる。ファイル上の並びは学習の順序に影響しない。

## 選択肢と比較

同一条件（370M局面、batch=16384、workers=4、CPU）で実測した。

### 案A: memmapで開き、DataLoaderのshuffle=Trueに任せる

前処理は生ファイルの連結だけで済む。メモリ使用量も最小になる。
実測は 5,303 samples/s で、halfkp_180M の学習時（約45,000
samples/s）の1/8.5だった。CPU使用率は75%まで落ち、I/O待ちが
支配的だった。14.8GBへのランダムアクセスがページキャッシュに
収まらない。

### 案B: 全ロードのまま（start methodはOS既定）

macOSのPython 3.10はmultiprocessingの既定がspawnである。DataLoader
のworkerごとにデータセットがpickleで複製されるため、4 workerで
約74GBに膨れる。実測ではプロセスがOSにkillされ、20個のセマフォが
残った。Tracebackは出ない。

### 案C: 全ロード＋fork（コピーオンライト共有）

`multiprocessing_context` にforkを明示する。workerは親のページを
共有し、複製が起きない。実測は 41,019 samples/s で、halfkp_180M と
同等の水準に戻った。22,570ステップの所要は約2.5時間の見込み。

### 案D: 事前シャッフル＋memmap＋順次読み

ファイル側を混ぜておき、DataLoaderを `shuffle=False` にする。
memmapでも順次アクセスになるため先読みが効く。メモリ使用量は最小で
済むが、`psv shuffle` のストリーミング化が前提になる。

## Decision

案Cを採用する。あわせて事前シャッフルを廃止する。

### 実装

`training/dataset.py` に読み込み方式の切り替えを持たせる。既定は
全ロードで、mmapはRAMに載らない規模用のエスケープハッチとする。

```python
def __init__(self, path, lambda_=0.7, score_limit=0, mmap=False):
    size = os.path.getsize(path)
    if size % 40 != 0:
        raise ValueError(f"ファイルサイズが40の倍数でない: {size}")
    if mmap:
        self.data = np.memmap(path, dtype=np.uint8, mode="r", shape=(size // 40, 40))
    else:
        self.data = np.fromfile(path, dtype=np.uint8).reshape(-1, 40)
```

`training/train.py` はDataLoaderにforkを明示し、`--mmap` フラグを
追加する。

```python
mp_ctx = multiprocessing.get_context("fork") if args.workers > 0 else None
```

学習データは生ファイルの連結で作る。

```
cd data/raw/hao_depth9
ls | grep -v "thread_index=023" | tr '\n' '\0' | xargs -0 cat > ../../train/train_370M.psv
```

`train_370M.psv` は hao_depth9 の thread_index 000〜047 から023を
除く47ファイル、369,779,710局面（14,791,188,400バイト）。023は
`valid_385M.psv` の由来ファイルなので除外し、train/validの対局単位
分離（P5で確立）を保つ。validを据え置くことで、valid lossを
halfkp_180M（0.51727）と直接比較できる。

`psv shuffle` は変更しない。小規模データの前処理では引き続き使える。

## Consequences

- 前処理が連結だけになり、データ拡大の手順が短くなった
- RAM容量が学習データ量の上限を決める。48GB機では370M局面
  （14.8GB）が実用上限に近い。プロセス群のRSS合計は48.1GBと表示
  されるが、これはCoW共有ページを各workerで重複計上した値である
- 次の拡大（hao_depth9の残り103ファイル、約8億局面・32GB）は本方式
  では載らない。そこで案Dに移る。`psv shuffle` のストリーミング化
  （ROADMAPの候補の「ストリーミングチャンクシャッフル」）とセットで
  起草する
- forkはmacOSのPythonで非推奨とされる。現状のworkerはnumpyの読み取り
  とRust製 `extract_features` の呼び出しに限られるため問題は出て
  いないが、worker内でスレッドを使う処理が増えるとデッドロックの
  危険がある
- `--mmap` を残したので、RAMが小さい環境でも学習自体は動く。速度は
  案Aの水準（1/8.5）に落ちる

## 追記（2026-07-26）

[ADR-0065](0065-large-scale-dataloader.md) が本ADRを置き換えた。

案Cの全ロード＋forkが通用するのは、データがRAMに収まる規模までである。
19.9億局面（79.7GB）は48GBに載らない。ADR-0065は本ADRの案D
（事前シャッフル＋順次読み）にバッチ一括抽出を足した形を採り、
DataLoaderのworkerプロセス自体を使わなくした。forkを明示する必要も
なくなり、本ADRが挙げたデッドロックの懸念は解消した。

`psv shuffle` のストリーミング化も同ADRで実装した。本ADRが
「案Dに移る前提」と書いた条件は満たされている。
