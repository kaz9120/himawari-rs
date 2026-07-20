# 0040: 学習器v2（PyTorch移行）

- Status: accepted（2026-07-20オーナー承認）
- Date: 2026-07-20
- 関連ADR: [0039](0039-trainer-v1.md), [0037](0037-nnue-file-format.md), [0038](0038-training-data-format.md)

## Context

P6で学習基盤を完成させるにあたり、lr schedule・チェックポイント・
学習ログ・early stoppingの実装が必要になった。これらはML
フレームワークの標準機能である。

P7ではネットワーク構造の変更実験に入る。自動微分なしでは構造を
変えるたびに逆伝播を手で書き直すことになる。ADR-0039で
「構造をいじる実験段階に入ったらフレームワーク移行を再検討する」
と定めている。

Rust MLフレームワーク（Burn, Candle, dfdx, tch-rs）も検討した。
Burnが唯一の現実的候補だが、NNUE学習の要であるsparse embedding
更新最適化が標準APIにない。PyTorchはnn.EmbeddingBagの
sparse gradient対応でこれを自然に扱える。

## 選択肢と比較

### 案A: 現Rust学習器にP6機能を自前実装

lr schedule等は各100-200行で実装できる。P6のコストは最小。
ただしP7で逆伝播の手書きが残る。

### 案B: Burn（Rust MLフレームワーク）

P6の基盤機能は標準提供。ただしsparse embedding更新最適化がなく、
FT層（6万行×256次元）の毎バッチ更新がボトルネックになるリスクが
ある。API安定性も発展途上。

### 案C: PyTorchに移行

全機能が成熟。sparse gradient対応あり。NNUE学習の実績豊富。
P7の構造実験にも自動微分で即対応できる。ADR-0039で想定済みの
移行パス。

## Decision

案Cを採用する。学習器をPyTorchで書き直す。

### 移行の範囲

Pythonに移すもの:
- 学習ループ全体（optimizer, lr schedule, checkpoint, logging）
- モデル定義（HalfKP + 利き塔 + 隠れ層）
- 量子化・.hmwr書き出し

Rustに残すもの:
- 推論エンジン（engine crateのnnueモジュール）
- PSVツール（psv shuffle, psv head, psv stat）
- SPRT対局（selfplay）
- Rust学習器（勾配・量子化の参照実装。ADR-0039の方針どおり）

境界はPSV形式（入力、ADR-0038）と.hmwr形式（出力、ADR-0037）。
両形式ともPython側でも読み書きを実装し、Rust実装との
roundtrip一致をテストする。

### プロジェクト構成

```
training/
  train.py          # エントリポイント
  model.py          # NNUEモデル（nn.Module）
  dataset.py        # PSVデータセット・DataLoader
  quantize.py       # f32→量子化・.hmwr書き出し
  requirements.txt
```

### モデル定義

nn.Moduleで現アーキテクチャ（ADR-0034）を再現する。

- FT層: nn.EmbeddingBag(FT_IN, FT_OUT, mode='sum', sparse=True)
  × 2。sparse=Trueでtouched行のみ勾配更新
- 利き塔: nn.EmbeddingBag(EFFECT_IN, EFFECT_OUT, mode='sum',
  sparse=True)
- 隠れ層: Linear(CONCAT, 32) → clamp(0,1) → Linear(32, 32)
  → clamp(0,1) → Linear(32, 1)
- 損失: BCE。sigmoid(score/600)とgame_resultのλ混合ターゲット

### 学習基盤

PyTorch標準機能で構成する。

- lr schedule: LambdaLRでwarmup + cosine decay
- チェックポイント: torch.save / torch.load
- ログ: TensorBoard（SummaryWriter）
- early stopping: valid loss監視のPythonロジック

### 検証

1. Rust参照実装との順伝播一致: 同一重み・同一局面で評価値を比較
2. P5のhao_v6相当の学習再現: valid lossが同等以下であることを確認
3. 量子化のroundtrip一致: Python書き出し→Rust読み込み→評価一致

## Consequences

- P6の基盤機能が既製品で手に入る
- P7の構造実験で自動微分が使える
- nn.EmbeddingBag(sparse=True)でtouched行限定更新が実現できる
- GPU利用が将来の選択肢に入る（.to('cuda')で切替可能）
- Python依存が増える。ただし推論・対局はRustのまま独立して動く
- Rust学習器は参照実装として残し、量子化の正しさ検証と
  フレームワーク非依存の再現性を担保する
