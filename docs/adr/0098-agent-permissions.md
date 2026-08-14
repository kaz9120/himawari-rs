# 0098: エージェントが待機で止まらないようにする

- Status: accepted
- Date: 2026-07-29
- 関連ADR: [0081](0081-portability.md), [0027](0027-sprt-framework.md)

## Context

SPRTは判定まで数十分から数時間かかる。その間、エージェントは待機して
判定を拾い、結果を記録してPRを進める。この待機に権限の確認が入ると、
オーナーが席を外している間に止まる。

オーナーから「私がいない場面でモニターの権限を要求されて止まるのは
勿体無い。24時間開発し続ける環境はまだ作れない」との指摘があった
（2026-07-29）。

原因は待機の書き方にある。これまでは次のような複合コマンドを直接
Monitorへ渡していた。

```bash
until grep -qE "^----" "$LOG" || ! pgrep -f "selfplay --baseline"; do sleep 45; done
tail -3 "$LOG"
```

構成要素（`grep`・`pgrep`・`tail`・`sleep`）はどれもClaude Codeが自動で
許可する読み取り専用コマンドである。しかし `until ... do ... done` の
複合コマンドは全体で1つの文字列として判定されるため、許可規則の
パターンに一致しない。毎回確認を求められる。

## 選択肢と比較

### 案A: 待機ロジックをスクリプトへ切り出す

`scripts/watch-sprt.sh`（SPRTの判定待ち）と `scripts/watch-ci.sh`
（PRのCI完了待ち）を置き、Monitorからはスクリプトを1行で呼ぶ。
`.claude/settings.json` の `permissions.allow` へ
`Bash(./scripts/watch-sprt.sh:*)` を書けば、パターンが一致する。

スクリプトは読み取り専用にする。ログを読み、プロセスの生存を見て、
結果を出すだけである。PRのマージのような破壊的操作は混ぜない。混ぜると
「読み取り専用だから許可した」という前提が崩れる。

### 案B: 複合コマンドを許可する規則を書く

`Bash(until *)` のような規則を足す。

`until` に続く任意のコマンドを許可することになり、事実上の任意コード
実行になる。採らない。

### 案C: 権限モードを緩める

確認なしで実行するモードへ切り替える。

SPRTの待機のために、すべての操作の確認を外すことになる。範囲が広すぎる。

## Decision

案Aを採る。

### 待機スクリプト

| スクリプト | 役割 | 終了コード |
|---|---|---|
| `scripts/watch-sprt.sh <ログ> [間隔]` | SPRTの判定を待つ | 0=判定、1=中断、2=引数エラー |
| `scripts/watch-ci.sh <PR番号> [間隔]` | CIの確定を待つ | 0=pass、1=fail、2=引数エラー、3=時間切れ |

どちらも読むだけで、状態を変えない。`watch-ci.sh` はマージしない。

### 許可リスト

`.claude/settings.json` をリポジトリへ置き、開発フローで繰り返す
読み取り専用の操作を許可する。

```json
{
  "permissions": {
    "allow": [
      "Bash(./scripts/watch-ci.sh:*)",
      "Bash(./scripts/watch-sprt.sh:*)",
      "Bash(awk *)",
      "Bash(cargo bench:*)",
      "Bash(cargo build:*)",
      "Bash(cargo clippy:*)",
      "Bash(cargo fmt:*)",
      "Bash(cargo run --release -p himawari-tools --bin verify:*)",
      "Bash(cargo test:*)"
    ]
  }
}
```

過去50セッションの記録を集計して選んだ。`grep`・`head`・`git status`・
`gh pr view` などは Claude Code が既に自動で許可するため書かない。

**任意コード実行になる規則は書かない。** 最頻だった `python3`（384回）は
入れない。`git commit`・`git push`・`git checkout` などの状態を変える
操作も入れない。オーナーの確認が要る操作は確認を通す。

`.claude/settings.local.json`（個人設定）は `.gitignore` へ入れ、共有する
のは `.claude/settings.json` だけにする。

### 運用上の注意: 待機と判断を分ける

`watch-ci.sh` をマージと `&&` で繋ぐと、**複合コマンド全体が破壊的操作
として判定され**、確認を求められる。起草した当日の運用で踏んだ
（2026-07-29）。

```bash
# 誤り: watch-ci.sh が読み取り専用でも、全体で承認が要る
./scripts/watch-ci.sh 78 && gh pr merge 78 --squash --delete-branch

# 正しい: 待機と判断を分ける
./scripts/watch-ci.sh 78      # 承認不要。CIの確定まで待つ
gh pr merge 78 --squash       # 承認を通る。mainへ入れる判断
```

スクリプトを読み取り専用に保っても、呼び出し側で破壊的操作と結合すれば
意味がない。**Monitorへ渡すのは待機だけにする。**

この分離は制約ではなく設計である。待機は自動で進み、mainへ取り込む判断は
オーナーの手元に残る。

## Consequences

SPRTとCIの待機で止まらなくなる。判定が出るまで数時間かかる変更でも、
オーナーが席を外している間に進む。

許可した範囲は読み取りとビルドに限る。実装の変更・コミット・push・
マージは引き続き確認を通る。「24時間動く」のは測定と待機の部分で、
mainへ入れる判断はオーナーの手元に残る。

`scripts/sprt.sh` そのものは許可リストへ入れていない。対局を回して
`data/sprt/` へ書き込むためである。SPRTの開始は確認を通す。開始さえ
済めば、以降の待機と記録は止まらない。ここを許可するかは、運用してから
判断する。
