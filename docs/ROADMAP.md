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

- バージョン: 0.7.0（P7出口。2026-07-26オーナー承認）
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
  ともに完了（133局面）。floodgateへ投入して実戦で効果を見る。
  `BookFile` の指定が要る（既定は定跡なし）
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
  差分はムーブループ内の枝刈り・LMRの項・historyの種類・時間配分式の
  4領域に集中する。次の候補はここから選ぶ
- ADR-0052（NMP動的化）は保留で確定（2026-07-23オーナー決定）。
  実装はadr-0052-wipブランチ。チューニング段階で再訪する
- データの残り: hao_depth9のstart_time=1695872823（127ファイル、
  約10億局面）が未取得。29.9億まで増やせるが、容量律速のうちは
  伸びが逓減する。FT拡大の後に判断する
- 持ち越し: x86_64でのperft・ベンチ再計測（環境入手待ち）、
  df-pn（任意）
