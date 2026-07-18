# 0025: 指し手オーダリング

- Status: proposed
- Date: 2026-07-18
- 関連ADR: [0012](0012-move-encoding.md), [0017](0017-movegen-classes.md), [0024](0024-search-v1.md)

## Context

alpha-betaの効率はオーダリングでほぼ決まる。生成を段階化して
「良さそうな手から試し、カットが出たら残りを生成すらしない」
構造（MovePicker）にする。将棋固有の論点は、駒打ちを含む
historyの添字設計と、成り・駒打ちを含むSEE（静的交換評価）。

## 選択肢と比較

historyの添字は、チェスの定番butterfly（[color][from][to]）だと
駒打ちのfromが表現できない。移動後の駒×移動先（[piece][to]、
32×81）ならMoveの上位ビット（ADR-0012）から直接引け、駒打ちも
自然に収まる。やねうら王も同型。後者を採用する。

continuation history（1〜2手前の手との組み合わせ）は効果が
大きいが、LMR等の枝刈り（P3）と相互作用するため、SPRTで
効果を測れるP3で導入する。P2はmain history＋killer＋counterまで。

## Decision

### MovePickerの段階

```
1. TT move（置換表の指し手。擬似合法性を検査して使う）
2. 良い取る手（SEE ≥ 0。捕獲駒の価値 − 動かす駒の価値/16 で降順）
3. killer 2手 → counter move
4. 静かな手（main historyの降順）
5. 悪い取る手（SEE < 0）
```

静止探索用は「取る手のみ、同じスコアで降順。SEE < 0 は捨てる」
（ADR-0024の規約）。

### history類

- main history: `[piece(32)][to(81)]` のi16。Moveの上位ビットから
  添字が直接出る。βカットした静かな手に `+depth²`（上限あり）、
  試して駄目だった静かな手に同量のペナルティ（gravity方式で減衰）
- killer: 各plyに2手。カット時に更新
- counter move: `[piece(32)][to(81)]` に「直前の手への反撃」を1手
- スレッドローカル（ADR-0020）。対局中は持ち越し、`usinewgame` でクリア

### 将棋SEE

対象マスの攻撃駒を安い順に交換していくswapアルゴリズム。

- 攻撃駒の列挙は `attackers_to` を占有更新しながら再計算する
  （香・飛・角の「裏に控えた駒」のX線を占有の削除で自然に扱う）
- 交換中の成りは考慮しない（駒の価値は現在の駒種で固定）。
  成りを考慮する精密化は、SPRTで測れるP3以降の課題とする
- 手駒からの打ちは交換に参加しない（盤上の駒のみ）
- pinは考慮しない（Stockfish同様の割り切り）

## Consequences

- MovePickerにより、カットが早いノードではQuiets生成が丸ごと
  省かれる
- historyの添字がMoveの上位ビットから分岐なしで出る。
  ADR-0012で32bit Moveに駒情報を載せた判断の回収点
- SEEの簡略化（成り・pin無視）は交換評価を数%誤るが、
  オーダリングと静止探索の枝刈り用途では実害が小さい。
  精密化はP3でSPRTにかけて判断する
- continuation historyの追加はP3の枝刈りADRとセットで行う
