# 0038: 教師データフォーマット（PackedSfenValue互換）

- Status: accepted（2026-07-20オーナー承認）
- Date: 2026-07-20
- 関連ADR: [0018](0018-sfen-perft.md), [0034](0034-nnue-architecture.md), [0037](0037-nnue-file-format.md)

## Context

P5は公開データセットから教師あり学習を始める（2026-07-20
オーナー方針）。最初の対象は nodchip/shogi_hao_depth9
（PackedSfenValue形式、約10億局面、MITライセンス）。
教師データの読み書き形式を決める。

## Decision

### 形式はPackedSfenValueをそのまま使う

- 1局面40バイト。packed sfen 32B に score（i16、手番視点cp）と
  move（u16、Move16）が続く。さらに gamePly（u16）、
  game_result（i8、手番視点±1/0）、padding（u8）が入る
- 独自形式への変換はしない。公開データ資産とやねうら王系
  ツール群の互換を優先し、将来の自前gensfenもこの形式で書き出す
- packed sfenは256bitのLSB-firstビット列である。手番1bit、先手玉・後手玉の
  位置7bit×2、盤上駒のハフマン符号（空1bit、歩4bit、香桂銀6bit、金6bit、
  角飛8bit）、手駒の順に並ぶ。
  符号表はやねうら王 `extra/sfen_packer.cpp` を正とする

### 実装の置き場所と範囲

- coreに `packed_sfen` モジュールを実装する。decode
  （32B→Position）とencode（Position→32B、検証と将来の
  gensfen用）の両方向
- 対応範囲は平手由来の40駒全数の局面。駒箱（駒落ち）符号は
  デコードエラーにする
- 学習はscoreとgame_resultを使う。moveは指し手一致率の診断用

### ツールと検証

- toolsに `psv` ツールを追加する（先頭N件抽出、チャンク
  シャッフル、統計表示、SFENダンプ）
- 乱数局面のencode→decode roundtrip一致をCIで回す
- 実データの読み込み（合法局面として構築できること、score
  分布のサニティ）はローカルで確認する（データはリポジトリに
  含めない。`data/` 配下、gitignore）

## Consequences

- やねうら王系の教師データ資産をそのまま利用でき、推論部
  （ADR-0037の独自形式）と学習部の形式が分離する
- シャッフルは前処理で行う（配布データは未シャッフルのため必須）
- ミラー等のデータ拡張が必要になったらリーダ側のオプションで足す

## 追記（2026-08-23）: moveフィールドの符号

moveフィールドはやねうら王系のMove16で、本エンジンのMove16と
ビット割り当てが違う（あちらはdrop=bit14・promote=bit15、こちらは逆）。
さらにhao_depth9の駒打ちは、fromフィールドに駒種がオフセットなしで
入っていた（やねうら王本流の「駒種+80」とは別の流儀）。読むときは
`Move16::from_yaneura` で変換する。両方の流儀を受ける。
ADR-0185のランキング群生成で最初にこのフィールドを消費して分かった。
