# 0055: LMRに条件項（improving・history連動）を導入する

- Status: rejected
- Date: 2026-07-22

2026-07-23追記: SPRT 960局（480ペア）でElo -9.0 [-30.7,+12.5]、
LLR -0.48。効果なし〜ややマイナスと判断し打ち切り・不採択。
0054でqsearch TTが入った直後のベースは履歴の質が変わって
いる可能性があり、improving項単体・history項単体の分解や
しきい値の再設計は再挑戦の余地としてIDEAS.mdに残す。
- 関連ADR: [0028](0028-pruning-extensions.md), [0046](0046-correction-history.md), [0047](0047-continuation-history.md)

## Context

探索改善キャンペーンの第9弾。現在のLMRはlog式テーブルに
PVで-1する調整しかない（`search.rs:770-780`）。手の筋の良し悪し
（history）や局面の改善傾向（improving）を無視して一律に
削っており、SF系ではここに条件項を足すのがオーダリング系で
最大級の利得源になっている。ADR-0046/0047で導入した履歴群が
そのまま材料になる。

log式の係数（0.5、2.25）は触らない。式の再チューニングでは
なく、固定係数の条件項を追加する。

## 選択肢と比較

### 案A: improving項 + history項の2項

- improvingでないとき r += 1（悪化局面では深追いしない）
- 手のhistoryスコア（main + continuation 2本の合算）に応じて
  r を±1する。良い筋は深く、悪い筋は浅く

配管が既存の値（improvingフラグ、オーダリングと同じ履歴合算）
だけで完結し、判定単位が明確。

### 案B: 案A + cutnode項（SF完全形に近い形）

cutnode（期待カットノード）で r += 2 はSFで効果が大きいが、
ノード種別フラグを探索の再帰全体に配管する変更が要る。
案Aの成否を見てから別ADRで積む。

## Decision

案Aを採用する。

### 実装スケッチ（search.rs、LMRブロックのみ）

```rust
let mut r = lmr_reduction(depth, count);
if is_pv {
    r = r.saturating_sub(1);
}
// improving項（ADR-0055）: 静的評価が悪化中は1深く削る
if !improving {
    r += 1;
}
// history項（ADR-0055）: オーダリングと同じ履歴合算で±1
let hist = self.history.get(m)
    + self.cont.get(prev1, m)
    + self.cont.get(prev2, m);
if hist > LMR_HIST_GOOD {
    r = r.saturating_sub(1);
} else if hist < LMR_HIST_BAD {
    r += 1;
}
```

初期定数（チューニングしない）: `LMR_HIST_GOOD = 4000`、
`LMR_HIST_BAD = -4000`（履歴1本のクランプ値。3本合算の
値域±12000に対し「1本分はっきり良い/悪い」を意味する）。
d下限の`max(1)`は既存のまま。

### 検証

SPRTはADR-0028の既定条件。両エンジンに
`--option "EvalFile=data/halfkp_180M.hmwr.best"`。

## Consequences

- 削り幅の分散が広がる（最大でlog式+2）。fail-highの再探索
  （既存のd < new_depth再読）が保険として機能する
- prev1/prev2はスコアリング時と同じ値をLMR判定でも読むため、
  追加コストはhistoryテーブル参照3回のみ
- H1採択後、案B（cutnode項）とLMRのcapture適用拡大を
  別ADRで検討する。見直しトリガーはネット再学習で履歴の
  スケール感が変わったとき
