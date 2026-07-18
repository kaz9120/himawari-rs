# 0028: 枝刈り・延長パッケージ

- Status: accepted
- Date: 2026-07-18
- 関連ADR: [0024](0024-search-v1.md), [0025](0025-move-ordering.md), [0027](0027-sprt-framework.md)

## Context

探索v1（ADR-0024）は骨格のみで、枝刈りは置換表カットと
mate distance pruningしかない。現在の弱さは探索能力の問題であり
（ROADMAP）、ここに何をどの順で積むかがP3の本体になる。
このADRで決めるのは、導入する手法の一覧と順序、1機能=1SPRTの
運用規約、パラメータの出発点。

## 選択肢と比較

手法自体はStockfish・やねうら王で有効性が確立しており、
「何を入れるか」より「どう検証しながら入れるか」が論点になる。
一括導入は実装が速いが、効果の切り分けができず、バグの混入点も
特定できない。1機能ずつSPRTを通す方式は遅いが、各手法の寄与が
記録に残り、退行を持ち込まない。後者を採用する。

## Decision

### SPRT運用規約

- 1機能=1SPRT。合格（H1採択）した変更だけをmainに取り込む
- 既定条件: `selfplay --openings openings/start_sfens_ply24.txt
  --tc 10+0.1 --concurrency 6`、elo0=0、elo1=5、α=β=0.05
- 強化変更はelo0=0/elo1=5。簡素化・等価リファクタの非劣性確認は
  elo0=−5/elo1=0で行い、H1採択（elo≒0以上の証拠）で取り込み可
- 単体では強さに現れない配管（評価値の受け渡し等の基盤）は、
  それを使う最初の機能と合わせて1つのSPRTで検証してよい
- 結果（対局数、W-D-L、Elo±CI、LLR）をコミットメッセージに記録する
- H0になった強化は取り込まない。パラメータを変えた再挑戦は可
- nodesモードは再現・デバッグ用。ゲートは必ず時間制で行う

### 導入順序

効果と依存関係から次の順で入れる。各項目が1SPRT。
実装時の知見で順序の入れ替えは可（規約が守られていればよい）。

1. 静的評価のTT保存と再利用（tt.evalは現在未使用）、improving判定。
   以降の枝刈りの判断材料になる基盤
2. NMP（null move pruning）。coreに `do_null_move` /
   `undo_null_move` を追加する（王手中は禁止、plies_from_null=0で
   千日手走査は既に遮断される）。将棋はzugzwangが稀で有効性は実証済み
3. LMR（log式リダクション。履歴・PV・王手で調整）
4. futility pruning（子ノード）とreverse futility（親ノード）
5. move count pruning（LMP）
6. 本探索でのSEE枝刈り（負SEEの取る手・quietの遅い手）
7. qsearchへの静かな王手（QuietChecks）追加（ADR-0024の持ち越し）
8. singular extension（TTの深さ・手を使うため後段）
9. IIR（internal iterative reduction）

### パラメータ

- 初期値はStockfish・やねうら王の実績値を出発点にする。
  将棋固有の補正（持ち駒による手数の多さ等）は最初から凝らず、
  SPRTの結果で直す
- 探索定数は `search.rs` 冒頭にまとめて置き、機能ごとに散らさない
- 定数の再調整も「1調整=1SPRT」で行う

### スコープ外

mate1ply・df-pn（詰み探索ADR）、入玉宣言勝ち、MultiPV・ponder、
Lazy SMPは別ADR。バックログの区分どおり。

## Consequences

- 各手法の寄与Eloがコミット履歴に残り、退行の原因特定が容易になる
- 序盤の手法は効果が大きくSPRTは速く終わる。効果が数Eloに
  収束してきたら、elo1を下げるか判定を長時間化する必要がある。
  その時点の運用変更はこのADRの改訂で扱う
- NMPのためにcoreへnull move操作が入る。perftや千日手判定への
  影響はないが、`plies_from_null` の意味が実際に使われ始めるため、
  repetition系のテストにnull move経由のケースを追加する
- 1機能ずつの直列導入のため、P3の探索強化には相応の期間がかかる。
  対価として「どこで強くなったか分からない」状態を避けられる
