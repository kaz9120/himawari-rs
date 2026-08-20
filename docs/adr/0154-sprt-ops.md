# 0154: SPRTの実行・監視・後処理を定型化する

- Status: accepted
- Date: 2026-08-10
- 関連ADR: [0028](0028-pruning-extensions.md), [0074](0074-feature-verification.md), [0098](0098-agent-permissions.md), [0122](0122-tooling-language-split.md), [0149](0149-experiment-runner.md)

## Context

SPRTは棋力が変わる変更のたびに必ず走るのに、運用が定型化されていなかった
（2026-08-09オーナー指摘）。実際に起きた問題は3つある。

1. `sprt.sh` が `exec` でselfplayに置き換わり、経過はstdoutにしか出ない。
   バックグラウンドで起動すると経過がエージェントのタスクファイルへ
   吸い込まれ、オーナーから見えなくなった（ADR-0153のSPRTで発生）
2. ログの命名と置き場が起動のたびに変わる。リダイレクト先をその場で
   決めていた時期の名残で、[ADR-0149](0149-experiment-runner.md)が
   実験ログへ課した規約にSPRTだけ乗っていなかった
3. `sprt-summary.py` は経過ログ前提なので、ログが無い回は途中経過を
   出す手段がなかった

## Decision

3点で固定する。

1. **経過ログを必ず書く**。`sprt.sh` は `run_logged`（ADR-0149）経由で
   selfplayを起動し、`data/logs/sprt-<名前>.log` へ経過を追記する。
   起動方法（前面・バックグラウンド）に関わらず同じ場所で読める。
   名前は `adrNNNN-<slug>` で、棋譜（`data/sprt/<名前>.jsonl`）・
   バイナリ（`data/bin/{base,cand}-<名前>`）と揃える
2. **途中経過の確認手段を1つにする**。`sprt-summary.py` は判定行が
   まだ無いログでは最後のpairs行から途中経過を出す（判定欄「判定前」）。
   監視の常用コマンドは summary（1回表示）・`tail -f`（流し見）・
   `watch-sprt.sh`（判定待ち）の3つ
3. **手順をスキルに固定する**。`.claude/skills/running-sprt/SKILL.md` が3つを持つ。起動
   （build-pair→verify→sprt.sh）、監視、終了後の記録と後処理
   （H1・H0・判定に至らずのそれぞれ）である。エージェントはSPRTのたびにこれを使う

条件の意味（既定条件・例外）はCLAUDE.mdの「SPRTゲート」が正のまま動かさない。
この ADRは実行の器だけを扱う。

## Consequences

- 途中経過が常に `data/logs/sprt-<名前>.log` にあり、オーナーがいつでも
  `sprt-summary.py` で確認できる
- 過去のログ名の不統一は遡って直さない。この決定以後の実行から揃える
- `run_logged` 経由になったことで、selfplayの終了コード（0=H1、1=H0、
  2=判定に至らず）は従来どおり呼び出し元へ伝わる
- [ADR-0175](0175-sprt-until-decision.md)で起動の入口が `sprt-run.sh` へ移り、
  完了の判定が `data/sprt/<名前>.result` の有無になった。スキルの手順も
  そちらへ更新した。**この ADRが定めた「実行の器をスキルに固定する」構造は
  変わらない**。器の中身が判定まで走る形に変わっただけである
