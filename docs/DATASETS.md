# 教師データカタログ

学習に使う公開データセットの所在・形式・前処理手順を管理する。
データ本体はリポジトリに含めない（`data/` はgitignore）。

## 利用中

### nodchip/shogi_hao_depth9

- URL: https://huggingface.co/datasets/nodchip/shogi_hao_depth9
- 形式: PackedSfenValue（40B/局面）
- 生成: Haoエンジン、depth 9
- 規模: 約10億局面（48ファイル、合計320GB）
- ライセンス: MIT
- 注意: 未シャッフル。qsearch PV葉への局面置換なし

取得済みファイル（2026-07-21時点。`data/` 配下）:

| thread_index | サイズ | 局面数（概算） |
|---|---|---|
| 000〜023 | 各250〜340MB | 各625〜850万 |
| **合計24ファイル** | **約14.7GB** | **約1億8000万** |

前処理（最新、P7用）:
1. `psv shuffle --in 000,...,022 --out train_385M.psv --seed 42`
   で23ファイルをシャッフル
2. `psv head --in 023.bin --out valid_385M.psv --count 200000`
   で検証データ（対局単位分離）
3. train_385M.psv = 180,640,795局面、valid_385M.psv = 200,000局面

旧前処理（P5/P6用）:
1. train_big.psv = 86,115,727局面（000〜010シャッフル）
2. valid_big.psv = 200,000局面（011から抽出）

最新の学習結果（halfkp_180M、純粋HalfKP、利き塔除去後）:
- data=train_385M.psv, epochs=1, batch=16384, peak_lr=1e-3,
  warmup=100, cosine decay, lambda=0.7
- best checkpoint: step 10000（valid loss 0.517）
- 対駒割SPRT: +528 Elo [+371,+3600]（H1採択、22ペア44局）

## 未取得（候補）

### nodchip/shogi_hao_depth9（残りファイル）

- thread_index 024〜047の24ファイル（合計約150GB、約3.7億局面）
- 同一データセットの残り。取得コマンド:
  `cd data && for i in $(seq -w 024 047); do curl -L -O "https://huggingface.co/datasets/nodchip/shogi_hao_depth9/resolve/main/kifu.tag%3Dtrain.depth%3D9.num_positions%3D1000000000.start_time%3D1695340981.thread_index%3D${i}.bin"; done`

### nodchip/tanuki-.nnue-pytorch-2024-07-30.1

- URL: https://huggingface.co/datasets/nodchip/tanuki-.nnue-pytorch-2024-07-30.1
- 形式: PackedSfenValue（40B/局面）
- 生成: tanukiエンジン、depth 9
- 規模: 320GB（7z圧縮5分割）
- ライセンス: MIT
- 注意: 未シャッフル。qsearch PV葉への局面置換なし
- 取得方法: 7z分割ファイルをダウンロード→結合→解凍
  `cat *.7z.00? | 7z x -si` で展開（要7z）
