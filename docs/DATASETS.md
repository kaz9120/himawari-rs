# 教師データカタログ

学習に使う教師データの所在・形式・前処理手順を持つ。データ本体はリポジトリに
含めない（`data/` はgitignore）。

教師の局面はtanuki- 2024の78.6億局面である（[ADR-0192](adr/0192-tanuki2024-teacher.md)）。
評価値は先頭20億だけをdlshogi 2021版の推論1回で付け直した（[ADR-0219](adr/0219-relabel-2b-in-place.md)）。
残り58.5億はtanukiの評価値のままである。いま学習に使う母集団 `pool1` は、
付け直した20億から重複を除いた17.1億に、付け直した入玉の局面を足したものである
（[ADR-0228](adr/0228-grow-labeled-pool.md)）。母集団を増やす回は止めている
（[ADR-0229](adr/0229-roadmap-visible-focus.md)）。前の教師
hao_depth9の加工済みpsvは2026-09-08に消した（容量の都合。再取得は
`hmwr data fetch --dataset hao` で通る）。

## 手元にあるもの（2026-10-16）

| ファイル | 中身 | 容量 |
|---|---|---|
| `data/train/pool1.psv` | 現行ネットの学習データ。21.5億局面（下の「現行ネットの学習条件」） | 86GB |
| `data/train/train_7860M_q1.psv` | tanuki 2024の学習データ、1手静止化済み。先頭20億の評価値はdlshogiの付け直し（S=430） | 314GB |
| `data/train/train_7860M_q1.psv.scores-before.i16` | 付け直す前のtanukiの評価値の控え（先頭20億、2バイト×局面数） | 4GB |
| `data/train/rank_tanuki_210M.rankpsv` | 同じシャッフル出力の先頭から作った兄弟群（[ADR-0185](adr/0185-sibling-ranking-loss.md)） | 24GB |
| `data/train/valid_tanuki_dl430_q1.psv` | tanukiの検証集合20万局面を、dlshogiで付け直したもの。dlshogiのラベルで学習するときに使う | 8MB |
| `data/train/valid_tanuki.psv` / `_q1.psv` | tanukiの検証集合20万局面（非静止・静止化） | 8MB×2 |
| `data/train/e4_dl430_train_q1.psv` | E4の1億をdlshogiで付け直したもの（[ADR-0215](adr/0215-dl-relabel-pilot.md)） | 4GB |
| `data/ledger/` | dlshogiで点数を付けた局面の盤面ハッシュの台帳。付け直しの重複を避ける（[ADR-0228](adr/0228-grow-labeled-pool.md)） | 16GB |
| `data/models/dlshogi/` | 付け直しに使うdlshogi 2021版のONNX | 105MB |
| `data/raw/entering_king/` | 入玉5億の生データ127ファイル（下の「入玉」） | 19GB |
| `data/raw/floodgate_archive/` | floodgateの全対局の年別アーカイブ（wdoorの7z、GitHubのミラー）。`hmwr data floodgate` が取得する | 2011〜2026年で約3.8GB |

過去の実験の中間ファイル（自己生成の `rl_*`、E4の統制、hao由来の3億、入玉の加工済み）は
2026-10-06に消した。`pool1` の材料（`dl20u`・`ek_dl430`・`ek_new_dl430`）は、
混ぜるときに消費した。どれもADRに作り方が残っている。

## 利用中: nodchip/tanuki-.nnue-pytorch-2024-07-30.1

| 項目 | 値 |
|---|---|
| URL | https://huggingface.co/datasets/nodchip/tanuki-.nnue-pytorch-2024-07-30.1 |
| 形式 | PackedSfenValue（40B/局面） |
| 生成 | tanukiエンジン、depth 9 |
| 規模 | 998ファイル・生293GiB・78.54億局面 |
| ライセンス | MIT |

未シャッフルで、qsearch PV葉への局面置換もない。前処理でどちらも手当てする。

### 取得と前処理

