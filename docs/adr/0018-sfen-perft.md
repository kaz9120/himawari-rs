# 0018: SFEN入出力とperft基盤

- Status: proposed
- Date: 2026-07-17
- 関連ADR: [0006](0006-ci-test-bench.md), [0014](0014-position-structure.md), [0017](0017-movegen-classes.md)

## Context

SFENはUSIの局面表現で、Positionの入出力・テスト・教師データの
すべてで使う。perftはP1の出口条件（既知値一致）であり、盤面表現と
指し手生成の正しさを立証する唯一の統合検証になる。

## 選択肢と比較

SFENパーサの置き場所は、coreに置く（Positionと同居）か、usi層に
置くかの2択。テスト・perft・教師データ生成（P5）のすべてがcoreだけで
完結すべきなので、coreに置く。KIF/CSA形式のパーサはP1では作らず、
棋譜からの学習データが必要になった時点（P5）で判断する。

perftの実装は、末端で1手ずつdo/undoする素直な実装と、深さ1で
生成数を数えるbulk counting の2案がある。bulk countingは数倍速く、
意味は同一（葉の合法手数の総和）なので採用する。ただし正しさの
基準として素直な実装も残し、浅い深さで両者の一致を確認する。

## Decision

- SFENの読み書きは `Position::from_sfen` / `Position::to_sfen` として
  coreに実装する。指し手のUSI表記変換はADR-0012のとおりMove側
- パースは不正入力に対してResultでエラーを返す（panicしない）。
  盤面の駒数上限（歩18枚等）・二歩・行き所のない駒・玉2枚の
  検証まで行い、不正局面を弾く
- `moves` 以降の指し手列の適用もfrom_sfenの責務とする
  （USIの `position` コマンドがそのまま乗る形）
- perftは `crates/tools` のbinとして実装する（ADR-0002の構成）。
  生成はAllモード（ADR-0017）、計数はbulk counting。
  `--slow` フラグで素直な実装に切り替えられるようにする
- テーブル駆動のテストデータは次の3群
  1. 平手初期局面: depth 1〜5（30 / 900 / 25,470 / 719,731 / 19,861,490）を
     CI（release）で、depth 6（547,581,517）はローカル手動で確認
  2. エッジケースSFEN集: 打ち歩詰め、pin、両王手、最大分岐局面
     （593手）、入玉形。公開されている値と照合し、値のない局面は
     素直な実装の結果を記録して回帰テスト化する
  3. SFEN往復: ランダム局面列でfrom_sfen→to_sfenの不動点性を
     property testで確認する

## Consequences

- P1の出口条件が「perft(5) = 19,861,490 がCIで緑」として機械化される
- perftのNPSがcriterionと別に記録され、盤面実装の性能回帰の
  番人になる（bulk countingの値で記録する）
- SFENの検証付きパースは教師データ読み込み（P5）で毎局面通る
  経路になるため、速度が問題になったら検証省略の高速版を
  追加する（トリガー: P5のデータローダ実装時に計測）
- KIF/CSA対応はスコープ外として明示的に先送りする
