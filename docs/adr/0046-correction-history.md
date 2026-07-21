# 0046: correction history（静的評価の履歴補正）を導入する

- Status: proposed
- Date: 2026-07-21
- 関連ADR: [0024](0024-search-v1.md), [0027](0027-sprt-framework.md), [0028](0028-pruning-extensions.md)

## Context

探索改善キャンペーン（2026-07-21オーナー決定、ROADMAP参照）の
第1弾。パラメータチューニングはせず、固定の初期定数で導入して
SPRTで効果を判定する。

静的評価は現在5箇所の探索制御に使われている。

- improving判定: `search.rs:437-441`
- reverse futility pruning: `search.rs:443-451`
- null move pruning の発動条件: `search.rs:453-483`
- futility pruning（子ノード）: `search.rs:518-529`
- qsearchのstand pat: `search.rs:649-658`

NNUE評価には局面クラスに依存する系統誤差がある。同じ歩形・
持ち駒構成で評価が一貫してずれるなら、探索値との乖離を履歴に
蓄積して静的評価を補正できる。枝刈りの判断精度が上がり、
Stockfish 16以降で最も利得の大きい改善の一つになっている。

## 選択肢と比較

### 論点1: 補正テーブルを引くキー

**案A: 歩構造キー（新設）**。盤上の歩（と金は含めない）と
持ち歩の枚数から作るincrementalなzobristキー。SFのpawn
correction historyに相当する。将棋では歩形と歩切れが評価の
骨格であり、局面クラスの代表として妥当性が高い。欠点は
coreにキー1本を追加する変更が要ること。

**案B: 既存hand_keyの流用**。手駒全体の生値（`position.rs:412`）を
splitmix64で分散させて使う。core変更が不要な一方、盤上の歩形を
区別できず、手駒は終盤ほど激しく変わるのでクラスの安定性が低い。

**案C: 多本立て（歩構造+手駒+continuation等）**。効果は最大だが、
1SPRTでどのテーブルが効いたか切り分けられない。キャンペーンの
1アイデア1ADR方針に反する。

### 論点2: キーの計算方式

**差分更新**: `StateInfo`にpawn_keyを追加し、do_moveで差分更新する。
board_key/hand_keyと同じ扱いなのでundo系は変更不要
（`position.rs:428-511`）。コストはほぼゼロ。

**毎ノード全計算**: core変更は不要だが、歩bitboardの走査と
ハッシュ化を毎ノード払う。既存コードがincrementalキーの
インフラを持っている以上、選ぶ理由がない。

## Decision

案A（歩構造キー）+ 差分更新を採用する。効果の実績と将棋での
妥当性を優先し、core変更は小さく検証可能なので受け入れる。
案Bは案AのSPRT結果を見てから追加テーブルとして別ADRで検討する。

### 実装スケッチ

歩構造キー（core側）:
- `StateInfo`に`pawn_key: u64`を追加（`position.rs:61-72`）
- 盤上の歩は既存`zobrist::PSQ`の歩エントリをそのまま流用してXOR
- 持ち歩は新テーブル`HAND_PAWN[2][19]`（zobrist.rsに追加）を
  枚数遷移でXOR
- 更新箇所: `do_move()`の打ち・移動・捕獲の各分岐、
  `do_null_move()`（引き継ぎ）、`from_sfen()`（全計算）
- 検証: 全計算関数`compute_pawn_key()`を用意し、perft系テストで
  差分=全計算の一致をdebug_assertする（board_keyと同じ流儀）

correction historyテーブル（engine側）:
- `CorrectionHistory { table: Box<[[i16; 16384]; 2]> }`
  （手番 × pawn_key下位14bit。64KB/スレッド）
- 保持はHistory/CounterMovesと同じ流儀: thread.rsのスレッド
  ローカルに置き、goごとに貸し出し、NewGameでクリア
  （`thread.rs:138-159`）

適用（search.rs）:
- 生の静的評価`raw`（evaluateまたはTT再利用）に
  `corrected = raw + entry/8`を加えた値を、上記5箇所すべてで使う
- `eval_stack`には補正後を保存する（improvingも補正後で判定）
- TTには従来どおり生値を保存する（補正テーブルは時間とともに
  変わるため、TTヒット時に再補正する）

更新（search()のノード終了時）:
- 条件: 王手中でない、best_moveがない or 静かな手、スコアが
  詰み圏でない、boundが矛盾しない
  （`best >= beta && best <= corrected`の場合と
  `best_moveなし && best >= corrected`の場合は更新しない）
- `bonus = clamp(diff * depth / 8, -128, 128)`
  （`diff = best - corrected`）
- gravity更新: `e += bonus - e * |bonus| / 1024`（値域±1024。
  既存History（`movepick.rs:33-37`）と同形式）

初期定数（チューニングしない）: テーブル16384エントリ、
値域±1024、適用スケール1/8（補正幅は最大±128）、
bonusスケール depth/8・クランプ±128。

### 検証

- pawn_keyの差分=全計算一致テスト（perft局面で網羅）
- SPRTはADR-0028の既定条件（tc 10+0.1、elo0=0/elo1=5、
  並列8、adjudicate 2000,8）。両エンジンに
  `--option "EvalFile=data/halfkp_180M.hmwr.best"`を渡し、
  同一ネットで対戦させる

## Consequences

- 歩構造キーはcoreの公開インフラになる。後続の補正テーブル
  （手駒キー、玉位置キー等）や、将来のpawn structure cacheにも
  流用できる
- eval_stackとTTで「補正後/生値」の使い分けが生まれる。
  コードコメントで明示しないと混同しやすい
- H0（効果なし）の場合もpawn_key自体は無害なので、コアの
  キーは残して補正テーブルだけ外す選択ができる
- 見直しトリガー: ネットを再学習して評価の系統誤差が変わったとき、
  補正テーブルの寄与をSPRTで再確認する
