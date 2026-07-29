# ロードマップと現在地

このプロジェクトの「やりたいこと・状況」はすべてこの文書で管理する。
GitHub Issuesは使わない。設計判断は [ADR索引](adr/README.md)、
未起草の設計論点はADR索引のバックログにある。
フェーズ管理は[ADR-0068](adr/0068-sprt-driven-versioning.md)で終え、
現在地・現行の最強構成・残作業の3節で運用する。
計測・検証の結果は [RESULTS.md](RESULTS.md)、改善アイデアは
[IDEAS.md](IDEAS.md)、教師データの所在は [DATASETS.md](DATASETS.md) を参照。
更新タイミングは、フェーズの出入り・残作業の増減があったとき。

## 現在地

- バージョン: 0.13.7（2026-07-29時点。棋力向上3件でMINORが3回上がった）
- フェーズ: **P8進行中（本格学習と探索改善）**

評価関数の構造は純粋HalfKP 256x2-32-32で確定した。19.9億局面の学習に
factorizerを併せ、対halfkp_370Mで**+243.8 Elo**、factorizer分で
**+28.1 Elo**を得た（[ADR-0066](adr/0066-halfkp-factorizer.md)）。
FT次元を512へ広げる案は、評価精度で上回るもののNPSが0.65倍に落ちる
代償が大きく不採択になった（[ADR-0067](adr/0067-ft-dimension-512.md)）。

学習基盤は41,000から449,000 samples/sへ改善し、19.9億局面の1エポックが
2時間で回るようになった（[ADR-0064](adr/0064-dense-ft-gradient-mps.md)・
[ADR-0065](adr/0065-large-scale-dataloader.md)）。

探索は2026-07-21〜25のキャンペーンで8件を採択し、単純加算で約+544 Elo
を積んだ（correction history +44.6、continuation history +20.7、
eval hash +54.1、singular +12.6、ProbCut +44.2、qsearch TT +113.6、
razoring +184.8、思考時間の難易度スケール +69.3）。

2026-07-28〜29に選定基準を[ADR-0089](adr/0089-improvement-criteria.md)へ
起こし、速度・ノード効率・終盤の正確さの3軸で候補を評価する運用にした。
「どの軸に乗るか」と「単独で効く仮説」を書けない候補は着手しない。
[ADR-0084](adr/0084-lmr-cutnode.md)（LMRのcutNode項）は構造的な理由だけで
着手して912局を使い、-1.1の中立で棄却した。この失敗が基準づくりの契機に
なっている。

基準を置いた後の2件は
[ADR-0085](adr/0085-correction-history-multi.md)（correction historyを
3系統へ、+17.7）と[ADR-0090](adr/0090-see-pruning.md)（lmrDepth基準と
SEE枝刈り、+45.6）である。後者は枝刈りを危険度で並べ直したときに
「危険度の低いSEEベースの2件だけが未実装」と分かって着手した。

同時期に実装の穴を5件埋めた。MultiPV出力の降順が崩れる不具合、詰みを
読み切っても深さ127まで回る挙動（[ADR-0088](adr/0088-mate-early-stop.md)）、
`seldepth` と `currmove` の欠落（[ADR-0086](adr/0086-search-observability.md)）、
aspirationのfail high/lowを報告していない件
（[ADR-0092](adr/0092-aspiration-bound-info.md)）、SEEが駒打ちを解いて
いない件（[ADR-0091](adr/0091-see-drop.md)）である。測定側も
[ADR-0087](adr/0087-sprt-resume.md)でSPRTに中断耐性を入れ、
`scripts/verify-feature.sh` で機能検証の局面と深さを固定した。

2026-07-24〜25にfloodgateへ初参戦した（30局19勝11敗、レート3186）。
負けはすべて相手レート3121以上で、実力の境界が3100〜3250にある。
負け11局はすべて長手数（105手以上）で、終盤10手の消費が7〜9秒に
落ちていた。持ち時間の枯渇であり、時間配分の設計が原因
（[ADR-0059](adr/0059-easy-move-scaling.md)で対処）。

当面の目標は2027年5月の世界コンピュータ将棋選手権への参加で、
参加時のビルドを1.0.0とする（[ADR-0068](adr/0068-sprt-driven-versioning.md)）。
そこまでは棋力向上をSPRTで1件ずつ積み、採択のたびにMINORを上げる。
実力の壁はfloodgateのレート3100〜3250にあり、まずここを越える。

計測の詳細は[RESULTS.md](RESULTS.md)を参照。開発の進め方は
[ADR-0070](adr/0070-pr-based-workflow.md)（PRベース、種別ごとのマージ条件）。

### 現行の最強構成

| 項目 | 実体 |
|---|---|
| エンジン | FT256ビルド（`data/bin/himawari-ft256`） |
| 評価関数 | `data/nets/halfkp_1900M_fact.hmwr.best`（valid loss 0.49513） |

FT512は評価精度で上回る（valid loss 0.49374、train loss 0.48338）が、
NPSが0.65倍に落ちる代償を取り返せず、SPRTで-72.8 Elo（968局、H0採択）
だった（[ADR-0067](adr/0067-ft-dimension-512.md)）。比較を続けられるよう、
512のバイナリ（`data/bin/himawari-ft512`）とネット
（`data/nets/halfkp_1900M_ft512.hmwr.best`）も残してある。