`hmwr data fetch --dataset tanuki2024` で取得し、消費型の全体シャッフル→
先頭から兄弟群を導出→分割出力を1本ずつ静止化、の順で加工する。空き
356GBに収める段取りとピークの占有は[ADR-0192](adr/0192-tanuki2024-teacher.md)の
「前処理パイプライン」にある。生データはシャッフルで消費するので、
手元に残っているのは `data/raw/tanuki2024/` の1ファイルだけである。

静止化は1手（`--max-plies 1`、[ADR-0136](adr/0136-quiet-teacher-positions.md)）。
`hmwr data quiet` の既定は1手である。既定が16手だった時期に、212GBを誤った条件で
静止化した事故がADR-0192にある。

検証集合は特定の1ファイルを学習から除いて切り出す。学習データと同じ
静止化を当てないと、best checkpointの選択が歪む
（[ADR-0136](adr/0136-quiet-teacher-positions.md)）。

### 現行ネットの学習条件

`pool1_2ep`（HalfKP 1024x16x32）に並べ替えを当てた `pool1_2ep_reorder`。
現行の評価関数の一次情報は `hmwr/config.py` の `EVAL_FILE` である。

- データ: `pool1.psv`（2,152,947,210局面）。付け直した20億から重複を除いた
  `dl20u`（[ADR-0225](adr/0225-dedup-20b.md)）に、dlshogiで付け直した入玉の局面を
  全体シャッフルで混ぜた
- 兄弟群: `rank_tanuki_210M.rankpsv`（ランキング損失）
- 2エポック、batch=16384、peak_lr 1e-3、λ0.7、262,812ステップ、24.8時間
- 検証集合: `valid_tanuki_dl430_q1.psv`
- 検収: `dl20u_2ep` 対 `pool1_2ep` で +5.9 Elo [+1.2, +10.6]、H1
  （[ADR-0228](adr/0228-grow-labeled-pool.md)）

**valid lossは検証集合を揃えないと比較できない**。静止化した教師で学習した
ネットは、非静止の検証集合では0.0285悪い値を出しながら対局では+20.3 Eloで
勝った。採否は対局で決める（[ADR-0136](adr/0136-quiet-teacher-positions.md)）。

### 規模の制約

RAM 48GBに対し78.6億のpsvは293GBあり、ページキャッシュに載らない。学習器は
約48kサンプル/秒で、78.6億の1エポックに約50時間、`pool1` の2エポックに約25時間かかる。

## 前の教師: nodchip/shogi_hao_depth9

| 項目 | 値 |
|---|---|
| URL | https://huggingface.co/datasets/nodchip/shogi_hao_depth9 |
| 形式 | PackedSfenValue（40B/局面） |
| 生成 | Haoエンジン、depth 9 |
| 規模 | 381ファイル・112GB・約30.0億局面 |
| ライセンス | MIT |

3グループすべてを取得して使い切った（[ADR-0135](adr/0135-teacher-data-3b.md)）。
生データと `train_2990M*.psv` は消してあり、3億局面の小規模学習用データと検証集合だけを
残している。取得と前処理は `hmwr data fetch --dataset hao` の1本で通る。
19.9億から29.9億への増量で+24.8、静止化で+13.9だった。過去世代
（180M・370M・1900M）の前処理条件は[ADR-0061](adr/0061-psv-memmap-dataset.md)と
[ADR-0065](adr/0065-large-scale-dataloader.md)にある。

## 入玉: nodchip/shogi_suisho5_depth9_entering_king

| 項目 | 値 |
|---|---|
| URL | https://huggingface.co/datasets/nodchip/shogi_suisho5_depth9_entering_king |
| 形式 | PackedSfenValue（40B/局面） |
| 生成 | floodgateの2015〜2024年から入玉局面を集め、Suisho5のdepth 9でラベル |
| 規模 | 127ファイル・19GB・約5億局面 |
| ライセンス | MIT |

`hmwr data fetch --dataset entering-king` で取得する。Suisho5の評価値は使わず、
局面だけを取り出してdlshogiで付け直した。重複を除いた約4.4億局面が `pool1` に
入っている（[ADR-0228](adr/0228-grow-labeled-pool.md)）。生データは
`data/raw/entering_king/` に残してある。

