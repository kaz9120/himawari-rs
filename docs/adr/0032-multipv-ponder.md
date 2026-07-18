# 0032: MultiPV・ponder

- Status: proposed
- Date: 2026-07-19
- 関連ADR: [0019](0019-usi-architecture.md), [0021](0021-time-management.md), [0024](0024-search-v1.md)

## Context

検討モード（MultiPV）と相手番思考（ponder）はGUI利用に必要な
機能で、ルール完全対応（P3出口）の一部。棋力には直接
寄与しないため、実装の正しさをどう担保するかが主な論点。

## Decision

### root movesの構造化

現在の `Vec<Move>` を `RootMove { mv, score, prev_score, pv }` の
Vecに置き換える。反復深化のイテレーションごとにスコアと
PVを保持し、MultiPVのランキングと安定ソートに使う。

### MultiPV

- USIオプション `MultiPV`（spin、既定1、最大200相当）を追加
- 反復深化の各深さで、上位K手を1手ずつ除外リストに入れながら
  K回探索する（Stockfish方式）。K=1のときは現行と同一の
  経路になるよう実装する
- info出力に `multipv k` を含める。K=1では省略（現行互換）

### ponder

- `USI_Ponder` 有効時、bestmoveに `ponder <相手の応手>` を
  付けて返す（PVの2手目。なければ省略）
- `go ponder` は相手番局面で通常探索を開始し、stopもoptimumも
  無視して読み続ける。`ponderhit` で計時をgo受信時刻から
  引き継ぐ（ADR-0021に実装済みの骨格を有効化）。
  `stop` が来たら即bestmoveを返す（外れた場合、GUIは
  positionを差し替えて新たにgoを送る）
- ponder中のTT・historyはそのまま次のgoに生きる（利得の源泉）

### 検証

- MultiPV: K=1が既存探索と同一ノード数になることをテストで
  固定する（回帰防止）。K>1はPVの重複がないこと、スコアが
  降順であることを検査する
- ponder: selfplayマネージャにponder対応を足すのはP3では
  やらない。GUI（ShogiHome等）での手動確認と、USIコマンド列の
  結合テスト（go ponder → ponderhit → bestmove）で担保する
- 棋力ゲートは非劣性SPRT（K=1経路が等価であることの確認）

## Consequences

- RootMove化は探索の可読性を上げ、Lazy SMP（ADR-0031）の
  ワーカー間比較にも使える
- MultiPVのK>1は探索効率が落ちるのが正常。検討モード専用で、
  対局時はK=1を前提とする
- ponderの時間管理（ponderhit後の配分）はADR-0021の式を
  そのまま使う。実戦での時間切れはNetworkDelay系の調整で吸収する
