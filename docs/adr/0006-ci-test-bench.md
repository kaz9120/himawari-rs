# 0006: CI・テスト・ベンチマーク戦略

- Status: accepted
- Date: 2026-07-17
- 関連ADR: [0002](0002-cargo-workspace.md)

## Context

将棋エンジンの正しさは層ごとに検証手段が異なる。型のエンコードは単体テスト、
盤面操作の整合性はproperty test、指し手生成の網羅性はperft、
棋力は対局（SPRT）でしか測れない。どの層をどこで回すかを決める。

## 選択肢と比較

CIの実行環境は、ubuntu x64のみのシンプル構成か、macos arm64を加えた
matrix構成かの2択。matrixはNEON側のビルド破壊を検出できるが、
ISA依存コードが存在しない現段階では検出できる問題がなく、CI時間だけが増える。

ベンチマークは、CIで閾値回帰検知まで行う案と、ローカル計測専用とする案がある。
GitHub Actionsの共有ランナーは実行時間が不安定で、数%の性能回帰検知には
使えない。

## Decision

CIはシンプルに始める。ubuntu-latestの単一ジョブで3つを回す。`cargo fmt --check`、
`cargo clippy --all-targets -- -D warnings`、`cargo test --workspace`
である。環境の追加は必要になった時点で行う。
具体的なトリガーは次の2つ。

- SIMD等のISA依存コードが入った時点で macos arm64 を追加する（P4想定）
- perftが実装された時点で releaseビルドのテスト実行を追加する（P1想定）

テストは4層で構成する。

| 層 | 内容 | 実行場所 |
|---|---|---|
| unit | 型のエンコード/デコード、テーブル生成の正しさ | CI（debug） |
| property | do/undo往復一致、差分計算=全計算、SIMD=スカラー（proptest） | CI（debug、ケース数は控えめに） |
| integration | perftテーブル駆動（P1〜）、USIゴールデンテスト（P2〜） | CI（release、perftはdepth 5まで） |
| 対局 | SPRT（P3で基盤を作る） | ローカル/専用機。CIには載せない |

テストを書くタイミングの規約は次の3点。

1. 単体テストは実装と同じコミットで書く。テストのない実装コミットを作らない
2. property testは対象の実装より先に書く（do/undo往復テストを書いてから
   do_moveを実装する、の順）
3. フェーズの出口条件（perft一致等）はCIに組み込み、緑を維持する

criterionベンチはローカル専用とし、計測値はADRやPRの根拠として記録する。

## Consequences

- CIは1ジョブで完結し、速く安く保てる。壊れる箇所が増えるまで複雑化しない
- NEON側のビルド破壊はP4まで検出されない。開発機がApple Siliconのため、
  実際にはローカルで常時検証されており、リスクは小さい
- 性能回帰はCIで検知しない。criterionの基準値をローカルで管理し、
  疑わしい変更はP3以降SPRTで棋力への影響を直接測る
