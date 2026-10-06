# Claude Code Routinesの手順

クラウドで定期実行するエージェントの手順を置く（[ADR-0209](../../docs/adr/0209-workflow-layers.md)）。
Routineのプロンプトは「このディレクトリの手順を実行する」の1行にする。手順の
中身をここへ置くのは、PRでレビューでき、履歴が残るためである。

| 手順 | 周期（日本時間） | やること | Routine |
|---|---|---|---|
| [micro-digest.md](micro-digest.md) | 毎日 03:00 | `micro` と `cloud` の付いたIssueを古い順に1件消化し、PRを出す | [trig_01PDkdQ3tFKWi2Ak3uyKCjJt](https://claude.ai/code/routines/trig_01PDkdQ3tFKWi2Ak3uyKCjJt) |
| [patrol.md](patrol.md) | 月曜 04:00 | 固定の観点でコードと文書を読み、`micro` Issueを切る | [trig_01L4faVCfNjCjsYHE68MQtzL](https://claude.ai/code/routines/trig_01L4faVCfNjCjsYHE68MQtzL) |
| [stockfish-watch.md](stockfish-watch.md) | 木曜 04:00 | Stockfishの新着PRを読み、本エンジンで成り立つ理由を書けるものをIssueにする | [trig_017okz4pZg7NbTvhnf8u8cJi](https://claude.ai/code/routines/trig_017okz4pZg7NbTvhnf8u8cJi) |

実装を伴う消化と、根拠を書く調査はOpus、手順の決まった巡回はSonnetで回している。
止めるときは、Routineのページで無効にする。消すのもそのページからで、APIからは
消せない。実行の記録は同じページにあり、動きがおかしいときはそこから辿る。

## 共通の約束

クラウドの環境にはリポジトリのcloneしかない。`data/`（教師・ネット・棋譜）と
開発機の計算資源には触れない。次のことはしない。

- 棋力が変わる変更。SPRTを回せないので扱わない。探索・評価関数・時間管理の
  挙動を変える案は、Issueに書いて対話セッションへ渡す
- 計測が要る変更（NPS、学習、対局）。`local` ラベルのIssueにする
- `Cargo.toml`・`.claude/settings.json`・`.github/workflows/` の変更
- mainへの直接push、force push

## クラウドの環境

2026-09-18の試験実行で確かめた。`gh` の節は2026-10-04に測り直し、
他のリポジトリを読む経路は2026-10-06に測った。

- **`gh` は入っているが、GraphQLは403で塞がれている**。`gh pr list`・`gh pr checks`・
  `gh pr create` はこの経路を使うので通らない。`gh auth status` はトークンを無効と
  報告するものの、認証は経路側が足すため `gh api` の読み取りは通る
- 読み取りのRESTは `gh api` で通る。CIの確定待ちは
  `gh api repos/{owner}/{repo}/commits/{sha}/check-runs` のポーリングで書ける。
  通るのは `repos/{owner}/{repo}/...` のリポジトリ単位の経路だけで、`search/issues`
  のような横断の経路は403になる。書き込みは経路で止まるものがあり、2026-10-03の
  実行では `DELETE git/refs` が403を返した
- Issueの一覧・作成・コメント、PRの作成・マージは、セッションに付いているGitHubの
  ツールで行う。手順の中の `gh issue list --json` や `gh pr ...` の例は、
  GraphQLを使うので `gh api` かツールへ読み替える
- **ツールもセッションの対象リポジトリに縛られる**。他のリポジトリは `gh api`・
  `curl`・ツールのどれでも403になり、`add_repo` の `access:"push"` も拒否される。
  公開されているリポジトリなら、WebFetchでHTMLを読むのが唯一の経路になる
  （[stockfish-watch.md](stockfish-watch.md)の手順1が例）
- `search_issues` のツールは自然言語の意味検索である。GitHubの検索の構文は解釈
  されず、本文の語で引く用途には使えない。ラベルや状態で絞るなら、
  `gh api 'repos/{owner}/{repo}/issues?...'` を `--jq` で加工する
- `hmwr pr create` と `hmwr ci wait` は使えない。中で `gh pr create` と
  `gh pr checks` を呼ぶためである。`ci wait` は403の出力を「まだ動いている」と読み、
  1時間待ってから落ちる。PRの本文は `./bin/hmwr pr template chore` のひな形から作り、
  見出しを全部残す
- pytestとnumpyは入っていない。`pip install --quiet pytest numpy` で入れる
  （CIと同じ集合）。numpyを欠くと収集の時点で止まり、テストが1件も走らない
- Rustのビルドは約40秒、`npm ci` と文書のlintは約45秒で通る

Routineを作るときは、MCPのコネクタを付けない（作成の後に
`clear_mcp_connections` で外す）。既定ではアカウントのコネクタが全部付く。
外部のPRやIssueの本文を読むエージェントに、メールやチャットの権限を持たせない。

ブランチは `claude/` で始まる名前になる（[ADR-0070](../../docs/adr/0070-pr-based-workflow.md)の
命名規則の例外）。PRの種別は「その他」にする。コミットの型は
`fix`・`docs`・`chore` のどれかで、`feat` は使わない。

push前に、CIと同じ検査をローカルで通す。

```
cargo fmt --all --check && cargo clippy --all-targets -- -D warnings
cargo test --release
pip install --quiet pytest numpy && python3 -m pytest tests -q
npm ci && ./bin/hmwr doc lint && ./bin/hmwr doc check
```

通らない検査があり、直し方が分からなければ、PRを出さずにIssueへ状況を
コメントして終わる。**緑にならないPRを置いていかない**。

CIが緑になったら、自分でsquashマージする（2026-09-17オーナー判断）。PRの
チェックが全部passになるのをGitHubのツールで確かめてから、squashでマージし、
ブランチを消す。チェックが終わる前にマージしない。
