# 0035: NNUE差分計算（accumulator）

- Status: proposed
- Date: 2026-07-19
- 関連ADR: [0014](0014-position-structure.md), [0023](0023-eval-interface.md), [0034](0034-nnue-architecture.md)

## Context

NNUEの速度はFT出力（accumulator）の差分更新で決まる。
1手で変わる特徴は高々数個なので、全計算（約1500特徴の和）を
避けて加減算だけで更新する。coreのDirtyPiece（ADR-0014で
実装済み。1手で変化する駒は最大2、玉移動フラグ付き）が
このための材料になる。決めるのは更新のタイミングと
refreshの扱い。

## 選択肢と比較

### 更新タイミング

案A（即時）: do_moveのたびにaccumulatorを更新する。実装が
単純だが、王手中でevaluateしないノードや枝刈りで即returnする
ノードの更新が無駄になる。

案B（遅延）: evaluateが呼ばれた時点で、計算済みの祖先まで
遡って差分を適用する（Stockfish方式）。無駄がないが、
スタックの計算済みフラグ管理が複雑になる。

案Bを採用する。枝刈り（ADR-0028）で大半のノードはevaluateに
到達しないため、差は大きい。

## Decision

- Evaluator::push/pop（ADR-0023の契約）でaccumulatorスタックを
  積み、各段に「計算済みか」フラグとDirtyPieceを保持する
- evaluate時: 計算済みの最も近い祖先から現在までのDirtyPieceを
  順に適用する（遡り差分）。祖先に計算済みがなければ全計算
- HalfKPは自玉の移動で自視点の特徴が全部変わるため、
  玉移動（DirtyPiece.king_moved）を挟む場合は当該視点を
  全計算（refresh）する。相手視点は差分でよい
- null move（ADR-0028）はDirtyPiece空で積み、視点の入れ替えのみ
- 検証: ランダムプレイアウトで「差分計算 = 全計算」の完全一致を
  全局面で照合する（P4出口条件）。玉移動・駒打ち・成り・捕獲の
  組み合わせを網羅するケースも固定で持つ

## Consequences

- 差分・全計算の一致テストが正解基準になるため、SIMD化
  （ADR-0036）やリファクタの回帰を機械的に検出できる
- 遡り差分の複雑さはスタック構造に閉じる。探索側は
  push/pop/evaluateの契約のまま変更不要（ADR-0023の狙いどおり）
- 玉移動refreshのコストは将棋では無視できない。入玉将棋で
  refresh頻度が上がる点は、P5以降のFinny table（視点別の
  差分基点キャッシュ）導入余地として記録しておく
