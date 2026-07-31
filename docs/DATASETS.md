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

取得済みファイル（2026-07-26時点。`data/raw/hao_depth9/` 配下）:

| start_time | thread_index | サイズ | 局面数 |
|---|---|---|---|
| 1695340981 | 000〜126（127ファイル） | 約38GB | 約9.5億 |
| 1695606850 | 000〜126（127ファイル） | 約39GB | 約10.0億 |
| **合計254ファイル** | | **77GB** | **約19.9億** |

2026-07-21時点の表にあった「48ファイル・約27GB」は誤りで、
実測は約14GBだった（1ファイルあたり約280〜340MB）。

前処理（最新、P8用。[ADR-0065](adr/0065-large-scale-dataloader.md)）:
1. start_time=1695340981のthread_index=023を除く253ファイルを
   `psv shuffle` で全体シャッフルする。2パスのバケット法で動くため、
   RAMに載らない規模でも通る。所要は約3分。
   ```
   cd data/raw/hao_depth9
   FILES=$(ls | grep -v "start_time=1695340981.thread_index=023" | paste -sd, -)
   psv shuffle --in "$FILES" --out ../../train/train_1900M.psv --seed 42
   ```
2. 検証データは valid_385M.psv（1695340981の023由来）を据え置く。
   valid lossをhalfkp_180M・halfkp_370Mと直接比較するため
3. train_1900M.psv = 1,991,580,338局面（79.7GB）、
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

### nodchip/shogi_hao_depth9（残りファイル）

- start_time=1695872823の127ファイル（約39GB、約10億局面）が未取得。
  取得すれば約30億局面になる
- 取得コマンド（zsh。`seq -w`は3桁ゼロ埋めにならないので
  brace展開を使う）:
  `cd data && for i in {024..047}; do curl -L -O "https://huggingface.co/datasets/nodchip/shogi_hao_depth9/resolve/main/kifu.tag%3Dtrain.depth%3D9.num_positions%3D1000000000.start_time%3D1695340981.thread_index%3D${i}.bin"; done`
- 存在しないthread_indexを指定するとHFは15バイトの
  「Entry not found」を返す。DL後にサイズを確認する

### nodchip/tanuki-.nnue-pytorch-2024-07-30.1

- URL: https://huggingface.co/datasets/nodchip/tanuki-.nnue-pytorch-2024-07-30.1
- 形式: PackedSfenValue（40B/局面）
- 生成: tanukiエンジン、depth 9
- 規模: 320GB（7z圧縮5分割）
- ライセンス: MIT
- 注意: 未シャッフル。qsearch PV葉への局面置換なし
- 取得方法: 7z分割ファイルをダウンロード→結合→解凍
  `cat *.7z.00? | 7z x -si` で展開（要7z）
