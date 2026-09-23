# 0220: 開発環境を再設計する。状態の契約・ポータル・コマンドの棚卸し・遠隔制御

- Status: accepted（2026-09-22、オーナーが4点を決めた。下の「決定」）
- Date: 2026-09-22
- 関連ADR: [0053](0053-docs-structure.md), [0149](0149-experiment-runner.md), [0175](0175-sprt-until-decision.md), [0180](0180-hmwr-cli-in-python.md), [0181](0181-agent-surface.md), [0208](0208-hmwr-resource-verbs.md), [0209](0209-workflow-layers.md), [0217](0217-sprt-pass-cap.md), [0219](0219-relabel-2b-in-place.md)

## Context

[ADR-0219](0219-relabel-2b-in-place.md)の付け直しが約10日GPUを占有し、その間は
学習の実験を走らせない。オーナーは、この期間を開発環境の再設計に充てる
と決めた（2026-09-22）。必要ならリファクタリングもコマンドの作り直しも
辞さない。

再設計の目的は、これからの開発が加速することである。この2週間で見えた
摩擦を挙げる。

- **今何が走っているかを1か所で見られない**。`queue status`・`exp show`・
  ログのtailの3か所を見て組み立てている。人が見ても同じ手間になる
- **ログの行の形式がコマンドごとに違う**。進み具合を機械で読むには、
  コマンドごとに解釈が要る
- **走行中の対局を止める動詞がない**。ADR-0211の打ち切りはSIGTERMで行い、
  キューは「失敗」と書き戻した（Issue #581）
- **実験の制御がIssueのラベルの手作業に寄っている**。積む・外す・並べ替える
  は `gh issue edit` で行う
- **`sprt run` と `match run` が二重になっている**。ネットの指定も、名前と
  パスが混在する（`sprt net` はパス、`match run` は名前）
- **オーナーが外から状況を見る手段がない**

pr_inbox（`~/Hobby/pr_inbox`）に流用できる型がある。Bunのserver＋Reactのwebを
launchdで常駐させ、Cloudflare TunnelとAccessで外から使う運用手順まで揃っている。
ポータルはこのリポジトリのモノレポの中に置く（オーナー指示）。この
プロジェクトのことはすべてここに閉じる。

## 決めること

5つに分ける。1と2が土台で、3〜5はその上に乗る。

### 1. 状態の契約

**状態は `hmwr` が1つのJSONで出し、他はそれを読むだけにする**。

長く走るコマンド（`data relabel`、`net train`、`match run`、`exp run`、`spsa run`）は、
心拍を `data/status/<領域>-<名前>.json` へ書く。60秒ごとと、開始・終了時に
書く。ログ（`data/logs/`）は人が読む用のまま残し、機械は心拍だけを読む。

```json
{
  "kind": "relabel",
  "name": "train_7860M_q1",
  "state": "running",
  "progress": {"done": 62500864, "total": 2000000000, "unit": "局面"},
  "rate": 2397.0,
  "eta_seconds": 19800,
  "started": "2026-09-22T13:00:00+0900",
  "updated": "2026-09-22T18:12:19+0900",
  "pid": 12345,
  "log": "data/logs/relabel-train_7860M_q1.log",
  "detail": {"scale": 430, "device": "coreml"}
}
```

`state` は `running`・`done`・`failed`・`stopped` の4つ。`detail` は領域ごとの
追加情報で、対局ならElo・LLR・ペア数、学習ならstepとvalid、実験なら
ステップの一覧と現在位置を持つ。

`hmwr status --json` はこれらを集めて1つにする。キュー（GitHubのIssueと
ラベル）、走行中の心拍、直近の結果（`.result`、実験台帳）、資源（ディスクの
空き、launchdの生死、GPUの使用）、開いているPR。`hmwr status`（JSONなし）は
同じ内容を表で出す。ポータルはこのJSONだけを読む。

### 2. リポジトリの構成

モノレポにする。今の構成は活かし、ポータルを足す。

```
crates/        Rust（エンジン・道具）
hmwr/          Python CLI（状態の契約の持ち主）
training/      学習器
experiments/   実験のspec
portal/        Bunのserver＋Reactのweb＋shared（pr_inboxの型）
docs/          ADR・ROADMAP・DATASETS
data/          成果物（gitignore）。data/status/ を足す
```

ルートの `package.json` はtextlint用なので、workspaceの親にして
`portal/` を子にする。CIにportalのlint・型検査・テストを足す。
[ADR-0053](0053-docs-structure.md)の文書の役割分担に `portal/README.md` を
足す。

### 3. コマンドの棚卸し

[ADR-0208](0208-hmwr-resource-verbs.md)の「資源への操作」を通し切る。

