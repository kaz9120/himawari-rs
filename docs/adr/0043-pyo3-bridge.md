# 0043: PyO3ブリッジ（学習パイプラインのRust-Python連携）

- Status: accepted（2026-07-20オーナー承認）
- Date: 2026-07-20
- 関連ADR: [0040](0040-training-infra-v2.md), [0038](0038-training-data-format.md), [0037](0037-nnue-file-format.md)

## Context

ADR-0040で学習器をPyTorchに移行すると決めた。PSVデコード・
特徴抽出・.hmwr I/Oの実装方法として、Python再実装とRust共有の
2択がある。

Python再実装は動作するが、以下の問題がある。

- packed sfenのハフマン復号、BonaPiece計算、利き計算など
  300行超のロジックが2重実装になる
- P7で特徴量を変更したとき、両方を同期する保守負担が生じる
- データローディングの速度がPythonに律速される

## Decision

PyO3でRustのPython拡張モジュール `himawari` を作り、既存の
Rust実装をPythonから直接呼ぶ。

公開する関数:

| 関数 | 用途 | Rust側の実装 |
|---|---|---|
| extract_features | PSVレコード→特徴インデックス+ターゲット | packed_sfen::unpack, halfkp_active, effect_active |
| save_hmwr | 量子化重み→.hmwrファイル書き出し | nnue_io::save |
| load_hmwr | .hmwrファイル→量子化重み | nnue_io::load |

Pythonに残すもの: モデル定義（nn.Module）、学習ループ、
f32→int量子化（モデル定義と密結合）。

クレート構成: `crates/py`（cdylib）。ビルドは `maturin develop`。

## Consequences

- 特徴抽出・ファイルI/Oのコードが1箇所に集約される。
  P7の構造変更時にRust側だけ修正すればよい
- データローディングがC速度になる
- maturinのビルド依存が増える。学習環境のセットアップに
  `pip install maturin && maturin develop` が必要
- Python単体のdataset.py/quantize.pyはPyO3呼び出しのラッパーに
  簡素化される。Rust未ビルド時のフォールバックは設けない
