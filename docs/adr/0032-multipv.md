# 0032: MultiPV

- Status: accepted
- Date: 2026-07-19
- 関連ADR: [0019](0019-usi-architecture.md), [0024](0024-search-v1.md)

## Context

検討モード（MultiPV）はGUI利用に必要な機能で、ルール完全対応
（P3出口）の一部。棋力には寄与しないため、既存探索を壊さない
ことの担保が主な論点になる。

ponderは当初このADRに含めていたが、相手番思考は持ち時間を
実質的に増やす棋力機能であり、ponderhit後の時間配分や
「ponder中に詰みを見つけて即bestmoveを返す2手指し」のような
定番バグへの防御を含めて独立に設計すべきものなので、
別ADRに分離した（バックログ参照）。

## Decision

### root movesの構造化

現在の `Vec<Move>` を `RootMove { mv, score, prev_score, pv }` の
Vecに置き換える。反復深化のイテレーションごとにスコアと
PVを保持し、MultiPVのランキングと安定ソートに使う。
この構造はLazy SMP（ADR-0031）のワーカー間比較にも使う。

### MultiPV

- USIオプション `MultiPV`（spin、既定1）を追加する
- 反復深化の各深さで、上位K手を1手ずつ除外リストに入れながら
  K回探索する（Stockfish方式）。K=1のときは現行と同一の
  経路になるよう実装する
- info出力に `multipv k` を含める。K=1では省略（現行互換）

### 検証

- K=1が既存探索と同一ノード数になることをテストで固定する
  （回帰防止）。K>1はPVの重複がないこと、スコアが降順である
  ことを検査する
- 棋力ゲートは非劣性SPRT（K=1経路が等価であることの確認）

## Consequences

- RootMove化は探索の可読性を上げ、後続ADR（Lazy SMP・ponder）の
  土台になる
- MultiPVのK>1は探索効率が落ちるのが正常。検討モード専用で、
  対局時はK=1を前提とする
