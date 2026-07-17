# 0002: cargo workspace構成とクレート分割

- Status: accepted
- Date: 2026-07-17
- 関連ADR: [0001](0001-adr-process.md), [0006](0006-ci-test-bench.md)

## Context

将棋エンジン本体のほかに、perft、教師データ生成（gensfen）、学習器、
SPRT対局マネージャなど複数のバイナリを作る計画である。
コードの依存方向を最初に固定しないと、探索や評価のコードが盤面表現に
逆流し、ツール類が重い依存を抱えることになる。

## 選択肢と比較

### 案A: 単一クレート＋feature flags

やねうら王（単一バイナリ＋コンパイルスイッチ）に近い構成。ビルド設定は単純だが、
依存方向をコンパイラで強制できず、feature の組み合わせ爆発でCIが複雑になる。

### 案B: workspaceでレイヤー分割

盤面表現を独立クレートにし、依存方向を core ← nnue/engine ← usi/tools に固定する。
クレート境界がAPI境界になり、ツール類は必要な層だけに依存できる。
境界をまたぐリファクタリングのコストは上がる。

## Decision

案Bを採用する。構成は次のとおり。

```
crates/
├── core/     # Square/Bitboard/Position/movegen/SFEN/Zobrist（探索非依存）
├── nnue/     # 特徴量・accumulator・推論SIMD（coreに依存）
├── engine/   # 探索・置換表・評価IF・時間管理（core, nnueに依存）
├── usi/      # プロトコル層 + bin: himawari（engineに依存）
└── tools/    # bin: perft, gensfen, trainer, selfplay
```

決め手は2点。coreを探索非依存に保つことで perft・学習器・対局マネージャが
軽い依存で書けること、unsafe境界（ADR-0004）をクレート単位で管理できることだ。

クレートは必要になったフェーズで作る。Phase 0では core と usi の雛形のみ置き、
nnue/engine/tools はP2以降で追加する。

## Consequences

- 依存方向がCargoで強制され、レイヤー違反がコンパイルエラーになる
- クレート間のAPIは意識的に設計する必要がある。core の公開APIは
  P1のADR群で決める
- インライン化はLTO（リリースプロファイルで有効）に委ねる。
  クレート境界がホットパスの性能問題になった場合は、この決定を見直す
