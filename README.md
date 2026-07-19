# himawari-rs

Rustで書くコンピュータ将棋エンジン（USIプロトコル対応）。

評価関数はNNUE、探索はalpha-beta＋Lazy SMPを最終形とし、
自己対局による教師データ生成と学習器まで自前で実装する。
参考: [Stockfish](https://github.com/official-stockfish/stockfish)、
[やねうら王](https://github.com/yaneurao/yaneuraou)、
[Chess Programming Wiki](https://www.chessprogramming.org/Main_Page)

## ドキュメント構成

このプロジェクトはGitHub Issuesを使わない。やりたいこと・設計・状況の
すべてをリポジトリ内の文書で管理する。

| 文書 | 役割 |
|---|---|
| [docs/ROADMAP.md](docs/ROADMAP.md) | 現在地・フェーズ進捗・残作業（状況はここが正） |
| [docs/adr/README.md](docs/adr/README.md) | 設計判断の索引と未起草バックログ（設計はここが正） |
| docs/adr/NNNN-*.md | 個々の設計判断（ADR） |

## 開発プロセス

設計判断はすべてADRとして記録し、実装より先に書く。
「ADR群を書く→実装→検証」をフェーズ単位で繰り返し、
各フェーズの出口条件（perft一致、対局完走など）を通過してから次へ進む。

## ビルドと使い方

```
cargo build --release
```

`target/release/himawari` がUSIエンジン本体。将棋所やShogiGUIに
エンジン登録して対局できる。開発用ツールは次のとおり。

```
cargo run --release -p himawari-tools --bin perft -- 5   # perft
cargo run --release -p himawari-tools --bin tsume        # 詰将棋スモーク
cargo run --release -p himawari-tools --bin makenet      # 乱数NNUEネット生成
cargo test --workspace                                   # テスト（debug）
cargo test --workspace --release                         # perft既知値の照合込み
```

自己対局・SPRT検定は `selfplay`（使い方は
`crates/tools/src/bin/selfplay/main.rs` 冒頭のコメント）。

ツールチェインは `rust-toolchain.toml` で固定している（nightly）。

## workspace構成

| クレート | 内容 |
|---|---|
| `crates/core` | 盤面表現・指し手生成・SFEN入出力（探索非依存） |
| `crates/engine` | 探索・置換表・評価・時間管理 |
| `crates/usi` | USIプロトコル層 + エンジンバイナリ `himawari` |
| `crates/tools` | 開発用ツール（perft・tsume・selfplay・makenet） |

NNUE推論は現状engineクレート内にある。クレート分離
（[ADR-0002](docs/adr/0002-cargo-workspace.md)の当初計画）の
要否はP5前に判断する（ADR索引のバックログ参照）。

## License

MIT
