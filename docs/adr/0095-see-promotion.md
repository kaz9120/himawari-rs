# 0095: SEEで初手の成りを扱う（ほぼ等価と判明）

- Status: accepted
- Date: 2026-07-29
- 関連ADR: [0091](0091-see-drop.md), [0025](0025-move-ordering.md), [0074](0074-feature-verification.md)

## Context

[ADR-0091](0091-see-drop.md)で駒打ちのSEEを解き、+67.0 Eloを得た。
既存実装に残っていた近似を正しただけで大きな差が出た。`see_ge` には
もう1つ近似が残っている。

```rust
/// mの静的交換評価がthreshold以上か。成りは考慮しない簡略版。
```

移動元の駒の価値をそのまま使っており、成りによる価値の上昇を見ていない。
将棋の成りは大きい。歩(90)→と金(540)で+450、飛(990)→竜(1395)で+405で
ある。同じ路線で効くと見込んで着手した。

## Decision

`see_ge` の初期条件を、成りを含む形に変える。

```rust
let before = PIECE_VALUE[self.piece_on(from).piece_type().index()];
let after = PIECE_VALUE[m.piece_after().piece_type().index()];
(
    PIECE_VALUE[self.piece_on(to).piece_type().index()] + after - before,
    after,
    self.occupied() ^ Bitboard::from_square(from),
)
```

成りの利得を取り分（`captured`）へ足し、取り返される駒（`placed`）を
成ったあとの価値にする。

## なぜほぼ等価なのか（2026-07-29）

**機能検証で4局面ともノード数が完全に一致した。** 理由は式にある。

`gain = after - before` とおく。swapの2段目は次のようになる。

| | 1段目 `swap1` | 2段目 `swap2 = placed - swap1` |
|---|---|---|
| 変更前 | `captured - threshold` | `before - captured + threshold` |
| 変更後 | `captured + gain - threshold` | `(before + gain) - (captured + gain - threshold)` = `before - captured + threshold` |

**2段目は完全に一致する。** 取り分と失う駒の両方へ同じ `gain` を足すため、
差し引きで消える。以降のswapループは2段目の値から始まるので、結果も
同じになる。

差が出るのは1段目の `if swap1 < 0 { return false }` だけである。

- 変更前: `captured < threshold` で偽
- 変更後: `captured + gain < threshold` で偽

つまり**閾値が正で、かつ取る駒の価値を超えるときにだけ**差が出る。

本エンジンが `see_ge` を正の閾値で呼ぶのは静止探索のfutility判定
（`see_ge(m, alpha - futility_base)`）の1か所だけである。他は0か負で
ある（[ADR-0090](0090-see-pruning.md)の枝刈りは負、オーダリングと
ProbCutは0）。固定深さ13の4局面では、そこでも差が出なかった。

### 元の実装は「簡略版」ではなかった

コメントの「成りは考慮しない簡略版」は、読むと精度が落ちているように
見える。実際には、閾値が0以下である限り結果は同じである。近似ではなく
**表現の違い**だった。

[ADR-0091](0091-see-drop.md)の駒打ちは違う。あちらは `return 0 >= threshold`
で交換そのものを解いていなかったため、結果が変わった。同じ「近似が残って
いる」でも、結果に効くものと効かないものがある。

## Consequences

探索は変わらない。[ADR-0074](0074-feature-verification.md)の基準では
SPRTにかけても中立にしかならないため、対局は行わない。

それでも入れるのは、SEEの意味論が式のうえで正しくなるためである。
将来 `see_ge` を正の閾値で使う場面（capture historyのスケール設計など）
が来たとき、成りを含む取り分で判定できる。コメントの誤解も解ける。

得られた知見は「近似を正しても、式のうえで相殺されるなら結果は変わらない」
ことである。[ADR-0091](0091-see-drop.md)が+67.0だったのを見て同じ路線を
期待したが、差が出るかどうかは式を追えば着手前に分かった。次に近似を
正すときは、まず式で差の出る条件を確かめる。
