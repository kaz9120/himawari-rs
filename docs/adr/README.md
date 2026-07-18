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
| [0028](0028-pruning-extensions.md) | 枝刈り・延長パッケージ | P3 | proposed |

## バックログ

未起草の決定事項。起草時に通し番号を採番し、上の表へ移す。

### P1: 盤面表現・指し手生成

（すべて起草済み）

### P2: USI + 探索v1

（すべて起草済み）

### P3: 探索強化 + 並列化 + ルール完全対応

| 決定事項 | 主要論点 |
|---|---|
| Lazy SMP設計 | 共有物の範囲、最終手の選び方 |
| 詰み探索 | mate1ply、df-pnは後回し |
| 入玉宣言勝ち | 27点法 vs 24点法 |
| MultiPV・ponder | root moves管理、info出力 |

### P4: NNUE推論

| 決定事項 | 主要論点 |
|---|---|
| NNUE特徴量設計 | HalfKP vs HalfKA_v2、BonaPiece番号付け |
| ネットワークアーキテクチャ | FT 256×2 vs 512×2、活性化関数 |
| 差分計算（accumulator） | DirtyPiece、遡り差分、玉移動refresh |
| 量子化スキーム | int16/int8、FV_SCALE |
| SIMD実装と抽象化 | std::simd主体、スカラー基準実装 |
| 評価ファイルフォーマット | やねうら王互換 vs 独自 |

### P5: 学習パイプライン

| 決定事項 | 主要論点 |
|---|---|
| 教師データフォーマット | PackedSfenValue互換 vs 独自 |
| gensfen設計 | 開始局面多様化、勝敗ラベル |
| 学習器アーキテクチャ | 自前Rust vs candle/burn vs nnue-pytorch方式 |
| 損失関数設計 | elmo式混合、勝率変換スケール |
| 学習運用と量子化整合 | QAT vs 学習後量子化 |
