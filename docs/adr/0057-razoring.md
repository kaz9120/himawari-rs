# 0057: razoringを導入する

- Status: accepted
- Date: 2026-07-24
- 関連ADR: [0028](0028-pruning-extensions.md), [0024](0024-search-v1.md)

## Context

探索改善キャンペーンの続行。静的評価がalphaを大きく下回る浅い
ノードでは、通常探索を省略してqsearchの結果だけで返す枝刈りが
有効である。reverse futility（ADR-0028）がbeta側で「良すぎる
なら刈る」のに対し、razoringはalpha側で「悪すぎるなら手を抜く」
補完的な位置づけである。

現在のsearch.rsには、depth==0でqsearchへ落ちる処理（464行）がある。
depth 1〜3の浅いノードで静的評価が絶望的なとき、早めにqsearchへ委ねる
仕組みはない。SF系ではrazoring（またはそれに
相当する浅い深さのqsearch降格）が標準装備されている。

## 選択肢と比較

### 案A: 直接qsearchに降格する（SF現代形）

`static_eval + margin <= alpha`のとき、qsearchの結果をそのまま
返す。二段階チェック（先にqsearchしてからalpha超えか判定）を
省いた簡潔な形。qsearchは取る手と王手を網羅するため、タクティ
カルな救済があればそこで拾える。

### 案B: 二段階チェック（古典形）

先にqsearchを呼び、結果がalphaを超えなければ返す。超えたら
通常探索を続行する。安全だが、qsearchを常に呼ぶ追加コストが
ある。結果的にqsearchを二重に呼ぶ（razoringで1回、depth==0で
もう1回）パスが生じうる。

## Decision

案Aを採用する。

### 実装スケッチ（search.rs）

定数をsearch.rs冒頭の定数群に追加する。

```rust
const RAZOR_MAX_DEPTH: u32 = 3;
const RAZOR_MARGIN: Value = 300;
```

配置はRFP（501-511行）の直後、NMP（513行〜）の前。

```rust
// razoring（ADR-0057）
if excluded == Move::NONE
    && !is_pv
    && !in_check
    && depth <= RAZOR_MAX_DEPTH
    && alpha.abs() < VALUE_MATE_IN_MAX_PLY
    && static_eval + RAZOR_MARGIN <= alpha
{
    return self.qsearch(alpha, beta, ply, 0);
}
```

条件パターンはRFP・ProbCut等の既存枝刈りに揃える（non-PV、
非王手、除外手なし、mate scoreガード）。マージンは固定300で
depth非依存とする。depth 1〜3の範囲では、静的評価がalphaから
300以上離れていればタクティカルな救済以外に逆転の見込みがなく、
qsearchがその救済を検出する。

初期定数（チューニングしない）: RAZOR_MAX_DEPTH=3、
RAZOR_MARGIN=300。SF系の実績値に基づく。

### 検証

SPRTはADR-0028の既定条件。両エンジンに
`--option "EvalFile=data/nets/halfkp_180M.hmwr.best"`。

## Consequences

- 浅い深さの絶望局面でqsearchへすぐ降格するため、無駄な
  通常探索ノードが減る。NPS改善ではなく探索木の縮小による効率化
- マージン300はRFP_MARGIN（120/depth）より大きく、futilityの
  合計マージン（200+120*depth）と同程度。razoringが発動する
  局面はfutilityより「さらに悪い」状況に限られる
- RFP（beta側）とrazoring（alpha側）が同時に成立することは
  ない（alpha < betaなので、両方の条件を同時に満たす
  static_evalは存在しない）
- qsearch呼び出しのコスト自体は小さいが、razoringが頻発すると
  探索の深さが実質浅くなる。マージンが小さすぎるとタクティカルな
  手を見落とすリスクがある。300はその安全側に寄せた値
