# ADR索引

設計判断はすべてADRに残す。実装より先に起草し、検証を経てStatusを更新する。
書式と運用ルールは[ADR-0001](0001-adr-process.md)にある。

探しているものが「今どこにいて次に何をするか」なら、この索引ではなく
[ROADMAP.md](../ROADMAP.md)を読む。

## 探し方

- 番号が分かっているなら[全ADR](#全adr)の表を引く。番号は起草順の通し番号で、
  0079は欠番である
- テーマから辿るなら[下の入口](#テーマ別の入口)を使う。主要な判断だけを抜き出してある
- **「これは試したか」を確かめるなら `rejected` の行を読む**。効かなかった案の記録は
  このプロジェクトの資産で、同じ測定を繰り返さないために残している

表のElo列はSPRTで得た値である。空欄は索引に値が載っていないことを意味し、
測った結果が別の場所にあることもある。SPRTの結果は各ADRとCHANGELOGが持つ。

## テーマ別の入口

### 基盤と規約

作業の進め方を決めている文書。着手前に読む。

- [0028](0028-pruning-extensions.md) SPRTを棋力変更のマージ条件に置く
- [0053](0053-docs-structure.md) 文書の役割分担
- [0068](0068-sprt-driven-versioning.md) バージョニングをSPRT採択基準に切り替える
- [0070](0070-pr-based-workflow.md) 開発をPRベースにし、種別でマージ条件を分ける
- [0074](0074-feature-verification.md) SPRTの前に機能検証を行う
- [0089](0089-improvement-criteria.md) 探索改善を速度・ノード効率・終盤の正確さの3軸で選ぶ
- [0109](0109-reference-parity.md) 参照実装への追従は1群1SPRTで進める
- [0149](0149-experiment-runner.md) 実験の実行とログを規約で固定する

### 盤面表現と指し手

やり直しの効かない土台の決定。P1期に集中している。

- [0010](0010-bitboard-layout.md) Bitboardレイアウト
- [0012](0012-move-encoding.md) Moveエンコーディング
- [0014](0014-position-structure.md) Position構造とdo/undo
- [0016](0016-legality-check.md) 王手・pin・合法性判定
- [0018](0018-sfen-perft.md) SFEN入出力とperft基盤
- [0173](0173-knight-non-promotion-movegen.md) 桂の3段目不成を通常の生成でも出す
- [0174](0174-mate1ply-knight-promotion.md) mate_1plyへ桂の成り王手を列挙する案（測って棄却）
- [0176](0176-lance-non-promotion-rank.md) 香の不成を2段目から3段目以降へ移す

### 探索

骨格の決定と、参照実装への追従。

- [0022](0022-transposition-table.md) ロックレス置換表
- [0024](0024-search-v1.md) 探索アルゴリズムv1
- [0025](0025-move-ordering.md) 指し手オーダリング
- [0110](0110-g1-history.md)〜[0119](0119-g10-book.md) 参照実装への追従（G1〜G10、単純加算+525.5 Elo）
- [0125](0125-search-decomposition.md) 探索本体を責務ごとに切り出す
- [0155](0155-reference-walkthrough.md) 残った乖離の監査と、整列を見送った判断
- [0160](0160-revisit-rejected-under-better-eval.md) 棄却済みの追従を掘り起こす案（導入済みと判明して棄却）
- [0162](0162-parallel-group.md) 並列の未測定3件を1群で測る（TTの手番分割は見送り）
- [0177](0177-tt-probcut-depth-slack.md) TT-ProbCutの深さ差を狭める案（測って棄却）

### 評価関数（NNUE）

構造の決定と、幅・量子化をめぐる測定。

- [0034](0034-nnue-architecture.md) NNUE特徴量とネットワーク構成
- [0035](0035-nnue-accumulator.md) 差分計算（accumulator）
- [0036](0036-nnue-quantization-simd.md) 量子化とSIMD実装
- [0127](0127-net-shape-bench.md) 構造の探索は学習前の速度計測から始める
- [0138](0138-ft-i8-quantization.md) FT重みのi8量子化
- [0156](0156-bucket-accumulator-cache.md) 玉位置ごとのaccumulatorキャッシュ
- [0159](0159-ft-width-1024.md) FT幅を1024へ拡大する
- [0168](0168-ft-dim-reorder.md) FT出力次元の並べ替えで第1層の空回りを減らす
- [0170](0170-l1-half.md) 後段のL1を16へ半減する
- [0171](0171-ft-pairwise-product.md) FT出力の対を掛けて駒対の相互作用を入れる
- [0172](0172-multihot-input.md) 入力を玉と駒のマルチホットにする案（測って棄却）

### 学習

学習器とデータの決定。データが律速だと分かるまでの経緯もここにある。

- [0038](0038-training-data-format.md) 教師データフォーマット
- [0040](0040-training-infra-v2.md) PyTorchへの移行
- [0065](0065-large-scale-dataloader.md) 大規模データの供給
- [0066](0066-halfkp-factorizer.md) 学習時だけの仮想特徴（factorizer）
- [0135](0135-teacher-data-3b.md) 教師データを29.9億局面へ広げる
- [0136](0136-quiet-teacher-positions.md) 教師局面をqsearchの静止局面へ置き換える
- [0144](0144-selfplay-teacher-loop.md) 自前生成による世代ループ

### 実戦と運用

対局・定跡・配布まわり。

- [0021](0021-time-management.md) 時間管理
- [0060](0060-opening-book-policy.md) 定跡の方針
- [0080](0080-net-release.md) 学習済みネットの配布
- [0108](0108-license-gplv3.md) GPLv3への変更
- [0146](0146-book-full-width-opening.md) 定跡の浅い層を全合法手で埋める
- [0152](0152-floodgate-cycle.md) floodgateの棋譜を定期回収する
- [0154](0154-sprt-ops.md) SPRTの実行・監視・後処理
- [0163](0163-sprt-hypothesis-choice.md) 対立仮説を着手時に決める（既定と非劣性の使い分け）
- [0175](0175-sprt-until-decision.md) SPRTを判定が出るまで走らせ、完了をファイルで検知する

## 全ADR

| ADR | タイトル | 起草日 | Elo | Status |
|---|---|---|---|---|
| [0001](0001-adr-process.md) | ADRテンプレートと運用 | 2026-07-17 |  | accepted |
| [0002](0002-cargo-workspace.md) | cargo workspace構成とクレート分割 | 2026-07-17 |  | accepted |
| [0003](0003-toolchain.md) | ツールチェイン方針 | 2026-07-17 |  | accepted |
| [0004](0004-unsafe-policy.md) | unsafe方針 | 2026-07-17 |  | accepted |
| [0005](0005-static-tables.md) | 静的テーブル生成戦略 | 2026-07-17 |  | accepted |
| [0006](0006-ci-test-bench.md) | CI・テスト・ベンチマーク戦略 | 2026-07-17 |  | accepted |
| [0007](0007-versioning.md) | エンジンのバージョニング戦略 | 2026-07-17 |  | superseded（[0068](0068-sprt-driven-versioning.md)） |
| [0008](0008-square-coordinates.md) | 座標系（Square/File/Rank） | 2026-07-17 |  | accepted |
| [0009](0009-piece-encoding.md) | Piece/PieceTypeエンコーディング | 2026-07-17 |  | accepted |
| [0010](0010-bitboard-layout.md) | Bitboardレイアウト | 2026-07-17 |  | accepted |
| [0011](0011-slider-effect-gen.md) | 飛角香の利き生成方式 | 2026-07-17 |  | accepted |
| [0012](0012-move-encoding.md) | Moveエンコーディング | 2026-07-17 |  | accepted |
| [0013](0013-hand-packing.md) | Hand（手駒）のパック表現 | 2026-07-17 |  | accepted |
| [0014](0014-position-structure.md) | Position構造とdo/undo | 2026-07-17 |  | accepted |
| [0015](0015-zobrist-hash.md) | Zobristハッシュ設計 | 2026-07-17 |  | accepted |
| [0016](0016-legality-check.md) | 王手・pin・合法性判定 | 2026-07-17 |  | accepted |
| [0017](0017-movegen-classes.md) | 指し手生成の分類と成り規約 | 2026-07-17 |  | accepted |
| [0018](0018-sfen-perft.md) | SFEN入出力とperft基盤 | 2026-07-17 |  | accepted |
| [0019](0019-usi-architecture.md) | USI実装アーキテクチャ | 2026-07-18 |  | accepted |
| [0020](0020-search-threading.md) | 探索スレッドモデル | 2026-07-18 |  | accepted |
| [0021](0021-time-management.md) | 時間管理 | 2026-07-18 |  | accepted |
| [0022](0022-transposition-table.md) | ロックレス置換表 | 2026-07-18 |  | accepted |
| [0023](0023-eval-interface.md) | 評価関数インターフェース | 2026-07-18 |  | accepted |
| [0024](0024-search-v1.md) | 探索アルゴリズムv1 | 2026-07-18 |  | accepted |
| [0025](0025-move-ordering.md) | 指し手オーダリング | 2026-07-18 |  | accepted |
| [0026](0026-repetition.md) | 千日手まわり | 2026-07-18 |  | accepted |
| [0027](0027-sprt-framework.md) | 強さ検証基盤（SPRT） | 2026-07-18 |  | accepted |
| [0028](0028-pruning-extensions.md) | 枝刈り・延長パッケージ | 2026-07-18 |  | accepted |
| [0029](0029-mate-search.md) | 詰み探索（mate1ply） | 2026-07-19 |  | accepted |
| [0030](0030-nyugyoku-declaration.md) | 入玉宣言勝ち | 2026-07-19 |  | accepted |
| [0031](0031-lazy-smp.md) | Lazy SMP | 2026-07-19 |  | accepted |
| [0032](0032-multipv.md) | MultiPV | 2026-07-19 |  | accepted |
| [0033](0033-ponder.md) | ponder | 2026-07-19 |  | accepted |
| [0034](0034-nnue-architecture.md) | NNUE特徴量とネットワーク構成 | 2026-07-19 |  | accepted |
| [0035](0035-nnue-accumulator.md) | NNUE差分計算（accumulator） | 2026-07-19 |  | accepted |
| [0036](0036-nnue-quantization-simd.md) | NNUE量子化とSIMD実装 | 2026-07-19 |  | accepted |
| [0037](0037-nnue-file-format.md) | NNUE評価ファイルフォーマット | 2026-07-19 |  | accepted |
| [0038](0038-training-data-format.md) | 教師データフォーマット（PackedSfenValue互換） | 2026-07-20 |  | accepted（2026-07-20オーナー承認） |
| [0039](0039-trainer-v1.md) | 学習器v1（教師あり） | 2026-07-20 |  | superseded |
| [0040](0040-training-infra-v2.md) | 学習器v2（PyTorch移行） | 2026-07-20 |  | accepted（2026-07-20オーナー承認） |
| [0041](0041-checkpoint-format.md) | 学習チェックポイント形式 | 2026-07-20 |  | rejected（ADR-0040のPyTorch移行により、torch.save/torch.loadで代替） |
| [0042](0042-training-log-registry.md) | 学習ログと実験レジストリ | 2026-07-20 |  | rejected（ADR-0040のPyTorch移行により、TensorBoard等で代替） |
| [0043](0043-pyo3-bridge.md) | PyO3ブリッジ（学習パイプラインのRust-Python連携） | 2026-07-20 |  | accepted（2026-07-20オーナー承認） |
| [0044](0044-p7-feature-experiments.md) | P7特徴量実験（玉ライン特徴・利き塔有無の比較） | 2026-07-21 |  | accepted（2026-07-21オーナー承認） |
| [0045](0045-remove-effect-tower.md) | 利き塔の除去 | 2026-07-21 |  | accepted（2026-07-21オーナー承認） |
| [0046](0046-correction-history.md) | correction history（静的評価の履歴補正）を導入する | 2026-07-21 |  | accepted |
| [0047](0047-continuation-history.md) | continuation history（手系列条件の履歴）を導入する | 2026-07-21 |  | accepted |
| [0048](0048-capture-history.md) | capture history（取る手の履歴）を導入する | 2026-07-21 |  | rejected |
| [0049](0049-eval-hash.md) | eval hash（評価値キャッシュ）を導入する | 2026-07-21 |  | accepted |
| [0050](0050-singular-extension-retry.md) | singular extension（TT手の単独延長）に再挑戦する | 2026-07-21 |  | accepted |
| [0051](0051-probcut.md) | ProbCut（浅い探索による上限カット）を導入する | 2026-07-21 |  | accepted |
| [0052](0052-nmp-dynamic.md) | NMPの動的リダクションと検証探索を導入する | 2026-07-21 |  | accepted |
| [0053](0053-docs-structure.md) | ドキュメントの役割分担とメンテナンスルールを定める | 2026-07-21 |  | accepted |
| [0054](0054-qsearch-tt.md) | qsearchに置換表を導入する | 2026-07-22 |  | accepted |
| [0055](0055-lmr-terms.md) | LMRに条件項（improving・history連動）を導入する | 2026-07-22 |  | rejected |
| [0056](0056-tt-prefetch.md) | TTのprefetchを導入する | 2026-07-24 |  | rejected |
| [0057](0057-razoring.md) | razoringを導入する | 2026-07-24 |  | accepted |
| [0058](0058-iteration-start-cutoff.md) | 反復深化の次イテレーション開始を時間予測で抑止する | 2026-07-25 |  | rejected（固定比率はADR-0059の係数積の粗い近似にすぎず、統合した） |
| [0059](0059-easy-move-scaling.md) | 思考時間を局面の難易度でスケールする | 2026-07-25 |  | accepted |
| [0060](0060-opening-book-policy.md) | 定跡の方針 | 2026-07-25 |  | accepted |
| [0061](0061-psv-memmap-dataset.md) | 学習データの事前シャッフルを廃止し、読み込みをRAM常駐＋forkに統一する | 2026-07-25 |  | superseded（[0065](0065-large-scale-dataloader.md)） |
| [0062](0062-root-move-nodes.md) | root手ごとの探索ノード数を集計する | 2026-07-25 |  | accepted |
| [0063](0063-book-loader-and-mini-book.md) | 定跡ローダと最小規模の定跡生成 | 2026-07-25 |  | accepted |
| [0064](0064-dense-ft-gradient-mps.md) | FT勾配をdenseにし、学習をMPSで回す | 2026-07-26 |  | accepted |
| [0065](0065-large-scale-dataloader.md) | 学習データをバッチ一括抽出とチャンク読みで供給する | 2026-07-26 |  | accepted |
| [0066](0066-halfkp-factorizer.md) | 学習時だけ駒単独の仮想特徴を併用する（factorizer） | 2026-07-26 |  | accepted |
| [0067](0067-ft-dimension-512.md) | FT次元を256から512へ拡大する | 2026-07-26 |  | accepted |
| [0068](0068-sprt-driven-versioning.md) | バージョニングをSPRT採択基準に切り替え、フェーズ管理を終える | 2026-07-27 |  | accepted |
| [0069](0069-release-notes-automation.md) | リリースノートをSPRT採択から自動生成する | 2026-07-27 |  | superseded（[0071](0071-release-please.md)） |
| [0070](0070-pr-based-workflow.md) | 開発をPRベースにし、変更の種別でマージ条件を分ける | 2026-07-27 |  | accepted |
| [0071](0071-release-please.md) | バージョン更新とリリースをrelease-pleaseで自動化する | 2026-07-27 |  | accepted |
| [0072](0072-history-pruning.md) | history pruning（履歴が悪い静かな手の枝刈り） | 2026-07-27 |  | proposed（保留。前提となるhistoryスケールの再設計待ち） |
| [0073](0073-history-bonus-scale.md) | history bonus/malus式の再設計 | 2026-07-27 |  | accepted |
| [0074](0074-feature-verification.md) | SPRTの前に機能検証を行う | 2026-07-27 |  | accepted |
| [0075](0075-razoring-margin.md) | razoringのマージンを2次式にし深さ制限を外す | 2026-07-28 |  | rejected（前提の誤り。機能検証で棄却） |
| [0076](0076-lmr-fixed-point-ttpv.md) | LMRのリダクションを固定小数化する（ttPv項は棄却） | 2026-07-28 |  | accepted（固定小数化のみ。ttPv項は棄却） |
| [0077](0077-qsearch-futility.md) | 静止探索にfutility枝刈りとmovecount制限を入れる | 2026-07-28 |  | accepted |
| [0078](0078-tt-probcut.md) | 置換表の下界を使った簡易ProbCut | 2026-07-28 |  | accepted |
| [0080](0080-net-release.md) | 学習済みネットをGitHub Releaseで独立に配布する | 2026-07-28 |  | accepted |
| [0081](0081-portability.md) | 開発環境をスクリプトで再現可能にする | 2026-07-28 |  | accepted |
| [0082](0082-book-release.md) | 定跡をGitHub Releaseで配布し、生成条件を成果物に残す | 2026-07-28 |  | accepted |
| [0083](0083-windows-static-crt.md) | WindowsバイナリをMSVCランタイム静的リンクで配布する | 2026-07-28 |  | accepted |
| [0084](0084-lmr-cutnode.md) | LMRにcutNode項を入れる（cutNodeの配管を含む） | 2026-07-28 |  | rejected |
| [0085](0085-correction-history-multi.md) | correction historyを3系統に増やす | 2026-07-28 |  | accepted |
| [0086](0086-search-observability.md) | 探索の可観測性を上げる（seldepth・currmove） | 2026-07-28 |  | accepted |
| [0087](0087-sprt-resume.md) | 中断したSPRTを棋譜から再開できるようにする | 2026-07-28 |  | accepted |
| [0088](0088-mate-early-stop.md) | 勝ちの詰みを見つけたら反復深化を打ち切る | 2026-07-28 |  | accepted |
| [0089](0089-improvement-criteria.md) | 探索改善の選定基準を3軸で置く | 2026-07-28 |  | accepted |
| [0090](0090-see-pruning.md) | lmrDepth基準を導入しSEEベースの枝刈りを入れる | 2026-07-28 |  | accepted |
| [0091](0091-see-drop.md) | SEEを駒打ちに対応させる | 2026-07-29 |  | accepted |
| [0092](0092-aspiration-bound-info.md) | aspirationのfail high/lowをinfoで報告する | 2026-07-29 |  | accepted |
| [0093](0093-mate1ply-in-search.md) | 1手詰め判定を探索へ組み込む | 2026-07-29 |  | rejected |
| [0094](0094-mate1ply-speedup.md) | mate_1plyの検証を軽くする | 2026-07-29 |  | accepted |
| [0095](0095-see-promotion.md) | SEEで初手の成りを扱う（ほぼ等価と判明） | 2026-07-29 |  | accepted |
| [0096](0096-rfp-margin.md) | reverse futilityのマージンと深さ上限を見直す | 2026-07-29 |  | rejected |
| [0097](0097-capture-history.md) | capture historyを入れ直す（スケールを揃える） | 2026-07-29 |  | rejected |
| [0098](0098-agent-permissions.md) | エージェントが待機で止まらないようにする | 2026-07-29 |  | accepted |
| [0099](0099-nnue-dot-sdot.md) | NNUE隠れ層の内積をSDOTで4行ずつ計算する | 2026-07-29 |  | accepted |
| [0100](0100-movepick-argmax-simd.md) | 指し手の最大スコア探索をSoA＋SIMDにする | 2026-07-29 |  | superseded（[0110](0110-g1-history.md)） |
| [0101](0101-movelist-uninit.md) | MoveListのゼロ埋めをやめる | 2026-07-29 |  | accepted |
| [0102](0102-move-horizon.md) | 残り手数の見積もりをmove horizon方式にする | 2026-07-29 | -107.2 | rejected |
| [0103](0103-root-score-gap.md) | rootの1位2位差を打ち切りの判定材料に足す案（実装前に棄却） | 2026-07-29 |  | rejected |
| [0104](0104-ponderhit-time-accounting.md) | ponderで読んだ時間を持ち時間の予算に数える | 2026-07-29 | -117.8 | rejected |
| [0105](0105-ttpv-propagation.md) | ttPvの伝播を直しRFPの安全弁に使う案（発動率不足で棄却） | 2026-07-29 |  | rejected |
| [0106](0106-ponderhit-continue.md) | ponderhitで探索を止めずに時間制限だけ差し替える | 2026-07-29 | -54.3 | rejected |
| [0107](0107-ponder-optimum-bonus.md) | USI_Ponderが有効なとき思考時間を1.25倍する | 2026-07-29 | -1.1 | rejected |
| [0108](0108-license-gplv3.md) | ライセンスをMITからGPLv3へ変更する | 2026-07-30 |  | accepted |
| [0109](0109-reference-parity.md) | 参照実装への追従を群単位で進める | 2026-07-30 |  | accepted |
| [0110](0110-g1-history.md) | historyの面と更新を参照実装へ揃える（G1） | 2026-07-30 | +88.5 | accepted |
| [0111](0111-g2-lmr.md) | statScoreとLMRのリダクションを参照実装へ揃える（G2） | 2026-07-30 | +124.0 | accepted |
| [0112](0112-g3-pruning.md) | ムーブループの枝刈りを参照実装へ揃える（G3） | 2026-07-30 | +95.2 | accepted |
| [0113](0113-g4-eval-pruning.md) | improvingの再定義とevalベース枝刈りを参照実装へ揃える（G4） | 2026-07-30 | +41.1 | accepted |
| [0114](0114-g5-singular.md) | singularの条件とmulti-cut・negative extensionを参照実装へ揃える（G5） | 2026-07-30 | +48.2 | accepted |
| [0115](0115-g6-qsearch.md) | qsearchを参照実装へ揃え、mate_1plyを指さない方式へ書き換える（G6） | 2026-07-30 | +45.6 | accepted |
| [0116](0116-g7-timeman.md) | 停止を予約する構造へ移し最小思考時間を入れる（G7） | 2026-07-31 | ±0（非劣性） | accepted |
| [0117](0117-g8-ponder.md) | ponderの会計・継続・予約を参照実装へ揃える（G8） | 2026-07-31 | +19.3 | accepted |
| [0118](0118-g9-aspiration.md) | 反復深化とaspirationを参照実装へ揃える（G9） | 2026-07-31 | +55.6 | accepted |
| [0119](0119-g10-book.md) | 定跡・投票・実務オプションを参照実装へ揃える（G10） | 2026-08-01 | +8.0 | accepted |
| [0120](0120-after-parity.md) | 追従一巡後の方向を評価関数と探索の結合へ置く | 2026-08-01 |  | proposed |
| [0121](0121-book-scale-up.md) | 定跡を損失の小さい順に掘り、上限と再開を付ける | 2026-08-01 |  | accepted |
| [0122](0122-tooling-language-split.md) | 開発スクリプトを役割で3言語に分ける | 2026-08-01 |  | accepted |
| [0123](0123-stop-and-resume.md) | 長時間走る処理は停止と再開ができること | 2026-08-01 |  | accepted |
| [0124](0124-hot-path-allocs.md) | 挙動を変えない高速化を群でまとめて測る | 2026-08-01 |  | accepted |
| [0125](0125-search-decomposition.md) | 探索本体を責務ごとに切り出す | 2026-08-01 |  | accepted |
| [0126](0126-mate-score-in-training.md) | 教師データの詰みスコアをどう扱うか | 2026-08-01 |  | accepted（2026-08-01オーナー判断。現行の素通しを維持する） |
| [0127](0127-net-shape-bench.md) | ネットワーク構造の探索は、学習前の速度計測から始める | 2026-08-01 |  | accepted |
| [0128](0128-round-robin-league.md) | 3つ以上の候補は総当たりリーグ戦で順位づける | 2026-08-01 |  | accepted |
| [0129](0129-auxiliary-heads.md) | 学習時だけの補助ヘッドでFTの表現を厚くする | 2026-08-02 |  | accepted |
| [0130](0130-freeze-ft.md) | FTを固定して、後段の実験を一桁速くする | 2026-08-02 |  | accepted |
| [0131](0131-frozen-ft-light-head.md) | 良いFTを凍結して軽量ヘッドを載せる作り方を、本番規模で確かめる | 2026-08-03 |  | accepted |
| [0132](0132-ft-distillation.md) | 太いFTの表現を、細いFTへ蒸留する | 2026-08-03 |  | proposed |
| [0133](0133-effect-pretraining.md) | 利き予測でFTを自己教師あり事前学習する | 2026-08-03 |  | accepted |
| [0134](0134-head-capacity.md) | 後段の容量が壁かを、上向きに振って確かめる | 2026-08-04 |  | proposed |
| [0135](0135-teacher-data-3b.md) | 教師データを29.9億局面へ広げる | 2026-08-04 |  | accepted |
| [0136](0136-quiet-teacher-positions.md) | 教師局面をqsearchの静止局面へ置き換えて学習する | 2026-08-04 |  | accepted |
| [0137](0137-output-buckets.md) | 出力層を盤上駒数バケットで分岐する（output bucket） | 2026-08-04 |  | proposed |
| [0138](0138-ft-i8-quantization.md) | FT重みをi8へ量子化して更新帯域を半減する | 2026-08-04 |  | accepted |
| [0139](0139-mate1ply-in-search-retry.md) | mate_1plyを通常探索へ入れ直す | 2026-08-04 |  | rejected |
| [0140](0140-king-line-features.md) | 玉ライン特徴をHalfKPへ追加する | 2026-08-04 |  | proposed |
| [0141](0141-singular-rate-calibration.md) | singular率を設計点へ較正し、多段延長を再訪する | 2026-08-04 |  | rejected |
| [0142](0142-dfpn-mate-search.md) | df-pnの詰み探索をrootへ並走させる | 2026-08-04 |  | proposed |
| [0143](0143-spsa-tuning.md) | 探索定数をSPSAで一括チューニングする | 2026-08-04 |  | proposed |
| [0144](0144-selfplay-teacher-loop.md) | 自前gensfenで教師データの世代ループを始める | 2026-08-04 |  | accepted |
| [0145](0145-continual-learning.md) | 前世代のネットから継続学習で積む | 2026-08-08 |  | proposed |
| [0146](0146-book-full-width-opening.md) | 定跡の浅い層を全合法手で埋める | 2026-08-08 |  | accepted |
| [0147](0147-effect-bucket-features.md) | 特徴indexを被利き数でバケット化する（EffectBucket） | 2026-08-08 |  | proposed |
| [0148](0148-effect-table.md) | 盤面の利きを差分で持つ | 2026-08-08 |  | proposed |
| [0149](0149-experiment-runner.md) | 実験の実行とログを規約で固定する | 2026-08-08 |  | accepted |
| [0150](0150-rootstrap-evaluation.md) | 世代ループでの検証損失の読み方 | 2026-08-08 |  | accepted |
| [0151](0151-speedup-sweep.md) | 挙動を変えない高速化の第2弾をプロファイル起点で洗い出す | 2026-08-09 |  | accepted |
| [0152](0152-floodgate-cycle.md) | floodgateの棋譜を定期回収し、分析と定跡追加を決定論の手順にする | 2026-08-09 |  | accepted |
| [0153](0153-superior-repetition-root-gate.md) | 優等・劣等局面の判定を探索経路内に限定する | 2026-08-09 |  | accepted |
| [0154](0154-sprt-ops.md) | SPRTの実行・監視・後処理を定型化する | 2026-08-10 |  | accepted |
| [0155](0155-reference-walkthrough.md) | 参照実装との精緻ウォークスルーで見つけた乖離を群で修正する | 2026-08-11 |  | proposed |
| [0156](0156-bucket-accumulator-cache.md) | 玉位置ごとのaccumulatorキャッシュで全計算を差分に置き換える | 2026-08-12 |  | accepted |
| [0157](0157-king-mirror-buckets.md) | HalfKPの玉位置を左右ミラーで45バケットへ畳む | 2026-08-12 |  | rejected |
| [0158](0158-mirror-factorizer.md) | 学習時だけ左右ミラーの仮想特徴を併用する | 2026-08-12 |  | rejected |
| [0159](0159-ft-width-1024.md) | FT幅を1024へ拡大する | 2026-08-12 | −0.1（互角として採択） | accepted |
| [0160](0160-revisit-rejected-under-better-eval.md) | 棄却した参照追従を1群にまとめて測り直す | 2026-08-13 |  | rejected |
| [0161](0161-hide-docs-chore-from-changelog.md) | docsとchoreをCHANGELOGから外し、バージョンを動かさない | 2026-08-14 |  | accepted |
| [0162](0162-parallel-group.md) | 並列の未測定3件を1群で測り、TTの手番分割は見送る | 2026-08-14 | +7.9（非劣性） | accepted |
| [0163](0163-sprt-hypothesis-choice.md) | 変更の性質でSPRTの対立仮説を着手時に決める | 2026-08-15 |  | accepted |
| [0164](0164-bona-piece-bitset.md) | BonaPiece集合の構築を駒ごとの走査からブロック配置へ置き換える | 2026-08-15 |  | accepted |
| [0165](0165-bona-block-layout.md) | BonaPiece集合をブロック単位のレイアウトへ変える | 2026-08-15 |  | accepted |
| [0166](0166-movepick-frame.md) | MovePickerの生成段を切り出してフレームを縮める | 2026-08-15 |  | accepted |
| [0167](0167-nnue-kernel-instructions.md) | NNUE推論の命令数を削る2案を測り、どちらも見送る | 2026-08-15 |  | rejected |
| [0168](0168-ft-dim-reorder.md) | FT出力次元を並べ替えて第1層の空回りを減らす | 2026-08-15 |  | accepted |
| [0169](0169-clip-nnz-fusion.md) | 活性の構築と非ゼロチャンクの列挙を1パスにまとめる案を棄却する | 2026-08-15 |  | rejected |
| [0170](0170-l1-half.md) | 後段のL1を16へ半減する | 2026-08-16 | +13.4 | accepted |
| [0171](0171-ft-pairwise-product.md) | FT出力の対を掛けて駒対の相互作用を入れる | 2026-08-16 | +65.4 | accepted |
| [0172](0172-multihot-input.md) | 入力を玉と駒のマルチホットにして表を75分の1にする | 2026-08-17 | −52.2 | rejected |
| [0173](0173-knight-non-promotion-movegen.md) | 桂の3段目不成を通常の指し手生成でも出す | 2026-08-17 | +11.2 | accepted |
| [0174](0174-mate1ply-knight-promotion.md) | mate_1plyへ桂の成り王手を列挙する | 2026-08-17 | −1.7 | rejected |
| [0175](0175-sprt-until-decision.md) | SPRTを判定が出るまで走らせ、完了をファイルで検知する | 2026-08-18 |  | accepted |
| [0176](0176-lance-non-promotion-rank.md) | 香の不成を2段目から3段目以降へ移す | 2026-08-18 | +1.7 | accepted |
| [0177](0177-tt-probcut-depth-slack.md) | TT-ProbCutの深さ差を4から2へ狭める | 2026-08-19 | −3.1 | rejected |
| [0178](0178-textlint-gate.md) | 日本語文書の書き方をtextlintでCIゲートにする | 2026-08-20 |  | accepted |
| [0179](0179-hmwr-cli.md) | 日常操作をhmwrコマンドひとつの入口にまとめる | 2026-08-20 |  | superseded |
| [0180](0180-hmwr-cli-in-python.md) | hmwrを独立コマンドにし、実処理をPythonへ移す | 2026-08-20 |  | accepted |
| [0181](0181-agent-surface.md) | エージェントの作業面を実態へ合わせ、規律を設定へ移す | 2026-08-20 |  | accepted |
| [0182](0182-readme-audience.md) | READMEの読み手を2つに固定し、変わり続ける事実を置かない | 2026-08-20 |  | accepted |

## バックログ

まだ起草していない設計論点。起草時に通し番号を採番し、上の表へ移す。

| 決定事項 | 主要論点 |
|---|---|
| nnueクレート分離の要否 | [ADR-0002](0002-cargo-workspace.md)の当初計画はnnue独立クレート。現状はengine内実装。学習器との共有範囲を見てから判断する |
| 宣言勝ちの24点法対応 | [ADR-0030](0030-nyugyoku-declaration.md)は27点法（CSA）。24点法モードを足してUSIオプションで切り替える。2028年の選手権から適用される見込み |
| NPS回帰のCI監視 | ベンチ局面のNPSをCIで記録し、退行を検知する |
