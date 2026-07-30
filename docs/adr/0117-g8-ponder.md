# 0117: ponderの会計・継続・予約を参照実装へ揃える（G8）

- Status: proposed
- Date: 2026-07-31
- 関連ADR: [0109](0109-reference-parity.md), [0116](0116-g7-timeman.md), [0104](0104-ponderhit-time-accounting.md), [0106](0106-ponderhit-continue.md), [0107](0107-ponder-optimum-bonus.md), [0033](0033-ponder.md)

## Context

[ADR-0109](0109-reference-parity.md)の追従キャンペーンの第9群。ponderまわりを
参照実装（やねうら王）へ揃える。

**この領域は2026-07-29に3件を棄却している。**

| ADR | 移したもの | 結果 |
|---|---|---|
| [0104](0104-ponderhit-time-accounting.md) | ponder中の時間を予算に数える会計 | -117.8 |
| [0106](0106-ponderhit-continue.md) | ponderhitで探索を継続する | -54.3 |
| [0107](0107-ponder-optimum-bonus.md) | optimumの1.25倍 | 棄却 |

いずれも参照実装の4本の支えのうち1本ずつを移したものだった。4本目が停止の予約で、
[ADR-0116](0116-g7-timeman.md)（G7）で入った。**この群で4本を揃える。**

噛み合い方はこうである。ponderで読んだ時間を予算に数えるので、ponderhit時点で
予算をほぼ使い切っていることがある。それでも困らないのは次が同時に効くからである。

1. 探索が止まっていないので既に深い
2. `stopOnPonderhit` が予約に留まり即停止しない
3. `set_search_end()` の `t2` 項が `minimum()` を保証する
4. `optimum` が1.25倍されているので予算そのものが厚い

**1本でも欠けると成立しない。** ADR-0104は2〜4を、ADR-0106は1だけを、ADR-0107は
4だけを移していた。

## Decision

4本をまとめて入れる。現行のponder（[ADR-0033](0033-ponder.md)）は、ponderhitで
探索を無音キャンセルし実時間で再起動する設計だった。**これを捨てて継続する形へ
変える。**

### 依存の棚卸し

実装の前に洗い出した。指示書が挙げた3点がすべて不足していた。

| 不足 | 実際の状態 | 対応 |
|---|---|---|
| `startTime`（go受領時刻） | なし。`TimeManager::new` 内で取っていた | `Limits::start` を追加し `parse_go` の先頭で記録 |
| `ponderhitTime` の更新 | なし。G7で `ponderhit_offset` を0固定 | `Shared::ponderhit_offset`（AtomicI64） |
| ponder中に止めない構造 | `Limits { infinite: true }` で時間管理を無効化 | 実limitsでtmを作り `Shared::ponder` でガード |

書き手（USIスレッド）と読み手（探索スレッド）が別なので、参照実装が
TimeManagementのメンバーに置く2値は原子変数にした。

### 入れたもの

| 項目 | 原典 |
|---|---|
| `Limits::start`（go受領時刻） | usi.cpp:506、T:76・148 |
| `Shared::ponder` の初期化 | S:114-120 |
| `Shared::ponderhit_offset` | timeman.h:120 |
| メインはponderでも実limitsでtm作成 | S:960-975 |
| ponder中は止めない | S:5502-5507 |
| ponderhitで探索を継続 | S:299-308 |
| `stop_on_ponderhit` の予約 | S:2043-2044、S:1965 |
| fail lowでの解除 | S:1783-1784 |
| check_timeの条件 | S:5551-5558 |
| `optimum` の1.25倍 | T:285-286 |

### ponderhitの順序を構造で保証した

参照実装の `set_ponderhit()` は `ponderhitTime = now()` を先に、`ponder = false` を
後に実行する。**順序が逆だと他スレッドが古い `ponderhitTime` で計算する**と
原典のコメントが明記している（S:302-304）。

