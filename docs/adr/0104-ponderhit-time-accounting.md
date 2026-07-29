# 0104: ponderで読んだ時間を持ち時間の予算に数える

- Status: rejected
- Date: 2026-07-29
- 関連ADR: [0021](0021-time-management.md), [0033](0033-ponder.md), [0059](0059-easy-move-scaling.md)

## Context

ponderhit後の時間配分が、ponderで読んだ実績を無視している。

`ThreadPool::ponderhit`（`thread.rs`）は、ponder探索を無音キャンセルして
実時間で**再起動**する。再起動時に `TimeManager::new` を呼び直すため、
計時の起点はponderhitの瞬間になる。ponderで8秒読んでいても、
そこから満額の配分をもう一度与える。

[ADR-0021](0021-time-management.md)は「計時はgo受信時刻を起点とする。
ponder中は消費せず、ponderhitで起点を引き継ぐ」と定めていた。
[ADR-0033](0033-ponder.md)の再起動方式がこの引き継ぎを落としている。
文書と実装が食い違ったまま残っていた。

### Stockfishの設計

`src/search.cpp` と `src/timeman.cpp` を確認した。

| 項目 | Stockfish |
|---|---|
| ponder時間を予算に数えるか | **数える**。計時の起点は `go ponder` 受信時刻 |
| ponderhitでの探索 | **継続**。`ponder` フラグを下ろすだけで探索状態は保つ |
| 時間切れがponder中に来たとき | `stopOnPonderhit = true` を立て、ponderhit到着と同時に停止 |
| Ponder有効時の補正 | `if (options["Ponder"]) optimumTime += optimumTime / 4;` |

ponderの探索時間は相手の持ち時間で進むので、自分の時計は減らない。
Stockfishはその「タダの探索」を当該手の思考として数え、ponderhit後は
不足分を足すだけにする。浮いた自分の持ち時間は次の手以降に回る。

数え方を厳しくした分の埋め合わせが `optimumTime += optimumTime / 4`
である。ponderが外れれば時間は消費されないため、期待値として1.25倍が
釣り合う。**縮める会計と伸ばす補正が対になっている。**

### 軸との関係

[ADR-0059](0059-easy-move-scaling.md)と同じ時間配分の軸に乗る。
[ADR-0102](0102-move-horizon.md)は配分を一律に薄くして棄却されたが、
本ADRは「既に読んだ分を二重に数えない」という会計の修正であり、
一律の削減ではない。ponderが当たった局面だけが対象になる。

**単独で効く仮説**: ponderhit時、エンジンは同じ局面を既に深く読み終えて
いる。そこから満額の時間を足すと、限界効用の低い深さに持ち時間を
投じることになる。読んだ分を予算に数えれば、その持ち時間が終盤へ回る。

## 選択肢と比較

### 案A: 現状維持

ponder時間を数えない。ponderが当たるほど1手に投じる総思考時間が増える。
持ち時間の消費は変わらないので、時計上の損はない。ただし同じ局面に
18秒相当を投じ、他の局面には満額しか配らない偏りが残る。

### 案B: ponder時間を予算に数え、Ponder有効時にoptimumを1.25倍する

Stockfishと同じ会計にする。計時の起点を `go ponder` 受信時刻に戻し、
`optimum` に25%の補正を掛ける。再起動方式はそのまま残す。

### 案C: 案Bに加えてponderhitで探索を継続する

Stockfishと完全に揃える。再起動で失う反復深化の途中経過
（root手の並び、ADR-0059のstable_iters）を保てる。ただし
[ADR-0033](0033-ponder.md)の無音キャンセル設計を作り替えることになり、
変更が大きい。

## Decision

案Bを採用する。

会計の修正（起点の引き継ぎ）と補正（1.25倍）は分離しない。
[ADR-0059](0059-easy-move-scaling.md)が「安全弁を含む1つの条件セットは
分離してSPRTを回さない」と定めた理由がそのまま当てはまる。会計だけを
入れるとponderが当たるほど不利になり、補正だけを入れると単なる増量に
なる。どちらもStockfishの設計ではない。

案Cは分ける。再起動をやめる変更は[ADR-0033](0033-ponder.md)の
2手指し防御に触れるため、会計の効果を測ってから別ADRで扱う。

### 実装

`go ponder` 受信時刻を保持し、ponderhitでの再起動時に
`TimeManager::new` へ渡す。`optimum` の1.25倍は `EngineOptions::ponder`
（USI_Ponder）が真のときに掛ける。Stockfishはオプションの有無で判定して
おり、当該探索がponder探索かどうかでは判定していない。

