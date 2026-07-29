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
| [0100](0100-movepick-argmax-simd.md) | 指し手の最大スコア探索をSoA＋SIMDにする | - | accepted |
| [0101](0101-movelist-uninit.md) | MoveListのゼロ埋めをやめる | - | accepted |
| [0102](0102-move-horizon.md) | 残り手数の見積もりをmove horizon方式にする | -107.2 | rejected |
| [0104](0104-ponderhit-time-accounting.md) | ponderで読んだ時間を持ち時間の予算に数える | -117.8 | rejected |

## バックログ

未起草の決定事項。起草時に通し番号を採番し、上の表へ移す。
フェーズ管理を終えた（[ADR-0068](0068-sprt-driven-versioning.md)）ため、
分類を持たない1つの表にまとめている。

| 決定事項 | 主要論点 |
|---|---|
| df-pn詰み探索 | 長手数詰み。mate1ply（[ADR-0029](0029-mate-search.md)）の後段。任意 |
| nnueクレート分離の要否 | ADR-0002の当初計画はnnue独立クレート。現状はengine内実装。学習器との共有範囲を見てから判断 |
| 新特徴量の設計 | 差分計算可能な特徴量の探索。LLM技術（attention等）の局所適用 |
| 出力ヘッド設計 | WDL・進行度・安定度の多ヘッド化。時間管理・枝刈り強度・contemptへの供給 |
| output bucket | 局面フェーズで最終層を分岐する。将棋は取った駒が持ち駒になり総数が減らないため、手数や盤上駒数など別の指標設計が要る（[ADR-0067](0067-ft-dimension-512.md)） |
| gensfen設計 | 開始局面多様化、勝敗ラベル。持将棋は24点法で裁定（2028年選手権から27点法→24点法へ変更予定） |
| 宣言勝ちの24点法対応 | ADR-0030は27点法（CSA）。24点法モードを追加しUSIオプションで切替 |
| RL世代ループ | 自己対局RL、gensfen自前生成、世代ループ |

| NPS回帰のCI監視 | ベンチ局面のNPSをCIで記録し退行を検知する。評価関数を固定した以上、探索改善の安全網になる |
