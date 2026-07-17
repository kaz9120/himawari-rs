# himawari-rs

Rustで書くコンピュータ将棋エンジン（USIプロトコル対応）。

評価関数はNNUE、探索はalpha-beta＋Lazy SMPを最終形とし、
自己対局による教師データ生成と学習器まで自前で実装する。
参考: [Stockfish](https://github.com/official-stockfish/stockfish)、
[やねうら王](https://github.com/yaneurao/yaneuraou)、
[Chess Programming Wiki](https://www.chessprogramming.org/Main_Page)

## 開発プロセス

設計判断はすべてADRとして記録し、実装より先に書く。
「ADR群を書く→実装→検証」をフェーズ単位で繰り返す。
フェーズ分割・バックログ・各ADRへのリンクは [docs/adr/README.md](docs/adr/README.md) を参照。

## ビルド

```
cargo build --release
cargo test
```

ツールチェインは `rust-toolchain.toml` で固定している（stable）。

## workspace構成

| クレート | 内容 |
|---|---|
| `crates/core` | 盤面表現・指し手生成・SFEN入出力（探索非依存） |
| `crates/usi` | USIプロトコル層 + エンジンバイナリ `himawari` |

nnue / engine / tools の各クレートは対応するフェーズで追加する
（[ADR-0002](docs/adr/0002-cargo-workspace.md)）。

## License

MIT
