# 0049: eval hash（評価値キャッシュ）を導入する

- Status: accepted
- Date: 2026-07-21
- 関連ADR: [0022](0022-transposition-table.md), [0028](0028-pruning-extensions.md), [0046](0046-correction-history.md)

## Context

探索改善キャンペーンの第4弾。同一局面のNNUE評価を繰り返し
計算している。main searchはTTのeval欄で再利用できるが
（ADR-0046で生値保存を維持）、qsearchのstand patは毎ノード
`evaluate()`を全計算する（`search.rs`のqsearch入口）。探索
ノードの大半はqsearchであり、置換の多い将棋では同一局面の
再訪も多い。局面キー→評価値の専用キャッシュ（eval hash）は
やねうら王系で定着した手法で、評価計算の森を丸ごと省ける。

## 選択肢と比較

### 案A: 共有eval hash（TTと同じ常駐共有テーブル）

`AtomicU64`の配列1本。エントリは上位32bit=キーの上位32bit、
下位32bit=評価値（i32）。評価は局面のみの関数なので全スレッドで
共有でき、Lazy SMPの再訪にも効く。XOR検証なしの単純比較で、
偽ヒット確率は probe あたり2^-32。

### 案B: スレッドローカルeval hash

競合ゼロだが、スレッド間の再訪を活かせずヒット率で劣る。
メモリもスレッド数倍かかる。

### 案C: qsearchでもTTのevalを使う（テーブル新設なし）

qsearchのTT probe/storeを拡充する案。テーブルは増えないが、
TTエントリはbound/move/depthを含み評価専用より重く、qsearchの
書き込み増でmain searchのエントリを押し出す副作用がある。
eval専用テーブルの方が用途が明確で干渉しない。

## Decision

案Aを採用する。

### 実装スケッチ

- `EvalHash { table: Vec<AtomicU64> }`。サイズは固定64MB
  （2^23エントリ）。USIオプションは設けない（チューニングなし方針。
  Hashとは独立）
- エントリ: `(key上位32bit << 32) | (eval as u32)`。probeは
  上位32bit一致で採用。空エントリは0（key上位32bit==0の局面は
  常にミス扱いで実害なし）
- 保持は`Shared`（TTと同居、`thread.rs`のnew_gameでクリア）
- 適用箇所は生評価の計算点2つ:
  - qsearch stand pat: probe→ヒットなら`evaluate()`を省略、
    ミスなら計算してstore
  - main searchのTTミス時の`evaluate()`: 同様にprobe/store
  - correction historyの補正（ADR-0046）はキャッシュの外側で
    従来どおり適用する（キャッシュは生値のみ持つ）
- 詰み圏の値は入らない（evaluateの出力域のみ）ので特別扱い不要

### 検証

- 機能検証: eval hash有効/無効で探索結果（ノード数・PV）が
  一致することを数局面で確認する（偽ヒットを除けば探索は不変の
  はず。不一致が出たら偽ヒット以外のバグを疑う）
- NPS計測: 中盤局面でのbefore/after
- SPRTはADR-0028の既定条件。両エンジンに
  `--option "EvalFile=data/halfkp_180M.hmwr.best"`

## Consequences

- メモリ+64MB（共有1本）。Hash 256MBと合わせても許容範囲
- 偽ヒット（2^-32/probe）は理論上探索を汚染しうるが、NNUE評価の
  ±1違い程度の影響と同水準で、実用上無視する（やねうら王系と
  同じ割り切り）
- 評価値キャッシュができると、将来の「教師局面のqsearch静止化」
  （gensfen自前実装）でも流用できる
- 見直しトリガー: スレッド数を大きく増やしてfalse sharingが
  見えたとき。またFT差分計算の構造を変えるとき
