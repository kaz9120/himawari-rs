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

取得済みファイル（2026-07-20時点。`data/` 配下）:

| thread_index | サイズ | 局面数（概算） |
|---|---|---|
| 000 | 290MB | 725万 |
| 001 | 340MB | 850万 |
| 002〜011 | 各250〜350MB | 各625〜875万 |
| **合計12ファイル** | **3.6GB** | **約8,600万** |

前処理:
1. `psv shuffle --in 000,001,...,010 --out train_big.psv` で11ファイルをシャッフル
2. `psv head --in 011.bin --out valid_big.psv --count 200000` で検証データ（対局単位分離）
3. train_big.psv = 86,115,727局面、valid_big.psv = 200,000局面

P5出口で使った学習条件（hao_v6 best、対駒割+253 Elo）:
- data=train_big.psv, epochs=4, batch=16384, lr=1e-3, lambda=0.7
- best checkpoint: step 3000（valid loss 0.526、エポック1の57%地点）

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

### nodchip/shogi_hao_depth9（残りファイル）

- thread_index 012〜047の36ファイル（合計約280GB、約70億局面）
- 同一データセットの残り。必要に応じて追加取得
