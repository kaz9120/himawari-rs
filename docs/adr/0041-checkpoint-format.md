# 0041: 学習チェックポイント形式

- Status: rejected（ADR-0040のPyTorch移行により、torch.save/torch.loadで代替）
- Date: 2026-07-20
- 関連ADR: [0039](0039-trainer-v1.md), [0037](0037-nnue-file-format.md), [0040](0040-training-infra-v2.md)

## Context

現在の学習器は中間状態を保存できない。学習を中断すると
f32重みとAdam状態が失われ、最初からやり直しになる。

P6でlr schedule（ADR-0040）を入れると学習が長くなる。
中断耐性が必要になる。P8のRL世代ループ（前世代ネットからの
継続学習）にもチェックポイントが前提になる。

推論用の量子化ネット（ADR-0037の.hmwr形式）はi8/i16で
丸められており、学習再開には使えない。f32精度の保存形式が要る。

## 選択肢と比較

### 案A: JSON metadata + raw f32バイナリ

ヘッダがJSON。jqやPythonで条件を検査できる。f32配列は
固定順序で続く。実装はシンプル。

### 案B: 全体をカスタムバイナリ

メタデータも固定長バイナリ。やや小さいが、検査に専用ツールが
必要。拡張性も低い。

## Decision

案Aを採用する。

### ファイル構造

```
[4B magic "HMCK"]
[4B version: u32 LE = 1]
[4B json_len: u32 LE]
[json_len bytes: UTF-8 JSON]
[0-7B padding to 8-byte align]
[f32 LE arrays: net, adam_m, adam_v]
```

f32配列はFloatNetのフィールド順: ft_w, ft_b, ef_w, ef_b,
w2, b2, w3, b3, w4, b4。Adam mとvも同じ順序で続く。

現構成でのファイルサイズ: パラメータ約1,590万 × 3 × 4B ≈ 190MB。

### JSON metadata

```json
{
  "arch": {
    "ft_in": 61776, "ft_out": 256,
    "effect_in": 1458, "effect_out": 32,
    "hidden": 32
  },
  "step": 3000,
  "adam_t": 3000,
  "epoch": 0,
  "samples": 49152000,
  "best_valid_loss": 0.526,
  "best_step": 2800,
  "peak_lr": 1e-3,
  "min_lr": 1e-6,
  "warmup_steps": 100,
  "total_steps": 21028,
  "lambda": 0.7,
  "batch": 16384,
  "score_limit": 0,
  "seed": 1,
  "data": "train_big.psv",
  "data_n": 86115727
}
```

arch情報でロード時にアーキテクチャ互換性を検証する。

### CLI

| パラメータ | 既定 | 意味 |
|---|---|---|
| --checkpoint-interval | valid-intervalと同じ | 保存間隔（ステップ） |
| --checkpoint-dir | なし | 保存先（指定で有効化） |
| --resume | なし | 復元元ファイル |

保存ファイル: {checkpoint-dir}/latest.ckpt を上書き保存する。
best valid loss更新時は {checkpoint-dir}/best.ckpt もコピーする。

### 再開時の動作

1. f32重み・Adam状態・stepを復元する
2. checkpoint内のハイパラとCLI引数を照合し、不一致を警告する
   （意図的変更は許容）
3. データファイルのepochとstep位置を計算して読み飛ばし、
   続きのバッチから再開する

## Consequences

- 長時間学習を安全に中断・再開できる
- 継続学習（前世代ネットからfinetune）がチェックポイントの
  ロード+lr変更で実現できる
- 190MBのファイルI/Oがcheckpoint-intervalごとに発生する。
  数百ステップに1回なら学習スループットへの影響は軽微
- ネットワーク構造の変更（P7）でフォーマットが非互換になる。
  arch検証で誤ロードを防ぎ、version番号で管理する
