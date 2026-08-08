# 教師データカタログ

学習に使う公開データセットの所在・形式・前処理手順を管理する。
データ本体はリポジトリに含めない（`data/` はgitignore）。

## 利用中

### nodchip/shogi_hao_depth9

- URL: https://huggingface.co/datasets/nodchip/shogi_hao_depth9
- 形式: PackedSfenValue（40B/局面）
- 生成: Haoエンジン、depth 9
- 規模: start_time 3グループ × thread_index 000〜126の
  計381ファイル。各約280〜350MB（約700〜850万局面）。
  合計約115GB・約29億局面（2026-07-21にHF APIで実測）
- ライセンス: MIT
- 注意: 未シャッフル。qsearch PV葉への局面置換なし

取得済みファイル（2026-08-04時点。`data/raw/hao_depth9/` 配下）。
**3グループすべてを取得し、このデータセットは使い切りになった**
（[ADR-0135](adr/0135-teacher-data-3b.md)）:

| start_time | thread_index | サイズ | 局面数 |
|---|---|---|---|
| 1695340981 | 000〜126（127ファイル） | 約37GB | 約9.9億 |
| 1695606850 | 000〜126（127ファイル） | 約37GB | 約10.0億 |
| 1695872823 | 000〜126（127ファイル） | 約37GB | 約10.0億 |
| **合計381ファイル** | | **112GB** | **約30.0億** |

2026-07-21時点の表にあった「48ファイル・約27GB」は誤りで、
実測は約14GBだった（1ファイルあたり約280〜340MB）。

前処理（最新。[ADR-0135](adr/0135-teacher-data-3b.md)）。
取得から加工まで `scripts/fetch-dataset.sh all` の1本で通る:
1. start_time=1695340981のthread_index=023を除く380ファイルを
   `psv shuffle` で全体シャッフルする。2パスのバケット法で動くため、
   RAMに載らない規模でも通る。
   ```
   cd data/raw/hao_depth9
   FILES=$(ls | grep -v "start_time=1695340981.thread_index=023" | paste -sd, -)
   psv shuffle --in "$FILES" --out ../../train/train_2990M.psv --seed 42
   ```
2. 検証データは valid_385M.psv（1695340981の023由来）を据え置く。
   valid lossをhalfkp_180M・halfkp_370M・halfkp_1900Mと直接比較するため
3. 教師局面を静止化する（[ADR-0136](adr/0136-quiet-teacher-positions.md)）。
   hao_depth9はqsearch PV葉への置換なしで配られており、駒の取り合いの
   途中の局面へ収束後の探索値が付いている。1手だけ進める設定で置換率は
   36.15%、29.9億で7.0時間かかる。
   ```
   psv quiet --in data/train/train_2990M.psv \
             --out data/train/train_2990M_q1.psv \
             --max-plies 1 --eval-file data/nets/<現行ネット>.hmwr.best
   ```
4. **検証集合も同じ設定で静止化する。** 学習データと土俵を揃えないと、
   best checkpointの選択が歪む。非静止の検証集合で測った値は、静止化した
   ネットには不利に出る（[ADR-0136](adr/0136-quiet-teacher-positions.md)）

前処理（P8用、19.9億。[ADR-0065](adr/0065-large-scale-dataloader.md)）:
1. 上と同じ手順を253ファイル（start_time=1695340981と1695606850）で行う
2. train_1900M.psv = 1,991,580,338局面（79.7GB）、
   valid_385M.psv = 200,000局面
4. 学習時は `--mmap --batch-loader` で読む。RAMに載らないため、
   チャンク単位のシャッフルで供給する（281,000 samples/s）

前処理（P8前半、370M用。[ADR-0061](adr/0061-psv-memmap-dataset.md)）:
1. 023を除く47ファイルを連結する。事前シャッフルは行わない
   （DataLoaderの`shuffle=True`が全体から一様にサンプリングする）
   ```
   cd data/raw/hao_depth9
   ls | grep -v "thread_index=023" | tr '\n' '\0' \
     | xargs -0 cat > ../../train/train_370M.psv
   ```
2. train_370M.psv = 369,779,710局面、valid_385M.psv = 200,000局面

前処理（P7用）:
1. `psv shuffle --in 000,...,022 --out train_385M.psv --seed 42`
   で23ファイルをシャッフル
2. `psv head --in 023.bin --out valid_385M.psv --count 200000`
   で検証データ（対局単位分離）
3. train_385M.psv = 180,640,795局面、valid_385M.psv = 200,000局面

旧前処理（P5/P6用）:
1. train_big.psv = 86,115,727局面（000〜010シャッフル）
2. valid_big.psv = 200,000局面（011から抽出）

最新の学習結果（halfkp_1900M_fact、純粋HalfKP 256x2-32-32 + factorizer）:
- data=train_1900M.psv（19.9億局面）, epochs=1, batch=16384,
  peak_lr=1e-3, warmup=100, cosine decay, lambda=0.7
- best checkpoint: step 116000（valid loss 0.49513）、所要149分
- 対halfkp_370M: +243.8 Elo（データ拡大分）、
  対halfkp_1900M: +28.1 Elo（factorizer分）。いずれもH1採択
- 詳細は計測記録、条件は
  [ADR-0064](adr/0064-dense-ft-gradient-mps.md)〜
  [ADR-0066](adr/0066-halfkp-factorizer.md)

## 未取得（候補）

### nodchip/tanuki-.nnue-pytorch-2024-07-30.1

- URL: https://huggingface.co/datasets/nodchip/tanuki-.nnue-pytorch-2024-07-30.1
- 形式: PackedSfenValue（40B/局面）
- 生成: tanukiエンジン、depth 9
- 規模: 320GB（7z圧縮5分割）
- ライセンス: MIT
- 注意: 未シャッフル。qsearch PV葉への局面置換なし
- 取得方法: 7z分割ファイルをダウンロード→結合→解凍
  `cat *.7z.00? | 7z x -si` で展開（要7z）
