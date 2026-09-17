# Claude Code Routinesの手順

クラウドで定期実行するエージェントの手順を置く（[ADR-0209](../../docs/adr/0209-workflow-layers.md)）。
Routineのプロンプトは「このディレクトリの手順を実行する」の1行にする。手順の
中身をここへ置くのは、PRでレビューでき、履歴が残るためである。

| 手順 | 周期 | やること |
|---|---|---|
| [micro-digest.md](micro-digest.md) | 毎晩 | `micro` と `cloud` の付いたIssueを古い順に1件消化し、PRを出す |
| [patrol.md](patrol.md) | 週次 | 固定の観点でコードと文書を読み、`micro` Issueを切る |
| [stockfish-watch.md](stockfish-watch.md) | 週次 | Stockfishの新着PRを読み、本エンジンで成り立つ理由を書けるものをIssueにする |

## 共通の約束

クラウドの環境にはリポジトリのcloneしかない。`data/`（教師・ネット・棋譜）と
開発機の計算資源には触れない。次のことはしない。

- 棋力が変わる変更。SPRTを回せないので扱わない。探索・評価関数・時間管理の
  挙動を変える案は、Issueに書いて対話セッションへ渡す
- 計測が要る変更（NPS、学習、対局）。`local` ラベルのIssueにする
- `Cargo.toml`・`.claude/settings.json`・`.github/workflows/` の変更
- mainへの直接push、force push

ブランチは `claude/` で始まる名前になる（[ADR-0070](../../docs/adr/0070-pr-based-workflow.md)の
命名規則の例外）。PRは `hmwr pr create --kind chore` で作る。コミットの型は
`fix`・`docs`・`chore` のどれかで、`feat` は使わない。

push前に、CIと同じ検査をローカルで通す。

```
cargo fmt --all --check && cargo clippy --all-targets -- -D warnings
cargo test --release
python3 -m pytest tests -q
./bin/hmwr doc lint && ./bin/hmwr doc check
```

通らない検査があり、直し方が分からなければ、PRを出さずにIssueへ状況を
コメントして終わる。**緑にならないPRを置いていかない**。

CIが緑になったら、自分でsquashマージする（2026-09-17オーナー判断）。

```
./bin/hmwr ci wait <PR番号> && gh pr merge <PR番号> --squash --delete-branch
```
