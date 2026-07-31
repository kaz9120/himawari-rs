# 0050: singular extension（TT手の単独延長）に再挑戦する

- Status: accepted
- Date: 2026-07-21
- 関連ADR: [0028](0028-pruning-extensions.md), [0022](0022-transposition-table.md), [0046](0046-correction-history.md), [0047](0047-continuation-history.md)

## Context

探索改善キャンペーンの第5弾で、過去失敗組の再挑戦第1号。
P3では2546局で-16.0 [-29.4,-2.6]の明確なマイナスだった
（4c8e538）。当時の分析は「除外手つき検証探索のコストに
TTエントリの質が釣り合っていない」。

前提が変わった。評価は駒割→NNUE（対駒割+528 Elo）になり、
correction history・continuation historyでTTに入る値と
オーダリングの質が上がった。TT手が本当に「唯一の良い手」で
ある局面を検証するコストが、当時より回収しやすい。
SF系ではsingular extensionは延長系で最大の利得源であり、
ROADMAPの候補でも再挑戦を予定していた。

## 選択肢と比較

### 案A: SF簡易形（単独延長のみ）

TT手の除外手つき検証探索を行い、fail-lowなら+1延長する。
double extension・negative extension・multi-cutは入れない。
判定単位が明確で、キャンペーンのチューニングなし方針に合う。

### 案B: SF完全形（double/negative extension、multi-cut込み）

利得は最大だが、係数が多くチューニングと不可分。案Aで
土台の成否を判定してから積む。

### 案C: P3実装の条件再現

当時の実装は残っていないため再現の意味が薄い。案Aとして
書き直す方が明快。

## Decision

案Aを採用する。

### 実装スケッチ（search.rs）

発動条件（すべて満たすとき、TT手に対して検証探索）:
- depth >= 7
- root/除外手つき探索中でない
- TT手あり、かつ`pos.is_legal(tt_move)`
- TTのboundがUPPERでない（lower/exact）
- TTのdepth >= depth - 3
- |TT値| が詰み圏でない

検証探索:
- `singular_beta = tt_value - 2 * depth`（cpスケール）
- TT手を除外した`search(depth/2, singular_beta-1, singular_beta)`
  をnon-PVで実行
- 結果が`< singular_beta`（他のすべての手がfail-low）なら
  TT手の探索深さを+1する（王手延長とは重複させず、
  `max(王手延長, singular延長)`とする）

除外手（excluded move）の配管:
- 探索関数に除外手を渡す（plyごとのスタックでもパラメータでも、
  既存の流儀に合わせる）
- 除外手つき探索では、(1) ムーブループで除外手をスキップ、
  (2) TTカットしない（probeはstatic eval再利用のため可）、
  (3) TT storeしない、(4) NMP・RFPをスキップ、
  (5) correction history更新をスキップ

初期定数（チューニングしない）: depth >= 7、tt_depth >= depth-3、
margin = 2*depth、検証深さ = depth/2。SF系の実績値。

### 検証

- 機能検証: 除外手つき探索がTT・履歴を汚染しないこと
  （同一局面の連続探索で結果が安定していること）
- SPRTはADR-0028の既定条件。両エンジンに
  `--option "EvalFile=data/halfkp_180M.hmwr.best"`

## Consequences

- 検証探索のコストで生ノードあたりの速度は下がる。延長の
  質で回収する構造なので、NPSでなくSPRTだけで判定する
- 除外手の配管は将来のmulti-cut（検証探索がbeta超えなら
  複数手が良い=カット）にも流用できる。案Bの拡張は本ADRの
  H1採択後に別ADRで検討する
- H0の場合、P3の分析（TT質不足）に加えて「NNUE時代でも
  この形は合わない」という情報が残る。margin・深さ条件を
  変えた再挑戦はチューニング段階に回す
- 見直しトリガー: 本格学習（P8後段）でネットが強くなったとき、
  延長条件の再検証
