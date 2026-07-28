# 0094: mate_1plyの検証を軽くする

- Status: proposed
- Date: 2026-07-29
- 関連ADR: [0029](0029-mate-search.md), [0093](0093-mate1ply-in-search.md), [0089](0089-improvement-criteria.md)

## Context

[ADR-0093](0093-mate1ply-in-search.md)で `mate_1ply` を探索へ組み込んだ
ところ、SPRTで -57.5 Elo（494局で打ち切り）だった。発動率は終盤で
13.9%まで上がるのに、呼び出しコストが効果を上回った。

コストの内訳を計測した。`is_mate_move` は候補手ごとに次を行う。

1. `pseudo_legal` と `is_legal` の検査
2. `do_move`
3. `generate_legal` で全合法手を生成し、空かどうかを見る
4. `undo_move`

問題が2つある。`do_move` は王手にならない手に対しても実行される。
`generate_legal` は回避手を全部集めてから空判定するが、1つ見つかれば
そこで結論が出る。

## Decision

`is_mate_move` を次のとおり変える。

```rust
if !pos.gives_check(m) {
    return false;      // 王手にならない手はdo_moveの前に弾く
}
pos.do_move(m);
let mut pseudo = MoveList::default();
generate(pos, GenType::Evasions, true, &mut pseudo);
let escapable = pseudo.as_slice().iter().any(|&mv| pos.is_legal(mv));
pos.undo_move(m);
!escapable
```

`gives_check` は開き王手も拾うため、元の実装（指してから `in_check` を
見る）と同じ手が残る。`.any()` は回避手を1つ見つけた時点で `is_legal`
の残りを省く。

[ADR-0089](0089-improvement-criteria.md)の軸1（探索の出力が変わらない
高速化）に当たる。返す手は変わらない。

## 計測（2026-07-29）

ランダムプレイアウトで集めた663局面（40手目以降）へ `mate_1ply` を
20回ずつ呼んだ。

| | 1回あたり | 詰み検出 |
|---|---|---|
| 変更前 | 1.52us | 220件 |
| 変更後 | 1.28us | 220件 |

**-16%**。詰みの検出数は変わらない。

候補手の内訳は次のとおり（663局面あたり）。

| 段階 | 回数 |
|---|---|
| 候補手 | 1,241 |
| 擬似合法かつ合法 | 1,213 |
| `gives_check` 通過（`do_move` 実行） | 906 |

`gives_check` で25%の `do_move` を省けた。

`mate_1ply_oracle` との照合テスト（`crates/engine/tests/mate_tests.rs`）が
通ることで、返す手が変わらないことを担保している。

## Consequences

`mate_1ply` が16%速くなる。ただし探索から呼ばれていないため、この変更
単独では棋力に影響しない。`tsume` ツールと将来の再挑戦のための土台である。

**16%では[ADR-0093](0093-mate1ply-in-search.md)の再挑戦には足りない。**
探索1ノードが約0.83us（NPS 1.2M）に対し、`mate_1ply` は1.28usかかる。
TTミスの約70%のノードで呼ぶ設計では、探索コストが倍増する計算になる。

残るボトルネックは `do_move` と候補生成である。やねうら王の `mate_1ply`
は駒を動かさず利きの計算だけで判定する。同じ方式へ書き換えれば桁で速く
なる見込みがあるが、[ADR-0029](0029-mate-search.md)の「指して検証する
ので誤検出がない」という設計を捨てることになる。再挑戦するならそこから
始める。
