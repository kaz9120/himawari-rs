# 0071: バージョン更新とリリースをrelease-pleaseで自動化する

- Status: accepted
- Date: 2026-07-27
- 関連ADR: [0068](0068-sprt-driven-versioning.md), [0069](0069-release-notes-automation.md)（本ADRが置き換え）, [0070](0070-pr-based-workflow.md)

## Context

[ADR-0070](0070-pr-based-workflow.md)でPRベースへ移したが、バージョン
更新は手作業のまま残した。PR内で `Cargo.toml` を書き換え、マージ後に
タグを打つ。同ADRは自動化を「第2段階」として先送りしている。

手作業には2つの弱点がある。バージョンの更新を忘れると番号が飛ぶ。
タグを打ち忘れるとリリースが作られない。どちらも気づきにくい。

[ADR-0069](0069-release-notes-automation.md)でリリースノートの生成は
自動化したが、CHANGELOGは持っていない。「0.12.0で何が入ったか」を
知るにはリリース一覧を辿ることになる。

別プロジェクト（hidoko）でrelease-pleaseが機能している。Conventional
Commitsからbumpを算出してリリースPRを作り、CI通過後の自動マージで
バージョン更新・CHANGELOG生成・タグ・GitHub Releaseまで通す構成である。

## 選択肢と比較

### 案A: release-pleaseを導入する

実績のあるツールに乗る。CHANGELOGの生成が付いてくる。Rustにも対応し
（`release-type: "rust"`）、`Cargo.toml` のバージョンを更新できる。

Conventional Commitsが前提になるため、コミットメッセージの規約を
変える必要がある。GitHub Actions用にfine-grained PATも要る。

### 案B: PRラベル駆動の独自ワークフロー

`strength` / `chore` ラベルを読み、マージ時に `Cargo.toml` を更新して
タグを打つ。ADR-0070の規約に完全に合わせられるが、実装と保守を自前で
抱える。CHANGELOGは別途作ることになる。

### 案C: 手作業を続ける

追加の仕組みが要らない。忘れやすさは残る。

## Decision

案Aを採る。

### コミットの型とbumpの対応

Conventional Commitsを採用する。使う型は4つとする。

| 型 | 対象 | bump |
|---|---|---|
| `feat` | SPRTでH1採択した変更 | MINOR |
| `fix` | 棋力に影響しないコードの変更 | PATCH |
| `docs` | 文書のみの変更 | なし |
| `chore` | CI・設定・テスト・依存更新など | なし |

判断は2つの問いで決まる。バイナリが変わるか、棋力が変わるか。
どちらも変わらなければ `docs` か `chore` で、バージョンは動かない。

[ADR-0068](0068-sprt-driven-versioning.md)は当初「PATCHに例外を設けない」
と定めた。手作業でバージョンを上げる前提では、上げるか否かの判断を
なくすことに意味があったためである。自動化するとその前提が変わる。
型を選ぶだけでbumpが決まるので、判断のコストは生じない。

そして本来、バージョンはビルド成果物の識別子である。文書だけを直しても
バイナリは同一で、別の番号を与える理由がない。ADR-0068のPATCHの定義を
「棋力に影響しないコードの変更をリリースするとき」へ改める。

型の割り当てで迷いやすいものを挙げる。

| 変更 | 型 | 理由 |
|---|---|---|
| リファクタ | `fix` | バイナリが変わる |
| 依存の更新 | `fix` | バイナリが変わる |
| テストの追加・修正 | `chore` | 配布するバイナリは変わらない |
| ADR・ROADMAPの更新 | `docs` | 同上 |
| ワークフローの修正 | `chore` | 同上 |
| 高速化（棋力は不変） | `fix` | SPRTで有意差がなければ棋力向上ではない |

CHANGELOGの見出しは `changelog-sections` で日本語に置き換える。
bumpしない型も履歴として残す。

```json
"changelog-sections": [
  { "type": "feat", "section": "棋力向上" },
  { "type": "fix", "section": "その他の変更" },
  { "type": "docs", "section": "ドキュメント", "hidden": false },
  { "type": "chore", "section": "内部", "hidden": false }
]
```

### 件名は日本語のまま、Eloを含める

型のprefixだけを足し、本文は日本語の平叙文を保つ。SPRTで得たEloは
件名に書く。CHANGELOGに数値が並び、棋力の履歴がそのまま読める。

```
feat: razoringを導入する（+184.8 Elo、ADR-0057）
fix: clippyの警告4件を解消する
```

対局数やLLRといった詳細は本文と `SPRT:` トレーラに残す
（[ADR-0069](0069-release-notes-automation.md)で決めた書式）。
トレーラはRESULTS.mdへの転記元として引き続き使う。

### MAJORはトレーラで指定する

選手権への参加でMAJORを上げる（ADR-0068）。これはコミットの内容から
判定できないため、`Release-As: 1.0.0` トレーラで明示する。

### ADR-0069の置き換え

CHANGELOGとGitHub Releaseのノートはrelease-pleaseが作る。
ADR-0069で `release.yml` に実装したノート生成は役目を終える。
`release.yml` はタグpushでCPU別バイナリをビルドし、release-pleaseが
作ったリリースへ添付する役割に絞る。

評価関数をリリースに含めない方針（ADR-0069）は変わらない。

### 必要な準備

fine-grained PAT（`RELEASE_PLEASE_TOKEN`、contents: write と
pull-requests: write）をシークレットに登録する。`GITHUB_TOKEN` が
起こしたイベントはワークフローを起動しないため、リリースPRにCIが走らず
auto-mergeの条件が満たされない。

`release-type: "rust"` がworkspaceの `[workspace.package] version` を
更新できるかは、導入時に実際のリリースPRで確かめる。更新されない場合は
`extra-files` でルートの `Cargo.toml` を対象に加える。

## Consequences

- バージョンの更新忘れとタグの打ち忘れがなくなる。番号は
  マージされたコミットの型から機械的に決まる
- CHANGELOGが手に入る。「棋力向上」セクションを追えば、採択の履歴と
  Eloが時系列で読める
- PATCHでもタグとリリースが作られる。ADR-0068は「タグはMINORとMAJOR
  のときだけ打つ」としていたが、この点を改める。リリース一覧に
  PATCHが並ぶ代わりに、棋力の履歴はCHANGELOGの「棋力向上」節が担う。
  publicリポジトリではGitHub Actionsの実行時間に制限がないため、
  リリースごとの5プラットフォームビルドも問題にならない
- 文書やCIだけの変更ではバージョンが動かない。リリースとバイナリが
  一対一で対応するようになる。ADR-0070のPRテンプレートからは
  「Cargo.tomlを更新した」というチェック項目が不要になる
- SPRT採択が2件続けてマージされた場合、リリースPRを1回だけマージすると
  MINORは1つしか上がらない。「1採択=1MINOR」を保つには、採択ごとに
  リリースPRをマージする。守れなくてもCHANGELOGには両方が残るため、
  履歴が失われるわけではない
- コミットメッセージの規約が変わる。過去のコミットは型を持たないが、
  release-pleaseは `bootstrap-sha` を起点にできるため、遡って整える
  必要はない
- release-pleaseの挙動に依存する。ツール側の仕様変更に追随する保守が
  発生する。自前実装（案B）を避けた対価である
