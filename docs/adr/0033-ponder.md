# 0033: ponder

- Status: proposed
- Date: 2026-07-19
- 関連ADR: [0019](0019-usi-architecture.md), [0020](0020-search-threading.md), [0021](0021-time-management.md), [0032](0032-multipv.md)

## Context

ponderは相手番の間も探索を続ける機能で、予測が当たれば
相手の消費時間ぶんの探索を先取りできる。実質的な持ち時間の
拡大であり、棋力に直結する。一方で実装は状態機械が絡み、
「ponder中に探索が終わって即bestmoveを返す2手指し」のような
定番バグの温床でもある。このADRで決めるのは、bestmove送信の
規律、ponderhit後の時間配分、コマンド競合の整理、検証方法。

## Decision

### 基本形（USIプロトコル準拠）

- `USI_Ponder` が有効のとき、bestmoveに `ponder <予測手>` を
  付ける。予測手はPVの2手目。なければ省略する
- GUIは予測局面つきで `go ponder` を送ってくる。エンジンは
  通常探索を開始するが、時間制限を持たない（infinite相当）

### bestmove送信の規律（2手指しの防御）

- 探索スレッドの出口を「探索終了」と「bestmove送信」に分離する。
  go ponder中は、詰みを発見しても、depth上限に達しても、
  探索スレッドは結果を保持したまま待機し、bestmoveを送らない
- bestmoveを送ってよいのは `ponderhit` か `stop` を受けた後だけ。
  この規律をThreadPoolの状態機械（ADR-0020）に実装し、
  「go ponder後、ponderhit/stop前にbestmoveが出ない」ことを
  結合テストで固定する

### ponderhit後の時間配分

- 計時はADR-0021のとおりponderhitを起点に通常配分する
  （ponder中の消費はゼロ扱い）。これを初版とする
- ponder中に既に深く読めている場合の早指し
  （optimumの割引。例: ponder経過時間の一部をoptimumから引く、
  または「前イテレーションからbestが安定していたら打ち切る」）は
  棋力に効くチューニング項目として、1調整=1SPRTで後から積む
- ponderhit時点で探索が終了済み（詰み確定など）の場合は
  即bestmoveを返してよい（この時点では手番なので合法）

### コマンド競合の整理

状態×受信コマンドの挙動を固定する。

| 状態 | ponderhit | stop | quit |
|---|---|---|---|
| ponder探索中 | 通常探索に切替、計時開始 | 即bestmove | bestmove省略で終了 |
| ponder探索終了・待機 | 即bestmove | 即bestmove | 同上 |
| 通常探索中 | 無視 | 即bestmove | 同上 |

### 検証

- 結合テスト: go ponder→ponderhit→bestmove、go ponder→stop→
  bestmove、ponder中にmate発見→bestmove保留→ponderhitで送信、
  の3系列をUSIコマンド列で自動化する
- 棋力の定量にはselfplayマネージャのponder対応が必要。
  対応（予測手の管理・ponderhit/stopの送出・時計の並行進行）は
  本ADRの実装後に行い、Ponder有効 vs 無効のSPRTで効果を測る
- GUI（ShogiHome等）での手動確認も行う

## Consequences

- 探索スレッドの状態機械が複雑になる。状態遷移表をテストで
  固定することで、選手権定番の2手指し・応答なしバグを防ぐ
- ponderの利得は予測的中率に依存する。的中率はinfo経由で
  観測できるようにし、チューニングの材料にする
- selfplayマネージャのponder対応までは効果の定量ができない。
  導入初版は結合テストとGUI確認のみで取り込み、SPRTは
  マネージャ対応後に実施する（このADRの残作業として管理）