| 今 | 後 | 理由 |
|---|---|---|
| `sprt run` / `sprt net` / `match run` | `match run` に統一。`--stop sprt` が既定 | 二重をなくす。`sprt` は `match` の別名として1版残す |
| `sprt net` がパス、`match run` が名前 | すべて名前（`data/nets/<名前>.hmwr`） | 名前の規則に揃える |
| 停止の動詞がない | `match stop <名前>`、`exp stop <名前>` | 停止ファイルをペアの切れ目で見る。見送りとして結果に残す（#581） |
| `queue` の積む・外す・並べ替えが手作業 | `queue add <spec>`、`queue drop <番号>`、`queue move <番号> --before <番号>` | Issueの作成とラベルを包む。正はGitHubのまま |
| 進み具合はログだけ | 心拍（上の1） | 機械が読める |
| `hmwr env` | `hmwr status` に吸収 | 見る場所を1つにする |

作り直しは、契約（1）に合わせて動詞を切り直す形で行う。既存のspecと
スキルが壊れないよう、別名を1版残してから消す。

### 4. ポータル

pr_inboxの骨組みを流用する。serverはBunで `127.0.0.1` だけを待ち、30秒ごとに
`hmwr status --json` を読んでwebへWebSocketで流す。webはモバイルファースト。

画面は4つ。

| 画面 | 中身 |
|---|---|
| 今 | 走行中の実験とステップ、進み具合と残り時間、対局のEloとLLR、学習のstepとvalid |
| キュー | 待ちの実験の並び。一時停止・再開・積む・外す・並べ替え・停止 |
| 結果 | 直近の結果（対局・学習）と、記録PRへのリンク |
| 資源 | ディスクの空き、launchdの生死、GPUの使用、開いているPR |

制御はserverが `hmwr` の動詞を呼ぶ。外からの入り口はCloudflare Tunnelだけにする。
手前にCloudflare Accessを置き、serverもJWTを検証する。pr_inboxの
`docs/operations.md` と同じ守りである。常駐はlaunchd。

### 5. 記録の無人化の続き

結果の記録は `claude -p` の経路で3件とも正しく出た（ADR-0212・0214・0215）。
この経路は残し、足りない2つを足す。記録PRがマージされたら実験のIssueを
閉じること（Issue #506）と、見送り・H0の記録である。

## 進め方

10日で、土台から順に入れる。各段はPRで区切り、動くものを積む。

| 段 | 内容 | 目安 |
|---|---|---|
| 1 | 心拍の形式と `hmwr status --json`。まずCLIだけで動かす | 2日 |
| 2 | 長く走るコマンド4つに心拍を足す。ADR-0219の走行が最初の実物 | 1日 |
| 3 | `portal/` を作り、読むだけの画面（今・結果・資源）を出す | 3日 |
| 4 | launchdとTunnel・Accessで外から見る | 半日 |
| 5 | 停止・積む・外す・並べ替えの動詞と、キューの画面 | 2日 |
| 6 | コマンドの統一（`match` へ）と別名、スキルと文書の追従 | 1日 |

3以降はUIの情報設計（`portal/docs/ui-design.md`）を先に書き、オーナーに見て
もらってから作る。

## 決定（2026-09-22、オーナー）

1. ポータルの認証はCloudflare Accessだけにする。LANからAccessなしの入り口は
   作らない
2. `sprt` を `match` へ統一する。スキル（running-sprt）と過去のspecの
   `sprt run` は別名で1版残す
3. `hmwr env` を `hmwr status` に吸収する
4. 画面の情報設計は、動くものを見てから直す。実装前の確認は要らない

## 経過

| 段 | PR | 内容 |
|---|---|---|
| 1 | #602 | 心拍の形式、`hmwr status`、付け直しの心拍 |
| 2 | 本PR | 学習・対局・実験・SPSAの心拍。`hmwr clean` が30日で消す |

2段目の配線は、子プロセスの出力の行を読んで心拍を動かす形にした。学習と
対局は進み具合を標準出力にしか出さないので、`proc.run` に行ごとの口を
足した。子プロセスの側は変えていない。実験のステップは所要が数分から
数日まで揃わないので、残り時間を見積もらない。SPRTもいつ判定に至るか
分からないので、総量を持たない（指し切りのときだけ持つ）。

## Consequences

- `data/status/` が増え、`hmwr clean` の対象になる。心拍は終了後も残し、
  30日で消す
- ポータルのためにBunとNodeの依存がリポジトリに入る。CIの所要が伸びる
- コマンドの統一で、スキルとADRの手順の記述を追従させる。別名を残す間は
  二重に見える
- 見直しのトリガーは、心拍の形式で表せない状態が出たときである。そのときは
  `detail` を広げず、形式を改版する
