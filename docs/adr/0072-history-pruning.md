# 0072: history pruning（履歴が悪い静かな手の枝刈り）

- Status: proposed（保留。前提となるhistoryスケールの再設計待ち）
- Date: 2026-07-27
- 関連ADR: [0028](0028-pruning-extensions.md), [0047](0047-continuation-history.md), [0025](0025-move-ordering.md), [0073](0073-history-bonus-scale.md)

## Context

2026-07-27にやねうら王masterと探索部の機能差分を棚卸しした（[IDEAS.md](../IDEAS.md)）。
ムーブループ内の枝刈りは、やねうら王が5種類を持つのに対し本エンジンは
2種類（LMP・quiet futility）しかない。この差が最大の構造的欠落だった。

枝刈りの不足が棋力を抑えている傍証がある。razoringは+184.8 Elo
（[ADR-0057](0057-razoring.md)）、qsearchのTT保存は+113.6 Elo
（[ADR-0054](0054-qsearch-tt.md)）で、いずれも「読まなくてよい局面を
読まずに済ませる」変更だった。同じ土壌が残っている。

欠落した3種類のうち、capture futilityとcapture SEE pruningはcapture history
（[ADR-0048](0048-capture-history.md)でrejected）の作り替えを前提とする。
history pruningだけは前提を持たず、単独で実装・検証できる。ここから着手する。

決めるのは、判定に使う履歴の組み合わせと、閾値のスケール換算の方法である。

## 選択肢と比較

判定材料をどう組むかが論点になる。閾値そのものはSPRTで直せるが、
どの履歴を足すかは後から変えると別機能になる。

### 案A: continuation history 2項の合計

1手前・2手前のcontinuation historyを足し、`-M * depth` を下回る静かな手を捨てる。
やねうら王と同型で、あちらは `cont[0] + cont[1] + pawnHistory` の3項を使う。
本エンジンはpawn historyを持たないため2項になる。

利点は、continuation historyが手の文脈（どの手に対する応手か）を持つため、
「この流れではこの手は悪い」という判断がmain historyより鋭いこと。
[ADR-0047](0047-continuation-history.md)で+20.7 Eloを得た実績もある。

欠点は、2項しかないぶん値の分散が小さく、閾値の設計がやねうら王より
シビアになること。

### 案B: main historyを加えた3項合計

案Aに `History::get(m)` を足す。項が増えて分散が広がり、閾値を取りやすい。

欠点は、main historyが`[移動後の駒][移動先]`しか持たず、fromも駒打ちの
区別も表現しないこと（[IDEAS.md](../IDEAS.md)の「main historyの次元拡張」）。
衝突した局面の値が混ざるため、枝刈りの判断材料としては信頼度が落ちる。
やねうら王もmain historyはこの判定に入れず、後段の`lmrDepth`補正でのみ使う。

### 案C: continuation history 1項（1手前のみ）

最も単純。ただし2手前の文脈を捨てる理由がない。案Aより弱い案で、
実装コストの差もない。

## Decision

案Aを採る。判定は次の形にする。

```
!is_pv かつ 非王手 かつ 静かな手 かつ 王手をかけない手 かつ best > 詰まされ圏
かつ cont.get(prev1, m) + cont.get(prev2, m) < HISTORY_PRUNING_MARGIN * depth
```

`HISTORY_PRUNING_MARGIN = -2000` を出発点にする。

この値はやねうら王の`-4097`をスケール換算して決めた。あちらの判定材料は
continuation history 2本（上限30000）とpawn history 1本（上限8192）の合計で、
理論上の値域は±68192になる。本エンジンはcontinuation history 2本のみで、
gravity方式のdivisorが16384のため値域は±32768である。比は0.4805で、
`-4097 × 0.4805 ≒ -1969` から丸めた。

[ADR-0028](0028-pruning-extensions.md)のパラメータ方針は「他エンジンの実績値を
出発点にする」だが、スケールが違う量をそのまま移植すると意味が変わる。
換算を挟むのはこの方針の範囲内と判断する。

深さの上限は設けない。閾値は深いほど負に大きくなり、そこを下回る手は稀に
なるため、深いノードでは自然に枝刈りが効かなくなる。上限を別途置く理由がない。

PVノードを除外するのは既存のLMPに合わせた。やねうら王は`followPV`
（前回反復のPV上にいるか）で除外範囲をさらに絞るが、本エンジンはこの情報を
持たない。導入するなら別ADRで扱う。

検証は[ADR-0028](0028-pruning-extensions.md)の既定条件でSPRTを行う。
elo0=0、elo1=5、α=β=0.05。H1採択でmainへ取り込む。

## 実装後の測定と保留（2026-07-27）

上の設計で実装しSPRTを回したが、1039ペア（2078局）で
Elo +0.8 [-13.7,+15.4]、LLR -0.15 と中立に張り付き、判定に至らず打ち切った。

原因を診断したところ、この枝刈りは**一度も発動していなかった**。
固定深さ13で3局面を探索し、導入前後のノード数を比較した結果である。

| 局面 | 導入前 | 導入後 |
|---|---|---|
| 1 | 425,834 | 425,834 |
| 2 | 5,187,409 | 5,187,409 |
| 3 | 201,904 | 201,904 |

完全一致であり、SPRTは実質同一のバイナリ同士を戦わせていた。
Elo +0.8 は機能の評価ではなくノイズである。

閾値をスイープすると、発動し始める点が分かった。

| `HISTORY_PRUNING_MARGIN` | 局面1のノード数 |
|---|---|
| -100 | 527,390 |
| -500 | 422,631 |
| -1000 | 425,834（発動せず） |
| -2000（実装値） | 425,834（発動せず） |

continuation historyの値が `-1000 × depth` に到達しない。理論上の値域は
±16384だが、実測では数百までしか振れていない。bonusが
`depth² + 2·depth`（depth 5で35）と小さく、gravityの平衡点に達するには
同一エントリへ何百回も更新が要る。テーブルは6.7Mエントリあり、
そこまで更新が集中しない。

やねうら王のbonusは `min(128·depth - 77, 1529)`（depth 5で563）、
malusは別式の `min(882·depth - 204, 2122)` で、depth 5でbonusが16倍、
malusが60倍ある。閾値 `-4097 × depth` はこのスケールを前提にしている。
Decisionで行ったスケール換算は値域の理論値を使ったが、実効的な分布を
見ておらず、そこが誤りだった。

したがって本ADRの「単独で実装・検証できる」という前提は成り立たない。
[ADR-0073](0073-history-bonus-scale.md)でbonus/malus式を再設計した後に
再挑戦する。閾値は再設計後の実効分布を測って決め直す。

## Consequences

枝刈りが1種類増え、静かな手の探索本数が減る。NPSではなく実効深さが伸びる形の
改善を狙う。読み抜けが増える方向のリスクは、`best > 詰まされ圏` のガードと
PVノード除外で抑える。

閾値がcontinuation historyのスケールに依存するため、
[IDEAS.md](../IDEAS.md)の「bonus/malus式の再設計」や「continuation historyの
段数拡張」を実施すると、この閾値の再調整が必要になる。両者を変えるときは
`HISTORY_PRUNING_MARGIN` の再測定を同じPRに含める。

H0になった場合、閾値を変えた再挑戦は[ADR-0028](0028-pruning-extensions.md)の
規約どおり可能である。ただし2回続けて中立なら、原因は閾値ではなく
判定材料の薄さ（continuation history 2項のみ）にあると考え、pawn historyの
導入を先に済ませてから戻る。