## 残作業

- 定跡（[ADR-0063](adr/0063-book-loader-and-mini-book.md)）は実装・生成
  ともに完了（133局面）。`book-v1` としてGitHub Releaseで配布している
  （[ADR-0082](adr/0082-book-release.md)）。floodgateへ投入して実戦で
  効果を見る。`BookFile` の指定が要る（既定は定跡なし）。現行ネットでの
  作り直しは実戦の結果を見てから判断する
- 時間管理の残り: 配分式そのものの再設計はIDEAS.mdの
  「探索: 時間管理」節に残した。やねうら王のmove_horizon方式が参考
- 容量を保ったままNPSを取り戻す。FT次元の拡大
  （[ADR-0067](adr/0067-ft-dimension-512.md)）は容量律速の診断を裏づけた
  一方、NPS 0.65倍の損で不採択になった。次はFT重みのi8量子化
  （IDEAS.mdの「量子化の再検討」）でメモリ帯域を半減させ、512の容量を
  活かせるかを試す
- 探索改善キャンペーンの継続（2026-07-23オーナー決定）:
  運用は従来どおり1アイデア1ADR・チューニングなし・SPRTゲート
  （[CLAUDE.md](../CLAUDE.md)参照）。2026-07-27にやねうら王masterと
  機能差分を棚卸しし、候補30件をIDEAS.mdの探索4節に整理した。
  候補の選定は[ADR-0089](adr/0089-improvement-criteria.md)の3軸で行う。
  機能差分は「やってないこと」のリストであって「効くこと」のリストでは
  ないため、差分の大きさだけを理由に着手しない
- LMRの項は群としてまとめて移植する。1項ずつ積む方針は
  [ADR-0076](adr/0076-lmr-fixed-point-ttpv.md)（ttPv、-43.8）と
  [ADR-0084](adr/0084-lmr-cutnode.md)（cutNode、-1.1）で2件続けて
  否定された。配管は `feat-adr0084-cutnode` ブランチに残してある
- ~~`mate_1ply` の探索組み込み~~: [ADR-0093](adr/0093-mate1ply-in-search.md)で
  棄却（-57.5）。発動率は終盤13.9%と基準を大きく超えたが、呼び出しコストが
  効果を上回った。再挑戦するなら指さずに判定する方式への書き換えから
- 速度軸（[ADR-0089](adr/0089-improvement-criteria.md)の軸1）の
  キャンペーン中。2026-07-29にプロファイルを取り、突出した3か所が
  見つかった。NNUEの隠れ層（23%）、指し手の最大スコア探索（17%）、
  FTの差分更新（14%）である。1件目は
  [ADR-0099](adr/0099-nnue-dot-sdot.md)で+59.7 Elo（NPS +20.5%）。
  残りは指し手選択のSoA＋SIMD化（ADR-0100として起草中）と、
  `MoveList` のゼロ埋め・Vec確保の除去（memset 1.8%＋malloc 1.4%）。
  FT差分はメモリ帯域律速で、IDEAS.mdの「量子化の再検討」に送った

### 2026-07-29の測定から見えた傾向

7件を測って、効いたものと効かなかったものがきれいに分かれた。

| 効いた（判定の精度を上げる） | 効かなかった（枝刈りの強度を変える） |
|---|---|
| SEEの駒打ち対応 +67.0 | RFPマージン緩和 -139.4 |
| SEE枝刈り +45.6 | mate1ply組み込み -57.5 |
| correction history 3系統 +17.7 | LMRのcutNode項 -1.1 |
| | capture history +4.0（閾値未達） |

**枝刈りの強度は既に釣り合っており、動かすと壊れる。判定材料の精度には
まだ伸びしろがある。** 次の候補もこの軸で選ぶ。

同日に速度軸も開き、[ADR-0099](adr/0099-nnue-dot-sdot.md)で+59.7 Eloを
得た。出力が1ビットも変わらない変更で、機能検証では4局面すべての
ノード数が一致している。**判定材料にも枝刈りにも触れずに済む3本目の軸が
あった**ことになる。ただしNPS +20.5%に対してElo +59.7で、eval hash
（NPS +10.4%で+54.1）と比べるとNPS 1%あたりの効きは半分だった。
速度の伸び率をそのままEloの期待値に読み替えない。

棄却4件からそれぞれ別の教訓が出ている。移植元の周辺機能が揃っているかを
確かめる（[ADR-0084](adr/0084-lmr-cutnode.md)）、発動率が高くてもコストを
併せて測る（[ADR-0093](adr/0093-mate1ply-in-search.md)）、マージンの数値は
条件と組でしか意味を持たない（[ADR-0096](adr/0096-rfp-margin.md)）、
先行する枝刈りが後段の履歴の学ぶ機会を奪う
（[ADR-0097](adr/0097-capture-history.md)）。
- ADR-0052（NMP動的化）は保留で確定（2026-07-23オーナー決定）。
  実装はadr-0052-wipブランチ。チューニング段階で再訪する
- データの残り: hao_depth9のstart_time=1695872823（127ファイル、
  約10億局面）が未取得。29.9億まで増やせるが、容量律速のうちは
  伸びが逓減する。FT拡大の後に判断する
- 持ち越し: x86_64でのperft・ベンチ再計測（環境入手待ち）、
  df-pn（任意）
