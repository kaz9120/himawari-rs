# 0024: 探索アルゴリズムv1

- Status: accepted
- Date: 2026-07-18
- 関連ADR: [0017](0017-movegen-classes.md), [0020](0020-search-threading.md), [0022](0022-transposition-table.md), [0023](0023-eval-interface.md)

## Context

P2の探索は「正しく、後で強化できる骨格」を作る段階で、
枝刈りの積み上げ（P3）の土台になる。ここで決めるのは
アルゴリズムの骨格、評価値の符号化、静止探索の範囲、PVの持ち方。
強さに関わるパラメータの調整はP3のSPRT基盤ができてから行う。

## 選択肢と比較

骨格はnegamax形式のalpha-beta＋反復深化で、実質的に選択肢はない。
判断が要るのは次の2点。

### PVの持ち方

置換表から復元する方式は追加メモリ不要だが、エントリの置換で
PVが途切れる。三角配列（各plyにPVバッファを持ち、βカット無しで
返るときに子のPVを連結する）は確実である。デバッグ時にPVを信頼できる
利点が大きい。三角配列を採用する。

### 静止探索の範囲

将棋は駒の取り合いに加えて成りの脅威があり、王手回避も必須。
v1では「取る手＋歩成（GenType::Captures、Normalモード）＋
王手されているときは全回避」とする。静かな王手（QuietChecks）の
追加はP3で検討する。

## Decision

- 反復深化: depth 1から、時間（ADR-0021のoptimum）または
  指定深さまで。depth 5以降はaspiration window（初期幅±20、
  fail時に指数拡大）
- 本探索: fail-softのalpha-beta。置換表のprobe/store
  （ADR-0022）、王手時はEvasions生成、非王手時は段階生成
  （オーダリングADRで詳細化）。合法手が1つもなければ詰まされて
  おり `mated_in(ply)` を返す
- 静止探索: stand-pat（evaluate）でβカット・α更新後、
  取る手＋歩成をSEE（オーダリングADR）で絞って探索。
  王手されているときは全回避を生成し、stand-patは使わない
- 評価値の符号化: `Value = i32`。
  `MATE = 32000`、`mate_in(ply) = MATE − ply`、
  `mated_in(ply) = −MATE + ply`、`INFINITE = 32601`、
  千日手値は千日手ADRで定義。置換表への保存・取得時に
  詰みスコアをplyで補正する（保存: 根からの距離を除去、
  取得: 現plyを加算）
- PV: 三角配列。infoのpv出力に使う
- 探索の深さ上限 `MAX_PLY = 128`。StateInfoスタック容量と整合
- info出力: 各イテレーション完了時に depth / score cp・mate /
  pv / nodes / nps / time。hashfullはTTの実装後に追加
- 宣言勝ち（入玉27点）はP3のADRで導入する。v1では扱わない

## Consequences

- P3の枝刈り（NMP/LMR等）は本探索のノード冒頭・moves loop内への
  追加として実装でき、骨格の書き直しは不要
- 詰みスコアのply補正はバグの定番なので、「TT経由で詰み手数が
  ずれない」ことを詰将棋テスト（P2出口条件）で検証する
- 静止探索に王手生成がないため、詰み周りの読み抜けが残る。
  5手詰・7手詰の正答率確認は本探索の深さで補う前提とし、
  不足ならP3のmate1ply・QuietChecksで対処する
- fail-soft運用のため、TTには窓外の値も真の上下界として入る
