# ADR索引とバックログ

本プロジェクトの設計判断はすべてADRとして記録する。書式と運用は
[ADR-0001](0001-adr-process.md) を参照。番号は全体の通し番号で、
起草した順に採番する。バックログ上の未起草の決定には番号を付けない。

設計判断はADRに起草し、実装と検証を経てacceptedにする。棋力が変わる
変更はSPRTでのH1採択がマージ条件になる
（[ADR-0070](0070-pr-based-workflow.md)）。現在地と残作業は
[ROADMAP.md](../ROADMAP.md) を正とする。

フェーズ管理（P0〜P8）は[ADR-0068](0068-sprt-driven-versioning.md)で
終えた。下の表の「起草時期」列は当時の区分を履歴として残したもので、
新規のADRには付けない。

## 起草済みADR

| ADR | タイトル | 起草時期 | Status |
|---|---|---|---|
| [0001](0001-adr-process.md) | ADRテンプレートと運用 | P0 | accepted |
| [0002](0002-cargo-workspace.md) | cargo workspace構成とクレート分割 | P0 | accepted |
| [0003](0003-toolchain.md) | ツールチェイン方針 | P0 | accepted |
| [0004](0004-unsafe-policy.md) | unsafe方針 | P0 | accepted |
| [0005](0005-static-tables.md) | 静的テーブル生成戦略 | P0 | accepted |
| [0006](0006-ci-test-bench.md) | CI・テスト・ベンチマーク戦略 | P0 | accepted |
| [0007](0007-versioning.md) | エンジンのバージョニング戦略 | P0 | superseded（[0068](0068-sprt-driven-versioning.md)） |
| [0008](0008-square-coordinates.md) | 座標系（Square/File/Rank） | P1 | accepted |
| [0009](0009-piece-encoding.md) | Piece/PieceTypeエンコーディング | P1 | accepted |
| [0010](0010-bitboard-layout.md) | Bitboardレイアウト | P1 | accepted |
| [0011](0011-slider-effect-gen.md) | 飛角香の利き生成方式 | P1 | accepted |
| [0012](0012-move-encoding.md) | Moveエンコーディング | P1 | accepted |
| [0013](0013-hand-packing.md) | Hand（手駒）のパック表現 | P1 | accepted |
| [0014](0014-position-structure.md) | Position構造とdo/undo | P1 | accepted |
| [0015](0015-zobrist-hash.md) | Zobristハッシュ設計 | P1 | accepted |
| [0016](0016-legality-check.md) | 王手・pin・合法性判定 | P1 | accepted |
| [0017](0017-movegen-classes.md) | 指し手生成の分類と成り規約 | P1 | accepted |
| [0018](0018-sfen-perft.md) | SFEN入出力とperft基盤 | P1 | accepted |
| [0019](0019-usi-architecture.md) | USI実装アーキテクチャ | P2 | accepted |
| [0020](0020-search-threading.md) | 探索スレッドモデル | P2 | accepted |
| [0021](0021-time-management.md) | 時間管理 | P2 | accepted |
| [0022](0022-transposition-table.md) | ロックレス置換表 | P2 | accepted |
| [0023](0023-eval-interface.md) | 評価関数インターフェース | P2 | accepted |
| [0024](0024-search-v1.md) | 探索アルゴリズムv1 | P2 | accepted |
| [0025](0025-move-ordering.md) | 指し手オーダリング | P2 | accepted |
| [0026](0026-repetition.md) | 千日手まわり | P2 | accepted |
| [0027](0027-sprt-framework.md) | 強さ検証基盤（SPRT） | P3 | accepted |
| [0028](0028-pruning-extensions.md) | 枝刈り・延長パッケージ | P3 | accepted |
| [0029](0029-mate-search.md) | 詰み探索（mate1ply） | P3 | accepted |
| [0030](0030-nyugyoku-declaration.md) | 入玉宣言勝ち | P3 | accepted |
| [0031](0031-lazy-smp.md) | Lazy SMP | P3 | accepted |
| [0032](0032-multipv.md) | MultiPV | P3 | accepted |
| [0033](0033-ponder.md) | ponder | P3 | accepted |
| [0034](0034-nnue-architecture.md) | NNUE特徴量とネットワーク構成 | P4 | accepted |
| [0035](0035-nnue-accumulator.md) | NNUE差分計算（accumulator） | P4 | accepted |
| [0036](0036-nnue-quantization-simd.md) | NNUE量子化とSIMD実装 | P4 | accepted |
| [0037](0037-nnue-file-format.md) | NNUE評価ファイルフォーマット | P4 | accepted |
| [0038](0038-training-data-format.md) | 教師データフォーマット（PackedSfenValue互換） | P5 | accepted |
| [0039](0039-trainer-v1.md) | 学習器v1（教師あり） | P5 | accepted |
| [0040](0040-training-infra-v2.md) | 学習器v2（PyTorch移行） | P6 | accepted |
| [0041](0041-checkpoint-format.md) | 学習チェックポイント形式 | P6 | rejected |
| [0042](0042-training-log-registry.md) | 学習ログと実験レジストリ | P6 | rejected |
| [0043](0043-pyo3-bridge.md) | PyO3ブリッジ（Rust-Python連携） | P6 | accepted |
| [0044](0044-p7-feature-experiments.md) | P7特徴量実験（玉ライン特徴・利き塔有無の比較） | P7 | accepted |
| [0045](0045-remove-effect-tower.md) | 利き塔の除去 | P7 | accepted |
| [0046](0046-correction-history.md) | correction history（静的評価の履歴補正） | P8 | accepted |
| [0047](0047-continuation-history.md) | continuation history（手系列条件の履歴） | P8 | accepted |
| [0048](0048-capture-history.md) | capture history（取る手の履歴） | P8 | rejected |
| [0049](0049-eval-hash.md) | eval hash（評価値キャッシュ） | P8 | accepted |
| [0050](0050-singular-extension-retry.md) | singular extension再挑戦 | P8 | accepted |
| [0051](0051-probcut.md) | ProbCut（浅い探索による上限カット） | P8 | accepted |
| [0052](0052-nmp-dynamic.md) | NMPの動的リダクションと検証探索 | P8 | accepted |
| [0053](0053-docs-structure.md) | ドキュメントの役割分担とメンテナンスルール | P0 | accepted |
| [0054](0054-qsearch-tt.md) | qsearchへの置換表導入 | P8 | accepted |
| [0055](0055-lmr-terms.md) | LMRの条件項（improving・history連動） | P8 | rejected |
| [0056](0056-tt-prefetch.md) | TTのprefetch | P8 | rejected |
| [0057](0057-razoring.md) | razoring（浅いノードのqsearch降格） | P8 | accepted |
| [0058](0058-iteration-start-cutoff.md) | 反復深化の次イテレーション開始の抑止 | P8 | rejected |
| [0059](0059-easy-move-scaling.md) | 思考時間の難易度スケール（安定度・評価下落・ノード集中） | P8 | accepted |
| [0060](0060-opening-book-policy.md) | 定跡の方針 | P8 | accepted |
| [0061](0061-psv-memmap-dataset.md) | 学習データの事前シャッフル廃止とRAM常駐＋fork | P8 | superseded（[0065](0065-large-scale-dataloader.md)） |
| [0062](0062-root-move-nodes.md) | root手ごとの探索ノード数の集計 | P8 | accepted |
| [0063](0063-book-loader-and-mini-book.md) | 定跡ローダと最小規模の定跡生成 | P8 | accepted |
| [0064](0064-dense-ft-gradient-mps.md) | FT勾配のdense化とMPS学習（MaskedAdam） | P8 | accepted |
| [0065](0065-large-scale-dataloader.md) | バッチ一括抽出とチャンク読みによるデータ供給 | P8 | accepted |
| [0066](0066-halfkp-factorizer.md) | 学習時のみの駒単独仮想特徴（factorizer） | P8 | accepted |
| [0067](0067-ft-dimension-512.md) | FT次元256→512（コンパイル時feature切替） | P8 | accepted |
| [0068](0068-sprt-driven-versioning.md) | SPRT採択基準のバージョニングとフェーズ管理の終了 | - | accepted |
| [0069](0069-release-notes-automation.md) | リリースノートのSPRT採択からの自動生成 | - | superseded（[0071](0071-release-please.md)） |
| [0070](0070-pr-based-workflow.md) | PRベース開発と種別ごとのマージ条件 | - | accepted |
| [0071](0071-release-please.md) | release-pleaseによるバージョン更新とリリースの自動化 | - | accepted |
| [0072](0072-history-pruning.md) | history pruning（履歴が悪い静かな手の枝刈り） | - | proposed（保留） |
| [0073](0073-history-bonus-scale.md) | history bonus/malus式の再設計 | - | accepted |
| [0074](0074-feature-verification.md) | SPRTの前に機能検証を行う | - | accepted |
| [0075](0075-razoring-margin.md) | razoringのマージンを2次式にし深さ制限を外す | - | rejected |
| [0076](0076-lmr-fixed-point-ttpv.md) | LMRのリダクションを固定小数化する（ttPv項は棄却） | - | accepted |
| [0077](0077-qsearch-futility.md) | 静止探索にfutility枝刈りとmovecount制限を入れる | - | accepted |
| [0078](0078-tt-probcut.md) | 置換表の下界を使った簡易ProbCut | - | accepted |
| [0080](0080-net-release.md) | 学習済みネットをGitHub Releaseで独立に配布する | - | accepted |
| [0081](0081-portability.md) | 開発環境をスクリプトで再現可能にする | - | accepted |
| [0082](0082-book-release.md) | 定跡をGitHub Releaseで配布し、生成条件を成果物に残す | - | accepted |
| [0083](0083-windows-static-crt.md) | WindowsバイナリをMSVCランタイム静的リンクで配布する | - | proposed |
| [0084](0084-lmr-cutnode.md) | LMRにcutNode項を入れる | - | rejected |
| [0085](0085-correction-history-multi.md) | correction historyを3系統に増やす | - | accepted |
| [0086](0086-search-observability.md) | 探索の可観測性を上げる（seldepth・currmove） | - | proposed |
| [0087](0087-sprt-resume.md) | 中断したSPRTを棋譜から再開できるようにする | - | proposed |
| [0088](0088-mate-early-stop.md) | 勝ちの詰みを見つけたら反復深化を打ち切る | - | proposed |
| [0089](0089-improvement-criteria.md) | 探索改善の選定基準を3軸で置く | - | accepted |
| [0090](0090-see-pruning.md) | lmrDepth基準を導入しSEEベースの枝刈りを入れる | - | accepted |
| [0091](0091-see-drop.md) | SEEを駒打ちに対応させる | - | accepted |
| [0092](0092-aspiration-bound-info.md) | aspirationのfail high/lowをinfoで報告する | - | accepted |
| [0093](0093-mate1ply-in-search.md) | 1手詰め判定を探索へ組み込む | - | rejected |
| [0094](0094-mate1ply-speedup.md) | mate_1plyの検証を軽くする | - | proposed |
| [0095](0095-see-promotion.md) | SEEで初手の成りを扱う（ほぼ等価と判明） | - | proposed |
| [0096](0096-rfp-margin.md) | reverse futilityのマージンと深さ上限を見直す | - | rejected |
| [0097](0097-capture-history.md) | capture historyを入れ直す（スケールを揃える） | - | rejected |
| [0099](0099-nnue-dot-sdot.md) | NNUE隠れ層の内積をSDOTで4行ずつ計算する | - | accepted |
| [0100](0100-movepick-argmax-simd.md) | 指し手の最大スコア探索をSoA＋SIMDにする | - | superseded（[0110](0110-g1-history.md)） |
| [0101](0101-movelist-uninit.md) | MoveListのゼロ埋めをやめる | - | accepted |
| [0102](0102-move-horizon.md) | 残り手数の見積もりをmove horizon方式にする | -107.2 | rejected |
| [0103](0103-root-score-gap.md) | rootの1位2位差を打ち切りの判定材料に足す案 | - | rejected |
| [0104](0104-ponderhit-time-accounting.md) | ponderで読んだ時間を持ち時間の予算に数える | -117.8 | rejected |
| [0105](0105-ttpv-propagation.md) | ttPvの伝播を直しRFPの安全弁に使う案 | - | rejected |
| [0106](0106-ponderhit-continue.md) | ponderhitで探索を止めずに時間制限だけ差し替える | -54.3 | rejected |
| [0107](0107-ponder-optimum-bonus.md) | USI_Ponderが有効なとき思考時間を1.25倍する | -1.1 | rejected |
| [0108](0108-license-gplv3.md) | ライセンスをMITからGPLv3へ変更する | - | accepted |
| [0109](0109-reference-parity.md) | 参照実装への追従を群単位で進める | - | accepted |
| [0110](0110-g1-history.md) | historyの面と更新を参照実装へ揃える（G1） | +88.5 | accepted |
| [0111](0111-g2-lmr.md) | statScoreとLMRのリダクションを参照実装へ揃える（G2） | +124.0 | accepted |
| [0112](0112-g3-pruning.md) | ムーブループの枝刈りを参照実装へ揃える（G3） | +95.2 | accepted |
| [0113](0113-g4-eval-pruning.md) | improvingの再定義とevalベース枝刈りを参照実装へ揃える（G4） | +41.1 | accepted |
| [0114](0114-g5-singular.md) | singularの条件とmulti-cut・negative extensionを参照実装へ揃える（G5） | +48.2 | accepted |
| [0115](0115-g6-qsearch.md) | qsearchを参照実装へ揃え、mate_1plyを指さない方式へ書き換える（G6） | +45.6 | accepted |
| [0116](0116-g7-timeman.md) | 停止を予約する構造へ移し最小思考時間を入れる（G7） | ±0（非劣性） | accepted |
| [0117](0117-g8-ponder.md) | ponderの会計・継続・予約を参照実装へ揃える（G8） | +19.3 | accepted |
| [0118](0118-g9-aspiration.md) | 反復深化とaspirationを参照実装へ揃える（G9） | +55.6 | accepted |
| [0119](0119-g10-book.md) | 定跡・投票・実務オプションを参照実装へ揃える（G10） | +8.0 | accepted |
| [0120](0120-after-parity.md) | 追従一巡後の方向を評価関数と探索の結合へ置く | - | proposed |
| [0121](0121-book-scale-up.md) | 定跡を損失の小さい順に掘り、上限と再開を付ける | - | proposed |
| [0122](0122-tooling-language-split.md) | 開発スクリプトを役割で3言語に分ける | - | proposed |
| [0123](0123-stop-and-resume.md) | 長時間走る処理は停止と再開ができること | - | proposed |
| [0124](0124-hot-path-allocs.md) | 挙動を変えない高速化を群でまとめて測る | - | proposed |
| [0125](0125-search-decomposition.md) | 探索本体を責務ごとに切り出す | - | proposed |
| [0126](0126-mate-score-in-training.md) | 教師データの詰みスコアは素通しのままにする | - | accepted |
| [0127](0127-net-shape-bench.md) | ネットワーク構造の探索は学習前の速度計測から始める | - | accepted |
| [0128](0128-round-robin-league.md) | 3つ以上の候補は総当たりリーグ戦で順位づける | - | accepted |
| [0129](0129-auxiliary-heads.md) | 学習時だけの補助ヘッドでFTの表現を厚くする | - | proposed |
| [0130](0130-freeze-ft.md) | FTを固定して、後段の実験を一桁速くする | - | proposed |
| [0131](0131-frozen-ft-light-head.md) | 良いFTを凍結して軽量ヘッドを載せる作り方を、本番規模で確かめる | - | proposed |
| [0132](0132-ft-distillation.md) | 太いFTの表現を、細いFTへ蒸留する | - | proposed |
| [0133](0133-effect-pretraining.md) | 利き予測でFTを自己教師あり事前学習する | - | proposed |
| [0134](0134-head-capacity.md) | 後段の容量が壁かを、上向きに振って確かめる | - | proposed |
| [0135](0135-teacher-data-3b.md) | 教師データを29.9億局面へ広げる | - | proposed |
| [0136](0136-quiet-teacher-positions.md) | 教師局面をqsearchの静止局面へ置き換えて学習する | - | proposed |
| [0137](0137-output-buckets.md) | 出力層を盤上駒数バケットで分岐する（output bucket） | - | proposed |
| [0138](0138-ft-i8-quantization.md) | FT重みをi8へ量子化して更新帯域を半減する | - | proposed |
| [0139](0139-mate1ply-in-search-retry.md) | mate_1plyを通常探索へ入れ直す | - | proposed |
| [0140](0140-king-line-features.md) | 玉ライン特徴をHalfKPへ追加する | - | proposed |
| [0141](0141-singular-rate-calibration.md) | singular率を設計点へ較正し、多段延長を再訪する | - | proposed |
| [0142](0142-dfpn-mate-search.md) | df-pnの詰み探索をrootへ並走させる | - | proposed |
| [0143](0143-spsa-tuning.md) | 探索定数をSPSAで一括チューニングする | - | proposed |
| [0144](0144-selfplay-teacher-loop.md) | 自前gensfenで教師データの世代ループを始める | - | proposed |
| [0145](0145-continual-learning.md) | 前世代のネットから継続学習で積む | - | proposed |
| [0146](0146-book-full-width-opening.md) | 定跡の浅い層を全合法手で埋める | - | proposed |
| [0147](0147-effect-bucket-features.md) | 特徴indexを被利き数でバケット化する（EffectBucket） | - | proposed |
| [0148](0148-effect-table.md) | 盤面の利きを差分で持つ | - | proposed |
| [0149](0149-experiment-runner.md) | 実験の実行とログを規約で固定する | - | proposed |
| [0150](0150-rootstrap-evaluation.md) | 世代ループの良し悪しを検証損失で測らない | - | proposed |
| [0151](0151-speedup-sweep.md) | 挙動を変えない高速化の第2弾をプロファイル起点で洗い出す | - | accepted |
| [0152](0152-floodgate-cycle.md) | floodgateの棋譜を定期回収し、分析と定跡追加を決定論の手順にする | - | proposed |

## バックログ

未起草の決定事項。起草時に通し番号を採番し、上の表へ移す。
フェーズ管理を終えた（[ADR-0068](0068-sprt-driven-versioning.md)）ため、
分類を持たない1つの表にまとめている。

| 決定事項 | 主要論点 |
|---|---|
| nnueクレート分離の要否 | ADR-0002の当初計画はnnue独立クレート。現状はengine内実装。学習器との共有範囲を見てから判断 |
| 新特徴量の設計 | 差分計算可能な特徴量の探索。LLM技術（attention等）の局所適用 |
| 出力ヘッド設計 | WDL・進行度・安定度の多ヘッド化。時間管理・枝刈り強度・contemptへの供給 |
| 宣言勝ちの24点法対応 | ADR-0030は27点法（CSA）。24点法モードを追加しUSIオプションで切替 |

| NPS回帰のCI監視 | ベンチ局面のNPSをCIで記録し退行を検知する。評価関数を固定した以上、探索改善の安全網になる |
