# 教師データカタログ

学習に使う教師データの所在・形式・前処理手順を持つ。データ本体はリポジトリに
含めない（`data/` はgitignore）。

現行の教師はtanuki- 2024の78.6億局面である（[ADR-0192](adr/0192-tanuki2024-teacher.md)、
+43.2）。前の教師hao_depth9の生データと加工済みpsvは2026-09-08に消した
（容量の都合。再取得は `hmwr data fetch --dataset hao` で通る）。公開データの
残り手は入玉5億の混合で、生データは取得済み。次の増量は自前生成の
ループ（[ADR-0201](adr/0201-loop-recipe.md)）になる。

## 手元にあるもの（2026-09-08）

| ファイル | 中身 | 容量 |
|---|---|---|
| `data/train/train_7860M_q1.psv` | tanuki 2024の学習データ、1手静止化済み | 314GB |
| `data/train/rank_tanuki_210M.rankpsv` | 同じシャッフル出力の先頭から作った兄弟群（[ADR-0185](adr/0185-sibling-ranking-loss.md)） | 24GB |
| `data/train/valid_tanuki.psv` / `_q1.psv` | tanukiの検証集合20万局面（非静止・静止化） | 8MB×2 |
| `data/train/train_300M.psv` / `_q1.psv`、`rank_300M_100M.rankpsv` | hao由来の3億の測定台（[ADR-0135](adr/0135-teacher-data-3b.md)）。静止化前は兄弟群の作り直しに要る | 36GB |
| `data/train/valid_385M*.psv` | hao由来の検証集合。テレメトリとして残す（[ADR-0150](adr/0150-rootstrap-evaluation.md)） | 8MB×3 |
| `data/raw/entering_king/` | 入玉5億の生データ127ファイル（下の「入玉」） | 19GB |
| `data/train/rl_*.psv` など | 自前生成の世代データと中間ファイル。実験ごとにADRが持つ | 実験による |

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
`psv quiet` の既定を1手に揃えてあるが、チェーンを直接組むときは明示する。
既定が16手だった時期に212GBを誤った条件で静止化した事故がADR-0192にある。

検証集合は特定の1ファイルを学習から除いて切り出す。学習データと同じ
静止化を当てないと、best checkpointの選択が歪む
（[ADR-0136](adr/0136-quiet-teacher-positions.md)）。

### 現行ネットの学習条件

`pairrank_7860M_q1`（HalfKP 1024x16x32、対の積＋ランキング損失）。

- データ: `train_7860M_q1.psv`（78.54億局面）と `rank_tanuki_210M.rankpsv`
- 1エポック、batch=16384、479,372ステップ、50.6時間（48kサンプル/秒）
- 検収: `rl_g1b_reorder` 対 `pairrank_7860M_q1_reorder` で
  +43.2 Elo [+26.9, +59.6]、H1（[ADR-0192](adr/0192-tanuki2024-teacher.md)）

**valid lossは検証集合を揃えないと比較できない**。静止化した教師で学習した
ネットは、非静止の検証集合では0.0285悪い値を出しながら対局では+20.3 Eloで
勝った。採否は対局で決める（[ADR-0136](adr/0136-quiet-teacher-positions.md)）。

### 規模の制約

RAM 48GBに対し78.6億のpsvは293GBあり、ページキャッシュに載らない。等価
高速化後の学習器で48kサンプル/秒、1エポック50時間である。

## 前の教師: nodchip/shogi_hao_depth9

| 項目 | 値 |
|---|---|
| URL | https://huggingface.co/datasets/nodchip/shogi_hao_depth9 |
| 形式 | PackedSfenValue（40B/局面） |
| 生成 | Haoエンジン、depth 9 |
| 規模 | 381ファイル・112GB・約30.0億局面 |
| ライセンス | MIT |

3グループすべてを取得して使い切った（[ADR-0135](adr/0135-teacher-data-3b.md)）。
生データと `train_2990M*.psv` は消してあり、3億の測定台と検証集合だけを
残している。取得と前処理は `hmwr data fetch --dataset hao` の1本で通る。
19.9億から29.9億への増量で+24.8、静止化で+13.9だった。過去世代
（180M・370M・1900M）の前処理条件は[ADR-0061](adr/0061-psv-memmap-dataset.md)と
[ADR-0065](adr/0065-large-scale-dataloader.md)にある。

## 取得済み・未使用: nodchip/shogi_suisho5_depth9_entering_king

| 項目 | 値 |
|---|---|
| URL | https://huggingface.co/datasets/nodchip/shogi_suisho5_depth9_entering_king |
| 形式 | PackedSfenValue（40B/局面） |
| 生成 | floodgateの2015〜2024年から入玉局面を集め、Suisho5のdepth 9でラベル |
| 規模 | 127ファイル・19GB・約5億局面 |
| ライセンス | MIT |

`hmwr data fetch --dataset entering-king` で取得済み。教師の入玉の薄さ
（1.65%、[ADR-0190](adr/0190-selfplay-decided-thinning.md)）と宣言負け
（floodgateで8局、[ADR-0199](adr/0199-roadmap-inventory.md)）へ効かせる
混合が候補表にある。

