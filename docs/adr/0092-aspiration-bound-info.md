# 0092: aspirationのfail high/lowをinfoで報告する

- Status: accepted
- Date: 2026-07-29
- 関連ADR: [0086](0086-search-observability.md), [0024](0024-search-v1.md)

## Context

aspiration窓を外れたとき、窓を広げて読み直すだけで何も報告していない。

```rust
if score <= alpha {
    beta = (alpha + beta) / 2;
    alpha = (score - delta).max(-VALUE_INFINITE);
    delta += delta / 2;
} else if score >= beta {
    ...
}
```

USIには `score cp <x> lowerbound` / `score cp <x> upperbound` があり、
将棋所はこれを評価値の増減表示に使う。オーナーが将棋所で対局を見て
「評価値のところに + や - が出ていない」と気づいた（2026-07-28）。

深さが進むと反復深化の1周に数秒かかる。その間、窓を外して読み直して
いることが外から分からない。[ADR-0086](0086-search-observability.md)で
`seldepth` と `currmove` を足したが、この経路が残っていた。

fail highは「実際の評価はこの値以上」、fail lowは「この値以下」を意味する。
評価が急に動いた局面ではこの2つが連続して出るため、探索が揺れている
ことが読み取れる。

## 選択肢と比較

### 案A: 列挙に専用のヴァリアントを足す

[ADR-0086](0086-search-observability.md)で `SearchInfo` を列挙にした
狙いがここで効く。`Bound(IterInfo, ScoreBound)` を足す。

消費側は `match` の網羅性検査で漏れが分かる。

### 案B: IterInfoにOption<ScoreBound>のフィールドを足す

最初にこちらで実装した。既存の `Iteration` をそのまま使い、フィールドで
区別する。

これだと消費側が区別を強制されない。実際、MultiPVのテストが確定ラインを
数えるつもりでfail報告まで拾い、3ラインのはずが5になって落ちた。
テストは直せるが、同じ取りこぼしが将来も起こる。列挙のヴァリアントなら
コンパイラが検出する。

## Decision

案Aを採る。

`SearchInfo::Bound(IterInfo, ScoreBound)` を足す。`ScoreBound` は
`Lower`（fail high、実際の評価はこの値以上）と `Upper`（fail low、
この値以下）を持つ。

fail lowではPVが空になりうる。alphaを超える手が1つも見つからないため
である。その場合は前の周のPV（`root_moves[pv_idx].pv`）を使う。それも
空なら報告しない。`pv` が空の `info` 行は意味がないためである。

出力の組み立ては `format_pv_line` に切り出し、確定値とfail報告で共有する。
`lowerbound` / `upperbound` はスコアの直後に置く（USIの語順）。

## 動作確認（2026-07-29）

初期局面から4手進めた局面を `go movetime 8000` で読ませ、実際に出力される
ことを確かめた。

```
info depth 5 seldepth 9 score cp 46 upperbound nodes 3662 nps 1220666 time 3 hashfull 0 pv 2f2e 8d8e 2e2d 2c2d
info depth 6 seldepth 10 score cp 58 lowerbound nodes 6120 nps 1224000 time 5 hashfull 0 pv 2f2e
```

深さ5でfail lowが3回続き、深さ6でfail highへ転じている。窓の再設定が
外から追えるようになった。

## Consequences

探索を変えない。`info` 行が増えるだけである。深さが進むほどaspirationの
failは減るので、行数の増加は序盤の浅い深さに集中する。

`SearchInfo` の消費側は `Bound` の扱いを書く必要がある。現在の消費側は
`thread.rs` とテストの2か所である。

`currline`（各スレッドの読み筋）を将来足すときも、同じ形で列挙へ
追加できる。
