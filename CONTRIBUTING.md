# 貢献の手引き

himawari-rsはRustで書いた将棋エンジンである。報告と提案を歓迎する。

## 報告する

[Issues](../../issues/new/choose)から、フォームを選んで書く。

| フォーム | 使うとき |
|---|---|
| 不具合の報告 | 導入や対局で起きた問題。文字化け、落ちる、時間切れなど |
| 提案・要望 | 機能の追加や変更の提案 |

エラー文やログは画像ではなくテキストで貼る。全部埋まっていなくてもよい。

## 変更を送る

変更はすべてPull Request経由で入る。PRテンプレートで種別を選ぶ。

- 棋力に関わる変更（探索・評価関数・時間管理）は、[ADR](docs/adr/README.md)を
  起こしてSPRT（自己対局の逐次検定）でH1採択されたものだけを取り込む。
  基準は[ADR-0028](docs/adr/0028-pruning-extensions.md)にある
- それ以外（文書、ツール、CI）はCIが緑なら取り込む

開発の流儀は[CLAUDE.md](CLAUDE.md)と[docs/adr](docs/adr/README.md)にある。
コミットの件名はConventional Commits（`feat:` `fix:` `docs:` `chore:`）で、
本文は日本語でよい。
