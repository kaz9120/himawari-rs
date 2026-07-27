# 0073: history bonus/malus式の再設計

- Status: proposed
- Date: 2026-07-27
- 関連ADR: [0025](0025-move-ordering.md), [0047](0047-continuation-history.md), [0072](0072-history-pruning.md)

## Context

[ADR-0072](0072-history-pruning.md)でhistory pruningを実装したが、
一度も発動しなかった。診断の結果、原因はhistoryの値が想定より
1〜2桁小さいことだった。

現在のbonusは `depth² + 2·depth` で、bonusとmalusに同じ式を使う。
gravity方式の更新は `h += bonus - h·|bonus|/16384` なので、平衡点は
±16384である。だがそこへ近づく速度はbonusの大きさで決まる。

| depth | himawari bonus/malus | やねうら王 bonus | やねうら王 malus |
|---|---|---|---|
| 1 | 3 | 51 | 678 |
| 3 | 15 | 307 | 2122 |
| 5 | 35 | 563 | 2122 |
| 10 | 120 | 1203 | 2122 |

depth 5でbonusが16倍、malusが60倍の開きがある。

問題は値域ではない。実測では両テーブルとも最大絶対値が14000超まで届き、
平衡点16384の近くに達する。問題は分布で、continuation historyは
6.7Mエントリのうち0.24%しか埋まらず、探索中に判定材料
`cont[0]+cont[1]` を引いた413万回のうち52.4%が「ちょうど0」だった
（[ADR-0072](0072-history-pruning.md)の測定）。

gravityは「同一エントリへ何度も更新が来る」ことを前提に平衡点へ近づく。
この疎さではその前提が成り立たない。1回の更新量が35では値が立たず、
大多数のエントリが0付近に留まる。やねうら王のmalus 2122は、
一度受けただけで枝刈りの閾値圏へ入る大きさである。同じ次元のテーブルでも
機能するのはこの差による。

影響はhistory pruningだけではない。IDEAS.mdに並ぶLMRのstatScore項、
capture系の枝刈りマージン（`131·captHist/1024` など）も、他エンジンの
係数はあちらのスケールを前提に決まっている。historyを判断材料として
使う候補すべてが、この式に依存する。

先に土台を直す。

## 選択肢と比較

### 案A: bonusとmalusを別式にし、やねうら王の係数へ揃える

`bonus = min(128·depth - 77, 1529)`、`malus = min(882·depth - 204, 2122)` とする。
実効的な値域が数千まで広がり、他エンジンの係数をそのまま参照できるようになる。

malusがbonusの1.4〜3.8倍になる。「良かった手を覚える」より
「外れた手を忘れる」を強くする設計で、Stockfish系が長く使ってきた形である。

欠点は、move orderingの挙動が全面的に変わること。
[ADR-0047](0047-continuation-history.md)で+20.7 Eloを得た構成の土台を
入れ替えるため、棋力が下がる可能性がある。

### 案B: gravityのdivisorを下げる

`16384` を小さくすれば平衡点が下がり、少ない更新回数で飽和する。
1行の変更で済む。

だが向きが逆である。問題は「値が小さすぎる」ことなので、
平衡点を下げるとさらに狭くなる。値域を広げたいなら divisor は上げる側で、
そうすると到達はいっそう遅くなる。この案は問題を解かない。

### 案C: bonusの係数だけ上げ、malusは対称のまま

`depth² + 2·depth` を `128·depth - 77` に替え、malusは引き続き `-bonus` とする。
案Aより変更が小さく、非対称化のリスクを負わない。

ただし他エンジンが一様に非対称を採っている理由を捨てることになる。
探索では「悪い手を早く後ろへ落とす」ほうが効き、malusを強くする根拠がある。
案Aとの差は1つの係数なので、まず案Aを測り、H0なら案Cへ後退するほうが
情報が多く得られる。

## Decision

案Aを採る。`update_quiet_stats` のbonusとmalusを次のとおり分ける。

```
bonus = clamp(128·depth - 77, 0, 1529)
malus = clamp(882·depth - 204, 0, 2122)
```

最善手にbonusを、探索して外れた静かな手に `-malus` を与える。
main historyとcontinuation history（1手前・2手前）の3箇所すべてに適用する。

`History::update` と `ContinuationHistory::update` は入力を±4000にクランプ
するため、上限2122はそのまま通る。クランプ幅の変更はこのADRでは行わない。

やねうら王はこれに加えて、bonusへ `353·(bestMove == ttMove)` と
`(ss-1)->statScore/32` を足し、malusを後方の手ほど減衰させる（`×977/1024`）。
どちらも本エンジンには前提が欠けている（statScoreが無い、
外れた手の順序を保持していない）。1機能=1SPRTの規約に沿って別ADRで扱う。

検証は[ADR-0028](0028-pruning-extensions.md)の既定条件で行う。
elo0=0、elo1=5、α=β=0.05。

このADRの変更は単体で棋力を上げるとは限らない。move orderingの質が
変わるだけで、枝刈りの判断材料としての価値はhistory pruningなど後続の
機能まで現れない。H0でも、実効値域が広がったこと自体は測定で確認し、
[ADR-0072](0072-history-pruning.md)の再挑戦は進める。その場合は
非劣性（elo0=-5、elo1=0）での取り込みを検討する。

## Consequences

historyの実効値域が数百から数千へ広がる。他エンジンの係数を参照する
根拠ができ、IDEAS.mdの「LMRの項追加」「capture系の枝刈り」
「history pruning」がいずれも成立する土台になる。

move orderingの挙動が全面的に変わる。[ADR-0047](0047-continuation-history.md)や
[ADR-0025](0025-move-ordering.md)で決めた構成そのものは変えないが、
効き方は変わる。killerやcounter moveとの相対的な強さも動く。

malusがbonusより大きくなるため、historyの分布は負側に偏る。
history pruningの閾値を決め直すときは、この偏りを織り込んで実効分布を
測る必要がある。理論値域からの換算は[ADR-0072](0072-history-pruning.md)で
一度失敗している。

採択後は、bonus/malusの係数そのものが1調整=1SPRTの対象になる。
やねうら王の係数は将棋向けに調整されたものだが、本エンジンの
テーブル次元（main historyが`[駒][移動先]`しかない）とは前提が違う。
IDEAS.mdの「main historyの次元拡張」を実施したら再調整する。
