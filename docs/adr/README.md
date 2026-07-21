# ADR索引とバックログ

本プロジェクトの設計判断はすべてADRとして記録する。書式と運用は
[ADR-0001](0001-adr-process.md) を参照。番号は全体の通し番号で、
起草した順に採番する。バックログ上の未起草の決定には番号を付けない。

開発は「ADR群を書く→実装→検証」をフェーズ単位で繰り返す。
各フェーズには出口条件があり、通過するまで次フェーズのADRを書かない。

| フェーズ | 内容 | 出口条件 |
|---|---|---|
| P0 | リポジトリ基盤・横断方針 | CIが緑、ADRプロセス稼働 |
| P1 | 盤面表現・指し手生成 | perft既知値一致（depth 5 = 19,861,490） |
| P2 | USI + 探索v1（駒割評価） | GUIで対局完走、詰将棋正答 |
| P3 | 探索強化 + Lazy SMP + ルール完全対応 | NPSスケーリング確認、SPRTゲート運用開始 |
| P4 | NNUE推論 | 差分計算=全計算の一致、SIMD=スカラー一致 |
| P5 | 学習パイプライン | 自前学習ネットが駒割にSPRT有意勝ち |

## 起草済みADR

| ADR | タイトル | フェーズ | Status |
|---|---|---|---|
| [0001](0001-adr-process.md) | ADRテンプレートと運用 | P0 | accepted |
| [0002](0002-cargo-workspace.md) | cargo workspace構成とクレート分割 | P0 | accepted |
| [0003](0003-toolchain.md) | ツールチェイン方針 | P0 | accepted |
| [0004](0004-unsafe-policy.md) | unsafe方針 | P0 | accepted |
| [0005](0005-static-tables.md) | 静的テーブル生成戦略 | P0 | accepted |
| [0006](0006-ci-test-bench.md) | CI・テスト・ベンチマーク戦略 | P0 | accepted |
| [0007](0007-versioning.md) | エンジンのバージョニング戦略 | P0 | accepted |
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
| [0046](0046-correction-history.md) | correction history（静的評価の履歴補正） | P8 | proposed |

## バックログ

未起草の決定事項。起草時に通し番号を採番し、上の表へ移す。

### P1: 盤面表現・指し手生成

（すべて起草済み）

### P2: USI + 探索v1

（すべて起草済み）

### P3: 探索強化 + 並列化 + ルール完全対応

| 決定事項 | 主要論点 |
|---|---|
| df-pn詰み探索 | 長手数詰み。mate1ply（ADR-0029）の後段。任意 |

### P4: NNUE推論

（ADRはすべて起草済み。実装はROADMAP参照）

| 決定事項 | 主要論点 |
|---|---|
| nnueクレート分離の要否 | ADR-0002の当初計画はnnue独立クレート。現状はengine内実装。P5の学習器との共有範囲を見てから判断 |

### P6: 学習基盤の完成

（ADR-0040で起草済み。early stopping・実験レジストリも同ADR内）

### P7: ネットワーク構造の探索・決定

| 決定事項 | 主要論点 |
|---|---|
| 利き塔のon/off | NPSコスト（-40%）に見合うか。学習済みネットでSPRT判定 |
| 新特徴量の設計 | 差分計算可能な特徴量の探索。LLM技術（attention等）の局所適用 |
| 出力ヘッド設計 | WDL・進行度・安定度の多ヘッド化。探索への供給 |
| FT次元・隠れ層構成 | 256→512等の拡大。NPSとのトレードオフ |
| output bucket | 駒数で層分岐。PSQT直結パス |
| factorizer | 学習時のみのK/P分解で汎化加速 |

### P8: 本格学習 + 探索改善 + データ拡大

| 決定事項 | 主要論点 |
|---|---|
| gensfen設計 | 開始局面多様化、勝敗ラベル。持将棋は24点法で裁定（2028年選手権から27点法→24点法変更予定） |
| 宣言勝ちの24点法対応 | ADR-0030は27点法（CSA）。24点法モードを追加しUSIオプションで切替 |
| RL世代ループ | 自己対局RL、gensfen自前生成、世代ループ |
| 探索改善パッケージ | correction history、singular extension再挑戦など（IDEAS.md参照） |
