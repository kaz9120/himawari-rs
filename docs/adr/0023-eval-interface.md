# 0023: 評価関数インターフェース

- Status: accepted
- Date: 2026-07-18
- 関連ADR: [0002](0002-cargo-workspace.md), [0014](0014-position-structure.md)

## Context

P2の評価は駒割ベースの簡素なもので始め、P4でNNUEに置き換える。
探索コードを書き換えずに評価関数を差し替えられる境界を、
NNUEの要件（plyに沿ったaccumulatorスタック、DirtyPieceの消費、
玉移動時のrefresh）を先取りした形で固定するのがこのADRの目的。

## 選択肢と比較

### 案A: trait object（dyn Eval）

差し替えは柔軟だが、葉ごとの仮想呼び出しコストと、
探索関数へのライフタイム・所有権の引き回しが煩雑。

### 案B: ジェネリクス（探索関数をEvalで単相化）

コストゼロだが探索コード全体に型パラメータが感染し、
ビルド時間とコードサイズが倍々になる。

### 案C: enumディスパッチ

`enum Evaluator { Material(..), Nnue(..) }` をスレッドローカルに
持ち、matchで分岐する。分岐は1回で予測が効き、型の感染がない。
バリアント追加時はenumに1行足す。

## Decision

案Cを採用する。インターフェースは次の4操作に固定する。

```
new_search(&mut self, pos: &Position)   // 探索開始時の全計算（NNUE: refresh）
push(&mut self, pos: &Position)         // do_move直後。StateInfoのDirtyPieceを消費
pop(&mut self)                          // undo_move直前
evaluate(&mut self, pos: &Position) -> Value  // 手番視点の評価値
```

- 探索はdo_move/undo_moveと対にpush/popを必ず呼ぶ（契約）。
  NNUE（P4）はこのフックでaccumulatorスタックを進める。
  ADR-0014の「accumulatorはnnueクレートのplyスタック」への布石
- P2の実装は `Material`: StateInfoのmaterial（差分計算済み、
  ADR-0014）に手番ボーナス（tempo、初期値20）を加えて返す。
  push/popは何もしない
- 評価値のスケールは歩=90を基準とするセンチポーン風の整数
  （ADR-0014のPIECE_VALUEと同一スケール）
- 簡易PSQTはP2では入れない。駒割のみで対局完走・詰将棋正答の
  出口条件は満たせる。位置評価の初出はP4のNNUEとする

## Consequences

- 探索コードは `Evaluator` のenumだけを知り、NNUE導入（P4）は
  バリアント追加＋4操作の実装で完結する
- push/popの呼び忘れはNNUE導入まで顕在化しない。P2の時点から
  Material側でもply深度カウンタを持ち、do/undoとの整合を
  debug_assertで検査しておく
- 駒割のみの評価は序盤で無方針になるが、P2の出口条件（完走・
  詰将棋）には影響しない。P4までの対局はテスト用途に割り切る
