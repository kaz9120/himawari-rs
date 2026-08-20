# 0086: 探索の可観測性を上げる（seldepth・currmove）

- Status: accepted
- Date: 2026-07-28
- 関連ADR: [0032](0032-multipv.md), [0024](0024-search-v1.md)

## Context

USIの `info` で出している項目は `depth` `score` `nodes` `nps` `time`
`hashfull` `pv` と、MultiPV時の `multipv` である。標準的な項目のうち
次の2つが欠けている。

- `seldepth`: 静止探索を含めて到達した最大ply
- `currmove`: rootで今読んでいる手

欠けている影響は2つある。1つは、長考中に何をしているか外から見えない
ことである。floodgateの対局を眺めていても、探索が進んでいるのか
止まっているのか区別できない。

もう1つは、探索の性質が測れないことである。`depth` と `seldepth` の差は
静止探索がどれだけ伸びているかを示す。枝刈りを足したとき、この差が
どう動いたかは設計の手がかりになる。今は測る手段がない。

`hashfull` は実装済みで、TTの埋まり具合は見えている。可観測性の欠落は
探索の深さ方向に集中している。

## 選択肢と比較

### 案A: 報告のコールバックを種別付きにする

`iterate` が呼ぶコールバックの引数を `IterInfo` から `SearchInfo` へ
変える。`SearchInfo` は列挙で、反復深化1周分の `Iteration` と、
rootで今読んでいる手の `CurrMove` を持つ。

```rust
pub enum SearchInfo {
    Iteration(IterInfo),
    CurrMove { depth: u32, mv: Move, number: usize },
}
```

種別が増えても列挙に足すだけで済む。`currline`（各スレッドの読み筋）や
`lowerbound` / `upperbound`（aspirationのfail時報告）を後から足すときも
同じ形に乗る。

呼び出し側は `match` が要る。現状の呼び出し側は `thread.rs` と
テストの2か所で、影響は小さい。

### 案B: コールバックを2本に増やす

`iterate(on_iter, on_currmove)` とする。

種別が増えるたびに引数が増える。`iterate` の呼び出し箇所すべてを
書き換えることになる。案Aなら列挙に追加するだけで、既存の `match` は
網羅性検査で漏れが分かる。

### 案C: seldepthだけ足す

`IterInfo` にフィールドを1つ増やせば済む。`currmove` は見送る。

`currmove` は長考中の唯一の生存信号になる。反復深化1周に数十秒かかる
深さでは、`Iteration` の報告が来ない時間は長い。ここを埋めたい。

## Decision

案Aを採る。

`seldepth` は `search` と `qsearch` の入口で `ply` の最大を記録する。
イテレーションごとに0へ戻し、その周の到達深さを表す。出力時は `depth`
との大きいほうを採る（rootだけで結論が出たとき `seldepth < depth` と
なるのを防ぐ）。

`currmove` は探索開始から3秒経ってから出す。短い探索で出すと `info` 行が
溢れる。UCIの慣例に合わせた閾値である（`CURRMOVE_MIN_MS = 3000`）。

### currmovenumberを出さない理由

UCIには `currmovenumber`（今何手目を読んでいるか）があり、やねうら王も
`source/usi.cpp` で `currmove` と並べて出力している。本エンジンは出さない。

USIの規定に `currmovenumber` は見当たらない。UCI由来の項目である。
GUIが未知の項目を無視する保証はなく、規定外の項目を出す利点は薄い。
`currmove` だけで「今どの手を読んでいるか」は伝わり、長考中の生存信号と
いう目的は満たせる。

必要になれば足せる。`SearchInfo::CurrMove` に番号を持たせるだけで済む。

ヘルパースレッドは出力しない。出力経路を持つのはメインワーカーだけで、
この変更でもその設計は変えていない。

## Consequences

`iterate` のコールバックが列挙を受け取る形になる。呼び出し側に `match`
が増えるが、報告の種類を足すときに引数を増やさずに済む。

`seldepth` の記録は `search` / `qsearch` の入口に1命令ずつ増える。
探索の挙動は変えない。ノード数も変わらない。

`depth` と `seldepth` の差が見えるようになる。静止探索の伸びを測る指標
として、今後の枝刈り設計で使う。

この変更は棋力に影響しない。探索の判断に `sel_depth` を読む箇所はない。
