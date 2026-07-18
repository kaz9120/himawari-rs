# 0021: 時間管理

- Status: accepted
- Date: 2026-07-18
- 関連ADR: [0019](0019-usi-architecture.md), [0020](0020-search-threading.md)

## Context

USIの `go` は btime/wtime（残り時間）、byoyomi（秒読み）、
binc/winc（加算）、movetime、infinite、ponderを渡してくる。
将棋の持ち時間は「切れ負け」「秒読み」「フィッシャー」の3形態が
混在し、チェスエンジンの時間管理をそのまま使えない。
時間切れは即負けなので、通信遅延のマージン設計も必須になる。

## 選択肢と比較

### 案A: 1手あたり固定配分

残り時間÷固定手数。実装は簡単だが、探索が安定した局面で
時間を捨て、荒れた局面で時間が足りない。

### 案B: optimum / maximum の2段階（Stockfish型）

「ふつうはoptimumで打ち切り、必要ならmaximumまで延びる」の
2水準を持つ。反復深化のイテレーション境界でoptimumを判定し、
探索中の強制打ち切りはmaximumで行う。手ごとの柔軟性が出る。

## Decision

案Bを採用する。定義は次のとおり。

- 計時はgo受信時刻を起点とする。ponder中は消費せず、
  ponderhitで起点を引き継ぐ
- 1手に使える時間の総枠:
  `avail = 残り時間 / max(残り想定手数, 16) + byoyomi + inc`。
  残り想定手数は `max(48 − game_ply / 2, 16)` の簡易式から始める
- `optimum = avail`、`maximum = min(avail × 3, 残り時間 + byoyomi)`。
  係数はP3でSPRTにより調整する（このADRでは初期値のみ決める）
- マージン: 毎手 `NetworkDelay`（既定120ms）を消費見積もりに
  加算し、切れ負けの終盤では `NetworkDelay2`（既定1120ms）を
  残して指す。いずれもUSIオプション（やねうら王互換の名前と既定値）
- 秒読み将棋で残り時間0のとき: `byoyomi − NetworkDelay2` を
  そのまま1手の上限とする
- `movetime` は optimum = maximum = movetime − NetworkDelay。
  `infinite` はstopが来るまで探索する
- 判定の実装: optimumは反復深化のイテレーション完了時に
  メイン探索スレッドが判定。maximumはメイン探索スレッドが
  数千ノードごとに現在時刻を見てstopフラグを立てる（ADR-0020）

## Consequences

- 時間切れ負けの防御がNetworkDelay/NetworkDelay2の2定数に
  集約され、ネットワーク対局（floodgate等）でも数値の調整だけで
  対応できる
- 配分式は最初は素朴でよい。P3のSPRT基盤ができた後、
  「同時間で強くなる」変更として係数をチューニングする
- ponderhitの計時引き継ぎはP2ではUSI_Ponder=false運用で
  実質使われない。実装だけ先に正しく入れておき、ponder有効化
  （MultiPV・ponderのADR、P3）で検証する
- GUIごとのbyoyomi解釈の差（stime等の拡張）はスコープ外。
  問題が出たGUIごとに対応を判断する