本エンジンは `ponderhit_offset` を `SeqCst` で書いたあとに `ponder` を `SeqCst` で
下ろす。`ponder == false` を観測したスレッドは必ず新しいoffsetも観測する。
逆順を構造で禁じている。

### 2手指し防御は残した

`PonderState::FinishedHolding` の待機を残した。go ponder中に探索が自然終了すると
条件変数で待ち、ponderhit/stopで解放する。参照実装の待機ループ（S:1162-1187）と
同じ形である。`Hit` の意味だけを「無音キャンセル」から「探索は続け、終わったら
出す」へ変えた。

## 測定

USI経由でG7完了時点と比較した。局面は `startpos moves 7g7f 3c3d 2g2f`、
`go btime 10000 wtime 10000 binc 100 winc 100`。

### 探索の継続

infoのdepthが巻き戻らないかを見た。

| 条件 | G7 | G8 |
|---|---|---|
| ponder 500ms→hit | 単調=False、最終depth 27 | **単調=True**、depth 26 |
| ponder 3000ms→hit | 単調=False、depth 29 | **単調=True**、depth 31 |

再起動をやめたので木を捨てていない。

### 会計

床を外した条件（`MinimumThinkingTime=1`・`RoundUpToFullSecond=false`）。

| 条件 | G7 総時間 / hit後 | G8 総時間 / hit後 |
|---|---|---|
| goのみ | 215ms | 212ms |
| ponder 500ms→hit | 605ms / 104ms | **521ms / 14ms** |
| ponder 3000ms→hit | 3123ms / 116ms | **3010ms / 2ms** |

goからの経過が予算に入っている。3000msポンダー後の最終depthは両者27で同じ
なので、**深さを落とさずに自分の時計を100ms余らせている。**

### stop_on_ponderhit

一時的な計測で追った。床なし・ponder 400msの例である。

```
stop_on_ponderhit=true  at 221ms depth=17
stop_on_ponderhit=false (fail low) at 259ms depth=18
stop_on_ponderhit=true  at 350ms depth=18
stop_on_ponderhit=false (fail low) at 359ms depth=19
stop_on_ponderhit=true  at 398ms depth=19
set_search_end e=402 off=402 by_sop=true max=832   -> search_end=283
```

ponder中に予算超過で立ち、fail lowで解除され、ponderhit直後に予約へ変わる。
この例では `elapsed(402) < maximum(832)` なので、**`stop_on_ponderhit` がなければ
予約は起きなかった。** 支えとして効いている。

床ありでは `search_end = 1880 + 3008 = 4888` となり、`t2` 項がponderhitから
1880msを保証していた。

### 回帰

`go depth 12` のノード数は2局面ともG7と完全一致（13864 / 23368）。ponderを
使わない探索は変わっていない。

## SPRT

（結果は判定後に追記する）

条件の選定に難がある。**床を外すと支えの1本が消える。**

既定の `MinimumThinkingTime=2000` は10+0.1で成立しない（[ADR-0116](0116-g7-timeman.md)）。
base同士でも4局すべてが時間切れで終わった。一方 `MinimumThinkingTime=1` にすると
`minimum()` が0になり、**`set_search_end` の `t2` 項が働かなくなる。** 実測で
ponderhit後2msで止まっていた。4本のうち3本だけを測ることになる。

そこで `MinimumThinkingTime=300`・`RoundUpToFullSecond=false` で測る。`minimum()` が
180msになり、ponderhitから180msの思考が保証される。4本すべてが働き、かつ時間切れが
出ないことを8局で確認した。

`--ponder-both` は必須である。これを付けたときだけ両者に `USI_Ponder=true` が渡り、
1.25倍とponder経路が有効になる。1局が2コアを使うので並列度は半分にする。

判定は既定の elo0=0 / elo1=5 で行う。この群は棄却済み3件を覆す仮説なので、
非劣性へ落とすのは棄却が出てからにする。

## Consequences

（判定後に書く）
