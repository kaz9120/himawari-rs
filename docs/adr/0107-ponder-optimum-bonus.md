# 0107: USI_Ponderが有効なとき思考時間を1.25倍する

- Status: proposed
- Date: 2026-07-29
- 関連ADR: [0104](0104-ponderhit-time-accounting.md), [0106](0106-ponderhit-continue.md), [0021](0021-time-management.md), [0033](0033-ponder.md), [0102](0102-move-horizon.md)

## Context

Stockfishは `USI_Ponder` に相当するオプションが有効なとき、思考時間を
25%増やす（`src/timeman.cpp`）。

```cpp
if (options["Ponder"])
    optimumTime += optimumTime / 4;
```

himawariにはこの補正がない。ponderの有無で1手の配分は変わらない。

この係数は[ADR-0104](0104-ponderhit-time-accounting.md)で一度扱った。
ただしそこでは「ponderで読んだ時間を予算に数える」会計の変更と**組で**
測っている。会計を厳しくする側と補正する側は対だ、という理由からだった。

その会計は-117.8で棄却され、[ADR-0106](0106-ponderhit-continue.md)で
前提（探索の継続）を足しても差し引きがほぼゼロだった。**会計を入れない
以上、1.25倍が埋め合わせるべき相手はもういない。** 単独の変更として
測り直せる。

オーナーが2026-07-29に「floodgateでponderを有効にしている。基本は常に
有効にするパラメータ」と示した。実戦では全手に効く。

### 軸と仮説

[ADR-0059](0059-easy-move-scaling.md)と同じ時間配分の軸。

**単独で効く仮説**: ponderが当たると、その手の思考の一部は相手の時計で
進む。つまり自分の時計1秒あたりに得られる探索量が、ponderなしより多い。
にもかかわらず配分はponderなしと同じである。ponderの取り分だけ、
自分の時計を厚く使ってよい。外れたときは時間を消費しないので、
期待値としては1.25倍が釣り合う。

これは[ADR-0102](0102-move-horizon.md)が失敗した「一律に薄くする」の
逆向きだが、性質が違う。ADR-0102は全局面から時間を奪ったのに対し、
本ADRは全局面へ配る。前者は「読めなくなる局面」を作るが、後者は作らない。
上限（maximum）は触らないので、荒れた局面での上振れも従来どおりである。

## 選択肢と比較

### 案A: 何もしない

ponderの有無で配分を変えない。現状。ponderで得た無料の探索を、自分の
時計の使い方に反映しない。

### 案B: Ponder有効時にoptimumを1.25倍する

Stockfishと同じ。`maximum` は触らない。

### 案C: ponderの的中率を測って動的に補正する

的中率が高いほど厚く配る。状態を対局間で持つ必要があり、定数も増える。
まず案Bで効くかを確かめてからでよい。

## Decision

案Bを採用する。

追加する定数は `PONDER_OPTIMUM_DIV = 4` の1個で、値は出典のまま使う。
**この係数は比率なので、持ち時間のスケールに依存しない。**
[ADR-0102](0102-move-horizon.md)で `MinimumThinkingTime`（絶対時間の
2000ms）を退けた理由は当てはまらない。10+0.1でも300+10でも「25%増やす」
の意味は同じである。

`maximum` は触らない。上限は時間切れ防止の役割なので、ponderの有無で
動かす理由がない。[ADR-0059](0059-easy-move-scaling.md)のscale上限
2.17との積が3.0を超える場合は、従来どおり `over_total` が
`min(optimum × scale, maximum)` で押さえる。

`USI_Ponder=false` では一切影響しない。

## 検証

機能検証（[ADR-0074](0074-feature-verification.md)）は、`go depth N` の
ノード数が変わらないことの確認になる。時間管理を通らない経路である。

SPRTは両者ponder・並列4（`selfplay --ponder-both`）。ponderが有効な
対局でしか差が出ない。他は[ADR-0028](0028-pruning-extensions.md)の
既定条件。

既定条件（10+0.1）とfloodgate（300+10）は、相手の思考時間とこちらの
optimumの比がどちらも1前後で揃っている。ponderの取り分の割合という点で
相似なので、既定条件での測定が実戦へ転移すると期待できる。

## Consequences

- ponder有効時、1手の配分が25%増える。持ち時間の消費が早くなる
- 悪い方向に出るとすれば、終盤の持ち時間が減ることによる。
  [ADR-0102](0102-move-horizon.md)の実測では、現行の配分は自分の30手目で
  持ち時間の70%を使う。そこからさらに厚くすると終盤が薄くなる
- 上限（maximum）は変えないので、1手の最大消費は増えない
- 効かなかった場合、ponderまわりの時間管理は4通り試して全滅になる
  （[ADR-0104](0104-ponderhit-time-accounting.md)・
  [ADR-0106](0106-ponderhit-continue.md)と本ADR、および両者の組み合わせ）。
  そのときは案C（的中率で動的に）へ進まず、時間管理から離れる
