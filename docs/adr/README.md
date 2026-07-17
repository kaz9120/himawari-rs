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

## バックログ

未起草の決定事項。起草時に通し番号を採番し、上の表へ移す。

### P1: 盤面表現・指し手生成

| 決定事項 | 主要論点 |
|---|---|
| 座標系（Square/File/Rank） | 筋優先 vs 段優先、Inv/Mir、BonaPiece整合 |
| Bitboardレイアウト | u128 vs [u64;2]、Qugiyとの相性 |
| Piece/PieceTypeエンコーディング | 5bit構成、駒順とBonaPiece計算 |
| 飛角香の利き生成方式 | magic vs PEXT vs Qugiy |
| Moveエンコーディング | 16bit vs 32bit、Move16/Move32二本立て |
| Hand（手駒）パック表現 | ビットフィールド、優等/劣等判定 |
| Position構造とdo/undo | StateInfoスタック、NNUE差分計算（DirtyPiece）の要件を先読み |
| Zobristハッシュ | board/hand分離、手番の扱い |
| 王手・pin・合法性判定 | checkers/blockers差分、打ち歩詰め |
| 指し手生成の分類と成り規約 | 段階区分、不成の扱い |
| SFEN入出力とperft基盤 | パーサ、perftハーネス |

### P2: USI + 探索v1

| 決定事項 | 主要論点 |
|---|---|
| USI実装アーキテクチャ | stdinスレッド、stop割り込み、setoption |
| 探索スレッドモデル | 常駐プール、Lazy SMP前提の構造 |
| ロックレス置換表 | AtomicU64×2 Relaxed、世代管理 |
| 評価関数インターフェース | 駒割+PSQT、DirtyPiece契約の固定 |
| 探索アルゴリズムv1 | alpha-beta+反復深化、静止探索 |
| 指し手オーダリング | TT→captures→killer→history、将棋SEE |
| 時間管理 | byoyomi/inc混在、2段階時間 |
| 千日手まわり | 連続王手、優等/劣等局面 |

### P3: 探索強化 + 並列化 + ルール完全対応

| 決定事項 | 主要論点 |
|---|---|
| 強さ検証基盤（SPRT） | 対局マネージャ、SPRTパラメータ |
| Lazy SMP設計 | 共有物の範囲、最終手の選び方 |
| 枝刈り・延長パッケージ | NMP/LMR/singular、1機能=1SPRT規約 |
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