定数は1個増える。出典はStockfish `src/timeman.cpp` の
`if (options["Ponder"]) optimumTime += optimumTime / 4;` で、値は既定の
まま使う。この係数は持ち時間のスケールに依存しない比率なので、
[ADR-0102](0102-move-horizon.md)で `MinimumThinkingTime` を退けた理由
（絶対時間の定数がSPRT条件で成り立たない）は当てはまらない。

### 先に埋める穴

ponder時間を予算に数えると、長考した相手のあとのponderhitで
「起動直後に持ち時間超過」が起こりうる。このとき現行実装は不正な手を
返す危険がある。

`iterate` は `root_moves` を生成順のまま並べる（`search.rs`）。深さ1の
最初のroot手を読んでいる途中で `stop` が立つと、`search_root` は
`best_idx = 0` のまま戻り、`iterate` は `root_moves[0].mv` を返す。
これは生成順の先頭の手であって、探索結果ではない。

現状でも `go infinite` の直後に `stop` が来れば同じ経路を通る。本ADRの
変更はこの経路に入る頻度を上げるため、先に塞ぐ。塞ぎ方は別ADRで扱う
（置換表の手をroot手の先頭へ出す。探索木が変わるためSPRTが要る）。

### 検証

機能検証（[ADR-0074](0074-feature-verification.md)）は、ponderを使わない
探索に影響しないことの確認になる。`go depth N` はoptimumがNoneなので
固定深さのノード数は完全一致するはずである。

SPRTには測定基盤の変更が要る。`selfplay` の `--ponder` は候補側だけを
相手番思考させる効果測定モードで、両者にponderを持たせられない
（`main.rs` の `[cfg.ponder, false]`）。本ADRは「ponderが当たったときの
会計」を比べるので、両者ともponder有効にしなければ差が出ない。

あわせて並列度を下げる必要がある。両者がponderすると1局が2コアを使い、
既定の並列8では物理コアを超えて持ち時間の消化が不安定になる
（`scripts/env.sh` が並列度を物理コアで決めている理由と同じ）。
並列4で回す。

## 結果

**-117.8 Elo [-183.0,-59.7]**（100局、49ペア、LLR -0.75で打ち切り）。
両者ponder・並列4、それ以外は[ADR-0028](0028-pruning-extensions.md)の
既定条件。CIが0を大きく下回ったため判定の確定を待たずに止めた。棄却する。

### 案Cは選択肢ではなく前提だった

会計だけを直すと、ponderで積んだ探索がまるごと捨てられる。

Stockfishがponder時間を予算に数えられるのは、**ponderhitで探索を継続
する**からである。予算を使い切った状態でponderhitを受けても、そのとき
探索は既に深い。止めれば深い結論がそのまま出る。

himawariは[ADR-0033](0033-ponder.md)の設計でponderhitに探索を再起動
する。予算を使い切った状態で再起動すると、深さ1で打ち切られる。置換表が
値を返すので不正な手にはならないが、ponderで到達した深さは失われる。
**会計を厳しくするほど、捨てる量が増える。**

既定条件（10+0.1）ではこれが顕著になる。相手の思考時間はこちらの
optimumとほぼ同じなので、ponderが当たった時点で予算をほぼ使い切る。
ponderhit後の探索は毎回ほぼ即座に打ち切られていた。

本ADRは案Cを「[ADR-0033](0033-ponder.md)の2手指し防御に触れるため、
会計の効果を測ってから別ADRで扱う」として範囲外にした。この切り分けが
誤りだった。**案Cは会計の前提であって、後回しにできる追加ではない。**

## Consequences

- 棄却する。ponderhitの計時は現状（起点をリセット）のまま残す
- 実装は `feat-adr0104-v2` ブランチに残す。ponderhitで探索を継続する
  変更（案C）を入れたあとなら、会計の修正が再び意味を持つ
- 順序が決まった。**先に案C（探索の継続）、次に会計**である。
  逆にはできない
- 教訓は[ADR-0102](0102-move-horizon.md)と同じ形である。移植元の設計は
  周辺の仕組みと組で成立している。片方だけを移すと、支えを失った側が
  損をする。ADR-0102は「見分けの仕組み」、本ADRは「探索の継続」が
  支えだった
- 測定基盤の `--ponder-both`（#99）は残る。案Cを測るときにそのまま使える
- floodgateでponderを有効にしているかの確認は引き続き要る
