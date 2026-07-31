# 0047: continuation history（手系列条件の履歴）を導入する

- Status: accepted
- Date: 2026-07-21
- 関連ADR: [0025](0025-move-ordering.md), [0028](0028-pruning-extensions.md), [0046](0046-correction-history.md)

## Context

探索改善キャンペーンの第2弾。quiet手のオーダリングは現在、
main history（移動後の駒32×移動先81。`movepick.rs:9-38`）だけで
スコアしている（`movepick.rs:270-278`のQuietsInit）。counter moveは
「直前の手→応手1手」の上書きテーブル（`movepick.rs:76`）で、
候補を1手挙げる以上の情報を持たない。

continuation historyは「直前の手（または2手前の自分の手）が
これだったとき、この応手が良かったか」をスコアで持つ。
文脈条件付きのオーダリングでmain historyより解像度が高く、
SF系ではオーダリング改善の主力になっている。

## 選択肢と比較

### 案A: 1-ply（counter move history）のみ

直前の相手の手を条件にするテーブル1本。実装最小。
2手前（自分の前の手との連携、follow-up）を取りこぼす。

### 案B: 1-ply + 2-ply（counter + follow-up）

SFと同じく、条件手の（駒、移動先）を外側添字にした単一テーブルを
共有し、1手前と2手前の両方から引いて加算する。ROADMAPの候補でも
counter/follow-upを1案として扱っており、1SPRTで判定する
単位として自然。

### 案C: SF完全形（王手中・捕獲別の分離、offset 3/4/6も）

効果は最大だが実装・メモリが重く、初回導入の判定単位としては
過剰。案Bで効果を確認してから拡張を検討すればよい。

## Decision

案Bを採用する。

### 実装スケッチ

テーブル（movepick.rs）:
- `ContinuationHistory { table: Box<[[[[i16; 81]; 32]; 81]; 32]> }`
  外側=条件手の（piece_after 32、to 81）、内側=応手の
  （piece_after 32、to 81）。約13.4MB/スレッド
- 更新はmain historyと同形式のgravity
  （クランプ±4000、divisor 16384。`movepick.rs:33-37`と同じ）
- 保持はHistory/CounterMovesと同じ流儀: スレッドローカル、
  goごとに貸し出し、NewGameでクリア

search.rs:
- plyごとの指し手スタック`move_stack`をWorkerに追加
  （do_move時に記録。null moveはMove::NONE）
- quietスコア: `history.get(m) + cont.get(prev1, m) + cont.get(prev2, m)`
  （prev1=1手前、prev2=2手前。NONEなら加算しない）。
  MovePicker::nextにcontと前2手を渡す
- 更新: `update_quiet_stats`で、成功したquiet手にbonus、
  試行済みquiet手に-bonusを、main historyと同時に
  prev1/prev2の両方の文脈へ与える（bonus式は既存の
  `depth*depth + 2*depth`のまま）

初期定数（チューニングしない）: bonus式・クランプ・divisorは
すべて既存main historyと同一。スコア合算は等重み。

### 検証

SPRTはADR-0028の既定条件（tc 10+0.1、elo0=0/elo1=5、並列8、
adjudicate 2000,8）。両エンジンに
`--option "EvalFile=data/halfkp_180M.hmwr.best"`。

## Consequences

- メモリが約13.4MB/スレッド増える（8スレッドで約107MB）。
  許容範囲だが、スレッド数を大きく増やす場合は再考する
- move_stackは後続の改善（capture history、NMPの検証探索、
  多重延長の文脈条件）でも使う基盤になる
- H1採択後、既存のCounterMoves（候補1手のテーブル）が
  冗長になる可能性がある。除去は別途、非劣性SPRT
  （elo0=-5/elo1=0）で判定する
- 見直しトリガー: 案C（王手中・捕獲別の分離、深いoffset）は
  本テーブルの効果が確認できたら検討する
