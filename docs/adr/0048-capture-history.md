# 0048: capture history（取る手の履歴）を導入する

- Status: accepted
- Date: 2026-07-21
- 関連ADR: [0025](0025-move-ordering.md), [0028](0028-pruning-extensions.md), [0047](0047-continuation-history.md)

## Context

探索改善キャンペーンの第3弾。取る手のオーダリングは現在、
MVV-LVA + 成り加点の静的スコアだけで決めている
（`movepick.rs:183-191`のcapture_score）。同点の取る手が
多い局面で、実際にカットを生んだ手を優先する情報がない。
quiet手はADR-0047までで履歴3本（main + 文脈2本）を持つのに、
取る手は履歴ゼロという非対称も残っている。

capture historyは「この駒がこのマスでこの駒種を取る」ことの
成否を履歴で持つ。SF系ではcapture ordering・SEE枝刈りの
補助として定着している。

## 選択肢と比較

### 案A: capture history単体

[取る側piece_after 32][to 81][取られる駒種 16]のテーブル1本。
約83KBで軽い。静的スコアへの加算のみ。

### 案B: 案A + SEE閾値やLMRへの波及

SFはcapture historyを枝刈り閾値にも使うが、閾値系は
パラメータ調整と不可分で、キャンペーンの「チューニングなし」
方針に反する。オーダリング専用の案Aで判定し、波及は
効果を見てから別ADRにする。

## Decision

案Aを採用する。

### 実装スケッチ

テーブル（movepick.rs）:
- `CaptureHistory { table: Box<[i16]> }`（フラット確保、
  32×81×16 = 41,472エントリ、約83KB）
- 添字: 取る側の（piece_after、to）と、取られる駒の
  piece_type（成りは成りのまま。piece_type 16種）
- 更新はgravity（クランプ±4000、divisor 16384。既存と同一）
- 保持はスレッドローカル（貸し出し・回収・NewGameクリア）

スコアリング:
- 取る手のスコアを `capture_score(pos, m) + capt_hist.get(...)` に
  変更する。適用箇所はCapturesInit（`movepick.rs:286`）、
  QCapturesInit（`movepick.rs:381`）、evasionの捕獲部
  （`movepick.rs:360`）の3箇所すべて（読み取りのみ）

更新（main searchのみ）:
- 探索ループでtried_captures（試行した取る手）を記録する
- best_moveが取る手でカットしたとき、その手にbonus
  （`depth*depth + 2*depth`、quietと同一式）、tried_capturesの
  他の手に-bonus
- best_moveがquietのときも、tried_capturesに-bonusを与える
  （SFと同じ。取る手を差し置いてquietが勝った事実の反映）
- qsearchでは更新しない

初期定数（チューニングしない）: bonus式・クランプ・divisorは
既存履歴と同一。スコアは静的スコアへの等重み加算。

### 検証

SPRTはADR-0028の既定条件（tc 10+0.1、elo0=0/elo1=5、並列8、
adjudicate 2000,8）。両エンジンに
`--option "EvalFile=data/halfkp_180M.hmwr.best"`。

## Consequences

- メモリ増は83KB/スレッドで無視できる
- GoodCaptures/BadCapturesの振り分け自体はSEE（see_ge）のままで、
  履歴はSEE同符号内の順序にだけ効く。SEE閾値への波及
  （bad capture救済など）は効果確認後に別ADRで検討する
- 見直しトリガー: 案Bの枝刈り波及。またcapture history導入で
  MVV-LVAの16倍係数とのバランスが悪い兆候（履歴が常に飽和する
  など）が見えたら、スケールを別ADRで再設計する
