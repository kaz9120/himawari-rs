# 教師データカタログ

学習に使う教師データの所在・形式・前処理手順を持つ。データ本体はリポジトリに
含めない（`data/` はgitignore）。

**公開データの供給は尽きている。** hao_depth9は2026-08-04に3グループすべてを
取得し、使い切りになった（[ADR-0135](adr/0135-teacher-data-3b.md)）。次の増量は
自前生成しかない（[ADR-0144](adr/0144-selfplay-teacher-loop.md)）。

## 利用中: nodchip/shogi_hao_depth9

| 項目 | 値 |
|---|---|
| URL | https://huggingface.co/datasets/nodchip/shogi_hao_depth9 |
| 形式 | PackedSfenValue（40B/局面） |
| 生成 | Haoエンジン、depth 9 |
| 規模 | 381ファイル・112GB・約30.0億局面 |
| ライセンス | MIT |

未シャッフルで配られる。qsearch PV葉への局面置換もない。前処理でどちらも
手当てする。

取得済みファイル（`data/raw/hao_depth9/` 配下）。

| start_time | thread_index | サイズ | 局面数 |
|---|---|---|---|
| 1695340981 | 000〜126（127ファイル） | 約37GB | 約9.9億 |
| 1695606850 | 000〜126（127ファイル） | 約37GB | 約10.0億 |
| 1695872823 | 000〜126（127ファイル） | 約37GB | 約10.0億 |
| 合計381ファイル | | 112GB | 約30.0億 |

### 取得と前処理

取得から加工まで `scripts/fetch-dataset.sh all` の1本で通る。中で何が起きるかを
知りたいとき、または途中から回すときは以下を読む。

**1. 全体シャッフル。** start_time=1695340981のthread_index=023を除く380ファイルを
`psv shuffle` にかける。2パスのバケット法で動くため、RAMに載らない規模でも通る。

```sh
cd data/raw/hao_depth9
FILES=$(ls | grep -v "start_time=1695340981.thread_index=023" | paste -sd, -)
psv shuffle --in "$FILES" --out ../../train/train_2990M.psv --seed 42
```

**2. 検証データは valid_385M.psv を据え置く**（1695340981の023由来）。
halfkp_180M・halfkp_370M・halfkp_1900Mのvalid lossと直接比べるためである。

**3. 教師局面の静止化**（[ADR-0136](adr/0136-quiet-teacher-positions.md)）。
hao_depth9は駒の取り合いの途中の局面へ収束後の探索値が付いている。1手だけ
進める設定で置換率は36.15%、29.9億で7.0時間かかる。

```sh
psv quiet --in data/train/train_2990M.psv \
          --out data/train/train_2990M_q1.psv \
          --max-plies 1 --eval-file data/nets/<現行ネット>.hmwr.best
```

**4. 検証集合も同じ設定で静止化する。** 学習データと土俵を揃えないと、best
checkpointの選択が歪む。非静止の検証集合で測った値は、静止化したネットには
不利に出る。この罠で改善を捨てかけた実例がADR-0136にある。

### 現行ネットの学習条件

`halfkp_2990M_q1`（純粋HalfKP 256x2-32-32 + factorizer）。

- データ: `train_2990M_q1.psv`（2,991,590,036局面）
- epochs=1、batch=16384、peak_lr=1e-3、warmup=100、cosine decay、lambda=0.7
- best checkpoint: step 170,000、valid loss 0.49097（静止化した検証集合）
- 1エポックの所要は31,017秒（8.6時間、96,000局面/秒）

条件は同じで教師だけを差し替えた比較が2つある。19.9億から29.9億への増量で
+24.8 Elo [+12.9, +36.8]（[ADR-0135](adr/0135-teacher-data-3b.md)）、静止化で
+13.9 Elo [+5.3, +22.5]（[ADR-0136](adr/0136-quiet-teacher-positions.md)）である。

**valid lossは検証集合を揃えないと比較できない。** 静止化した教師で学習した
ネットは、非静止の検証集合では0.0285悪い値を出しながら対局では+20.3 Eloで
勝った。採否は対局で決める（[ADR-0136](adr/0136-quiet-teacher-positions.md)）。

過去世代（180M・370M・1900M）の前処理条件は
[ADR-0061](adr/0061-psv-memmap-dataset.md)と
[ADR-0065](adr/0065-large-scale-dataloader.md)にある。

### 規模の制約

RAM 48GBに対し29.9億のpsvは111GBあり、ページキャッシュに載らない。処理速度は
19.9億の222,000局面/秒から96,000局面/秒へ落ち、所要は8,952秒から31,017秒に
なった。これ以上増やすなら供給側の設計が要る。

## 未取得（候補）

### nodchip/tanuki-.nnue-pytorch-2024-07-30.1

| 項目 | 値 |
|---|---|
| URL | https://huggingface.co/datasets/nodchip/tanuki-.nnue-pytorch-2024-07-30.1 |
| 形式 | PackedSfenValue（40B/局面） |
| 生成 | tanukiエンジン、depth 9 |
| 規模 | 320GB（7z圧縮5分割） |
| ライセンス | MIT |

未シャッフルで、qsearch PV葉への局面置換もない点はhao_depth9と同じ。
7z分割ファイルをダウンロードして結合し、`cat *.7z.00? | 7z x -si` で展開する
（要7z）。
