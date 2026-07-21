# 0051: ProbCut（浅い探索による上限カット）を導入する

- Status: accepted
- Date: 2026-07-21
- 関連ADR: [0028](0028-pruning-extensions.md), [0049](0049-eval-hash.md), [0050](0050-singular-extension-retry.md)

## Context

探索改善キャンペーンの第6弾。beta+マージンを大きく超えそうな
局面では、浅い確認探索で「十分良い取る手が1つある」ことを
示せれば、高深度の全探索を省略してカットできる。NNUE評価の
精度が前提の枝刈りで、SF系で定着している。P3では評価が駒割
だったため見送っていた（IDEAS.md）。

## 選択肢と比較

### 案A: SF簡易形（取る手のみ、固定マージン）

non-PV・非王手・depth>=5で、`probcut_beta = beta + 200`を
閾値に、SEE>=0の取る手だけをqsearch→浅い通常探索の2段で
確認する。係数は2つ（マージン200、深さ削減4）で固定。

### 案B: SF完全形（確率的マージン、historyによる手の選別）

マージンをdepth依存にし、対象手を広げる形。チューニングと
不可分なので、案Aで土台の成否を判定してから検討する。

## Decision

案Aを採用する。

### 実装スケッチ（search.rs）

発動条件（ムーブループ前、NMPの後）:
- non-PV、非王手、除外手つき探索中でない（ADR-0050の配管）
- depth >= 5
- |beta| < VALUE_MATE_IN_MAX_PLY
- `probcut_beta = beta + 200`
- TTに深い情報があり矛盾する場合はスキップ:
  tt_hitで`tt_depth >= depth - 3`かつ`tt_value < probcut_beta`
  なら発動しない

本体:
- 取る手（SEE >= 0のもの）を生成し、各手について
  1. do_moveしてqsearchを窓`(-probcut_beta, -probcut_beta+1)`で
     実行。`-value >= probcut_beta`でなければ次の手へ
  2. 通ったら同じ窓で通常探索`depth - 4`を実行
  3. それでも`-value >= probcut_beta`なら、その値でカット
     （fail-softで値を返す）。TTにはlower bound・depth-3として
     保存する
- 確認に使う手は最大数を制限しない（SEE>=0の取る手は少ない）

初期定数（チューニングしない）: マージン200、確認探索の
深さ削減4、depth >= 5。SF系の実績値。

### 検証

SPRTはADR-0028の既定条件。両エンジンに
`--option "EvalFile=data/halfkp_180M.hmwr.best"`。

## Consequences

- 高いbetaのノードを浅い探索で刈るため、終盤の駒得局面で
  効きやすい。逆に静かな局面ではSEE>=0の取る手が少なく
  コストは小さい
- マージン200はNNUE評価のスケールに依存する。ネットを
  再学習してスケールが変わったら再確認する（見直しトリガー）
- 案B（マージンのdepth依存化、対象手の拡大）はH1採択後の
  チューニング段階で検討する
