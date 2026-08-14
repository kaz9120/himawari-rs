# 0125: 探索本体を責務ごとに切り出す

- Status: accepted
- Date: 2026-08-01
- 関連ADR: [0074](0074-feature-verification.md), [0109](0109-reference-parity.md), [0119](0119-g10-book.md)

## Context

`crates/engine/src/search.rs` が2810行あり、`Worker::search` 1つで1006行を占める。
[ADR-0109](0109-reference-parity.md)〜[0119](0119-g10-book.md)の追従11群で機能を積み上げた
結果である。**動いているが、次の変更を入れる足場としては読みにくい。**

200行を超える関数を数えると5つある。

| path:line | 関数 | 行数 |
|---|---|---|
| `search.rs:1331` | `Worker::search` | 1006 |
| `search.rs:811` | `Worker::iterate` | 387 |
| `search.rs:2337` | `Worker::qsearch` | 283 |
| `thread.rs:290` | `spawn_worker` | 234 |
| `selfplay/main.rs:86` | `parse_args` | 207 |

責務が混ざっている箇所もある。`spawn_worker` の1つのクロージャに、ジョブ受信・
`TimeManager` の構築・`Worker` の組み立て・**USI info行の整形**・**投票と投了とbestmove出力**
が同居している。探索スレッドの関数の中にUSIプロトコルの出力が埋まっている。

`iterate` にも、思考時間の伸縮5係数の計算（`search.rs:1076-1135`）が入っている。
その定数群（`FALLING_*`・`TIME_REDUCTION_*`・`INSTABILITY_*`・`EFFORT_*`）は
`search.rs:181-215` にあるが、本来は `timeman.rs` の持ち物である。

## 選択肢と比較

### 案A: 触らない

動いているものを触るとバグが入る。探索の変更は棋力に直結し、機能検証で一致を確かめても
「一致すること」しか分からない。

### 案B: 責務で切り出す

参照実装の Step 番号がコメントで刻まれているので、切断線は既に可視化されている。
ロジックを動かさずに関数化できる。機能検証（[ADR-0074](0074-feature-verification.md)）で
全局面のノード数が一致すれば、探索に影響していないことを機械的に確かめられる。

### 案C: 設計から書き直す

責務を整理して構造を作り直す。読みやすさは最も上がるが、参照実装との対応が切れる。
追従で入れた機能を原典と突き合わせられなくなり、次の追従が難しくなる。

## Decision

案Bを採る。

**決め手は、切断線がすでにコメントで引かれていることである。** 参照実装のStep番号に沿って
ブロックが閉じており、機械的に関数へ移せる。新しい設計を考える必要がない。

案Cを採らないのは、参照実装との対応を保つ価値が大きいからである。追従は一巡したが、
[ADR-0114](0114-g5-singular.md)のsingular多段化と時間配分の分母が保留として残っている。
原典の行番号で照合できる状態を壊さない。

### 切り出す境界

`Worker::search` から、既存のブロック境界のまま次を出す。

| 現在の範囲 | 役割 | 切り出す先 |
|---|---|---|
| `1355-1376` | 千日手・最大手数・宣言勝ち・mate distance pruning | `terminal_value`（`qsearch:2346-2361` と重複しており、共通化できる） |
| `1413-1465` | TT probe・TTカット・カット時のhistory更新 | `probe_tt` と `on_tt_cutoff` |
| `1561-1769` | razoring / RFP / NMP / IIR / ProbCut | `prune_before_moves`（209行が単一の `if !in_check` ブロックとして閉じている） |
| `1783-1880` | singular extensionの3分岐 | `singular_extension` |
| `1937-2034` | 浅い枝刈り | `prune_shallow` |
| `2036-2081` | リダクション項1〜11 | `reduction_amount` |
| `2229-2330` | statistics更新・TT store・correction history更新 | `finalize_node` |

残るムーブループ本体が150行程度になり、探索の骨格が1画面に収まる。

`spawn_worker` からはUSI出力を追い出す。info行の整形と、投票・投了・bestmove出力を
`ThreadPool` 側へ移す。探索スレッドの関数がプロトコルを知らない状態にする。

`iterate` の時間伸縮の計算は `timeman.rs` へ移し、定数もそこへ連れて行く。

### 検証

**機能検証で全局面のノード数が一致することを必須とする。** 一致しなければ切り出しで
挙動が変わっており、その変更は取り消す。評価値と最善手も一致すること。

SPRTは行わない。ノード数がビット一致する変更は、探索に影響していない
（[ADR-0074](0074-feature-verification.md)）。

## Consequences

- 次の変更を入れる足場が読みやすくなる。1006行の関数へ機能を足すより、責務ごとの関数へ
  足すほうが影響範囲を見切りやすい
- 参照実装との対応は保たれる。Step番号のコメントは関数へ移すので、原典の行番号で照合できる
- 関数呼び出しが増える。探索のホットパスなので、インライン化されなければNPSが落ちる。
  機能検証はノード数しか見ないため、**NPSも併せて測る**。落ちていたら `#[inline]` を検討する
- 時間管理の定数が `timeman.rs` へ移る。`search.rs` から時間の判断が消え、探索と時間管理の
  境界がはっきりする。[ADR-0116](0116-g7-timeman.md)で保留にした時間配分の分母を再訪する
  ときの足場になる
