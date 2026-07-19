# 0036: NNUE量子化とSIMD実装

- Status: proposed
- Date: 2026-07-19
- 関連ADR: [0003](0003-toolchain.md), [0004](0004-unsafe-policy.md), [0034](0034-nnue-architecture.md), [0035](0035-nnue-accumulator.md)

## Context

NNUE推論は整数量子化とSIMDで初めて実用速度になる。
既存公開評価関数の読み込み互換（ADR-0034）を守るため、
量子化スキームは選択の余地がなく標準に従う。論点はSIMDの
実装手段と、正しさの担保方法。

## Decision

### 量子化（やねうら王互換で固定）

- FT: 重み・バイアスint16、accumulatorはint16で保持
- 隠れ層: 重みint8、バイアスint32、活性化はClipped ReLUで
  0..127のu8に飽和
- 出力: int32をFV_SCALE = 16で除算し、歩=90スケールの
  評価値に写像する

### SIMD実装

- まずスカラー基準実装を書く。これが正解器であり、
  全プラットフォームのフォールバックになる
- SIMD版は `std::simd`（portable SIMD、nightly機能）で書く。
  ツールチェインはnightly固定（ADR-0003）なので追加コストはない。
  対象はApple Silicon（NEON 128bit）と将来のx86_64
  （AVX2 256bit）で、レーン幅はstd::simdの抽象に任せる
- 固有intrinsicsへの降格（unsafe、ADR-0004の適用対象）は、
  std::simdで性能が不足すると計測で示されたときだけ検討する

### 検証

- 「SIMD = スカラーの完全一致」をランダム局面の全数照合で
  固定する（P4出口条件）。飽和・丸めの径路もビット一致を要求する
- ベンチ（ADR-0006）にevaluate単体のスループットを追加し、
  スカラー比の倍率をROADMAPに記録する

## Consequences

- 量子化を互換で固定したため、推論値の正解基準
  （公開エンジンとの一致、ADR-0034）がビット単位で成立する
- std::simdが将来stabilize/変更されてもスカラー実装が
  移行の安全網になる
- int8乗算の飽和挙動などプラットフォーム差が出やすい部分は、
  一致テストが検出する。検出されたらスカラー側を正とする
