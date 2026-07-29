# 0106: ponderhitで探索を止めずに時間制限だけ差し替える

- Status: proposed
- Date: 2026-07-29
- 関連ADR: [0033](0033-ponder.md), [0104](0104-ponderhit-time-accounting.md), [0021](0021-time-management.md), [0020](0020-search-threading.md), [0074](0074-feature-verification.md)

## Context

[ADR-0033](0033-ponder.md)は、ponderhitで探索を無音キャンセルして実時間で
**再起動**する設計を採った。「TTが木を即復元する」ためコストは小さい、と
いう見立てだった。

[ADR-0104](0104-ponderhit-time-accounting.md)がこの見立てを崩した。ponderで
読んだ時間を予算に数える案をSPRTにかけ、-117.8 Eloで棄却された。負けた
理由は会計そのものではなく、**再起動が反復深化の途中経過を捨てること**に
あった。予算を使い切った状態で再起動すると深さ1で打ち切られ、ponderで
到達した深さが失われる。

Stockfishはponderhitで探索を継続する（`src/search.cpp`）。`ponder` フラグを
下ろすだけで探索状態は保つ。だから予算を使い切った状態でponderhitを
受けても、そのとき探索は既に深い。

再起動が捨てているのはTTに入らないものである。反復深化の周回、
root手の並び、[ADR-0059](0059-easy-move-scaling.md)の `stable_iters` と
前イテレーションのスコア。TTは局面ごとの値を復元するが、
「どこまで読み進めたか」は復元しない。

### 軸と仮説

[ADR-0089](0089-improvement-criteria.md)の軸1（速度）に近い。同じ持ち時間で
読める量が増え、探索の中身は変えない。

**単独で効く仮説**: ponderが当たったとき、現状は相手の手番で積んだ探索を
捨てて深さ1から読み直す。捨てずに続ければ、同じ持ち時間でより深い結論に
届く。

## 選択肢と比較

### 案A: 再起動のまま、TTの復元を速くする

置換表の当たりを増やす方向。ただし反復深化の周回そのものは復元できない。
深さ1から積み直す往復は残る。

### 案B: 探索を止めず、時間制限だけを差し替える

ponder中は無制限で走らせ、ponderhitで実時間の制限を入れる。探索スレッドは
何も知らずに走り続け、次の判定で新しい制限に従う。Stockfishと同じ形になる。

## Decision

案Bを採用する。

時間制限を原子変数の `TimeCtl` に持たせ、`TimeManager` はそれを読むだけの
薄い層にした。`go ponder` では `disarm()` で無制限にし、`ponderhit` で
`arm()` して実時間の制限を入れる。探索は止めない。

### 会計は変えない

計時の起点は **ponderhitの瞬間のまま**にする。`go ponder` の受信時刻へ
戻す変更は[ADR-0104](0104-ponderhit-time-accounting.md)で測って棄却済み
なので、混ぜない。本ADRは「同じ予算でより多く読む」だけの変更になる。

会計の再挑戦は本ADRが入ったあとに別途行う。そのときは前提が揃っている。

### 2手指し防御を壊さない

[ADR-0033](0033-ponder.md)の状態機械は残す。変えたのは `Hit` の意味である。

| 状態 | 変更前 | 変更後 |
|---|---|---|
| `Searching` → ponderhit | `Hit` にしてstopを立て、待って再起動 | `Hit` にして制限を差し替えるだけ |
| `Hit` で探索終了 | bestmoveを出さない（再起動側が出す） | **bestmoveを出す**（これが本番の結論） |
| `FinishedHolding` → ponderhit | `Stopped` にして即bestmove | 変更なし |
| stop | `Stopped` にしてbestmove | 変更なし |

ponder中に探索が自然終了したときの保留（`FinishedHolding`）は従来どおり
働く。GUIの指示より先にbestmoveを出すことはない。

### 実装の注意

- ヘルパースレッドは従来どおり無制限で、`TimeManager::unlimited()` を
  持つ。`TimeCtl` を共有するのはメインだけ（[ADR-0020](0020-search-threading.md)）
- ponder中はノード数と `movetime` の制限も外す。時間は `TimeCtl` が
  無制限でも、`limits.nodes` が残っていると途中で止まる
- `TimeCtl` の書き換えは起点を先に置く。探索側が古い起点と新しい制限を
  同時に見ても、経過が過大に出るだけで時間切れ側へ倒れる

## 検証

機能検証（[ADR-0074](0074-feature-verification.md)）では、`go depth N` の
ノード数が変わらないことを確かめる。時間指定なしの経路は `TimeCtl` が
無制限のままなので、固定深さの探索は影響を受けない。

ponderhit後の到達深さを実測した。8秒ponderしてからponderhit、
持ち時間300秒＋10秒加算。

| | ponderhit後 | 最終depth | nodes |
|---|---|---|---|
| 変更前（再起動） | 13.0s | 23 | 21,195,660 |
| 変更後（継続） | 15.2s | **24** | **36,366,261** |

同じ予算で1段深く、ノード数は1.7倍になった。ponderで積んだ8秒が
そのまま活きている。

SPRTは両者ponder・並列4で行う（`selfplay --ponder-both`）。1局が2コアを
使うため、並列は物理コアの半分に落とす。他は
[ADR-0028](0028-pruning-extensions.md)の既定条件。

## Consequences

- ponderが当たった局面で、同じ持ち時間でより深く読める
- ponderが外れた局面（stop）の挙動は変わらない
- `TimeManager` が原子変数越しの読み出しになる。`over_maximum` は
  2048ノードごとに呼ばれるが、Relaxedな読み2回なので無視できる
- ponderhit後の1イテレーションが長引くと、optimumを超えてから終わる
  ことがある。深い局面ほどイテレーションが重いためで、上限は従来どおり
  maximumが押さえる
- [ADR-0104](0104-ponderhit-time-accounting.md)（会計をgo ponder起点へ）の
  再挑戦が可能になる。本ADRが採択されたら次に測る
- `USI_Ponder=false` の対局では一切影響しない。SPRTの既定条件も変わらない
