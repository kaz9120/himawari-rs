# ポータル

himawari-rsの今の状態を、ブラウザ（主にスマホ）から見るための画面である。
設計は[ADR-0220](../docs/adr/0220-dev-environment-redesign.md)にある。

読み手は、ポータルを起動・運用する人（オーナーとエージェント）である。
画面の中身は `hmwr status --json` がすべてで、ポータルはログやファイルを
直接読まない。表示を足したいときは、先に `hmwr status` へ足す。

## 画面

| 画面 | 中身 |
|---|---|
| 今 | 実行中の実験とステップ、各処理の進み具合と残り時間、待ち・失敗の実験 |
| 結果 | 直近の対局の判定とElo、直近の学習の検証損失 |
| 資源 | ディスクの空き、キューの常駐、開いているPR、設定 |

いまは読むだけである。一時停止・積む・外す・並べ替え・停止の操作は、
ADR-0220の5段目で足す。

## 起動

Bunが要る（開発機は1.3系）。`portal/` で次を実行する。

```
bun install
bun run build     # web/dist を作る
bun run start     # http://127.0.0.1:8790 で待ち受ける
```

serverは30秒ごとに `bin/hmwr status --json` を呼び、WebSocketで画面へ流す。
画面の「再取得」はその場で呼び直す。

開発中は2つを別々に起動する。`bun run dev:server` と `bun run dev:web` で、
webは http://localhost:5174 から開き、APIをserverへ中継する。

## 設定

環境変数で渡す。launchdのplistに書く前提で、設定ファイルは持たない。

| 変数 | 既定 | 意味 |
|---|---|---|
| `HMWR_PORTAL_PORT` | 8790 | 待ち受けのポート |
| `HMWR_PORTAL_HOST` | 127.0.0.1 | 待ち受けのアドレス。変えない |
| `HMWR_PORTAL_INTERVAL` | 30 | 状態を読む間隔（秒） |
| `HMWR_ACCESS_TEAM_DOMAIN` | なし | Cloudflare Accessのチームのドメイン |
| `HMWR_ACCESS_AUD` | なし | AccessアプリのAUDタグ |
| `HMWR_REPO` | このリポジトリ | `bin/hmwr` を探す場所 |

## 入口の守り

外からの入口はCloudflare Tunnelだけにし、手前にCloudflare Accessを置く
（ADR-0220の決定1）。serverもAccessのJWTを検証する。

- 同じMacのブラウザから `localhost` で開いたときだけ、検証を省く
- Accessの2つの変数が未設定なら、localhost以外からの要求はすべて断る（403）
- 2つのうち片方だけを書くと、起動を止める

TunnelとAccessの設定と、launchdでの常駐はADR-0220の4段目で足す。

## 検査

```
bun test           # serverの守りと状態の取得、表示の整形
bun run typecheck
```

CIも同じ2つとbuildを走らせる。
