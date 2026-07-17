# 0004: unsafe方針

- Status: accepted
- Date: 2026-07-17
- 関連ADR: [0003](0003-toolchain.md)

## Context

将棋エンジンのホットパス（指し手生成・評価・置換表アクセス）では、
境界チェックの除去やSIMD intrinsicsのためにunsafeが必要になる。
一方でロックレス置換表のような並行コードは、unsafeの使い方を誤ると
未定義動作（UB）になる。許容範囲と検証手段を先に決める。

## 選択肢と比較

### 案A: unsafe全面禁止

安全だが、SIMD intrinsicsが使えず NNUE推論の性能目標を達成できない。非現実的。

### 案B: 無制限に許容

Stockfish/やねうら王のC++流をそのまま持ち込む案。特に置換表の
「データレース許容」はRustではUBであり、コンパイラの最適化前提が崩れる。

### 案C: カテゴリ限定＋検証義務

unsafeを使途3カテゴリに限定し、書き方の規約と検証手段をセットで課す。

## Decision

案Cを採用する。unsafeの許容カテゴリは次の3つに限定する。

1. SIMD intrinsics（`std::arch`）の呼び出し（`std::simd` で性能が出ない箇所の差し替え用。ADR-0003）
2. 置換表など、スレッド間共有のためのatomic境界の内側
3. ホットパスの `get_unchecked` 等の境界チェック除去

規約は次のとおり。

- すべてのunsafeブロックに `// SAFETY:` コメントを義務付ける。
  clippyの `undocumented_unsafe_blocks` をworkspace全体でwarnにし、CIの
  `-D warnings` でエラー化する（設定済み）
- `unsafe_op_in_unsafe_fn` はdeny（設定済み）
- unsafeで除去した検査には対応する `debug_assert!` を併置する
- まずsafeで実装し、criterionのベンチ差が実証された箇所だけをunchecked化する
- 置換表はStockfish式のレース許容ではなく、`AtomicU64` のRelaxed load/storeで
  書く（詳細は置換表のADR、P2で起草）。データレースそのものを作らない

検証手段は、SIMDを含まないunsafeコードにMiriをローカルで随時適用し、
並行コードはloomまたはストレステストで検査する。
適用範囲の詳細は置換表のADRで決める。

## Consequences

- unsafeの箇所が3カテゴリに集約され、レビューと監査が容易になる
- 「safeで書いてから測って除去」の手順により、根拠のないunsafeが入らない。
  そのぶん最適化の工程は1段増える
- Miri/loomはビルド時間と手間がかかる。適用しすぎて開発が止まらないよう、
  対象を並行コードとポインタ操作に絞る
