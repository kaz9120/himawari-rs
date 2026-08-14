# 0161: docsとchoreをCHANGELOGから外し、バージョンを動かさない

- Status: accepted
- Date: 2026-08-14
- 関連ADR: [0068](0068-sprt-driven-versioning.md), [0071](0071-release-please.md)

## Context

[ADR-0071](0071-release-please.md)は型とbumpの対応を次のように定めた。
`feat` はMINOR、`fix` はPATCH、`docs` と `chore` はbumpなしである。
「バージョンはビルド成果物の識別子であり、同じバイナリに別の番号を
与える理由がない」がその根拠だった。

実態は違う。文書だけを直してもPATCHが上がっている。直近30リリースの
うち20件が `docs` または `chore` のみで、バイナリは前版と同一である。
0.35.14から0.35.19までの6リリースは5件が文書のみだった。

原因は `release-please-config.json` の `changelog-sections` にある。
ADR-0071は「bumpしない型も履歴として残す」つもりで `docs` と `chore` に
`hidden: false` を指定したが、release-pleaseではこの2つが両立しない。

release-pleaseの既定のversioning strategyは、breaking changeがあれば
MAJOR、`feat` があればMINOR、それ以外は型を問わずPATCHへフォールバック
する（`src/versioning-strategies/default.ts`）。

```typescript
return new PatchVersionUpdate();
```

リリースPRを作るかどうかは、生成したリリースノートが空かどうかで決まる
（`src/strategies/base.ts`）。

```typescript
if (!bumpOnlyOptions && this.changelogEmpty(releaseNotesBody)) {
  this.logger.info(`No user facing commits found since ... - skipping`);
  return undefined;
}
```

つまり `hidden: false` はCHANGELOGへ載せる指定であると同時に、
「リリースを起こす」指定でもある。ADR-0071とワークフローのコメントが
実態と食い違ったまま17日間動いていた。

導入以降のコミットは306件で、内訳は `chore` 173・`docs` 58・`fix` 48・
`feat` 27である。4件に3件がバイナリを変えない変更で、そのすべてが
バージョンを進めていた。

## 選択肢と比較

### 案A: `docs` と `chore` を `hidden: true` にする

ADR-0071が当初決めた対応表へ実態を合わせる。設定の2行を変えるだけで
済む。直近30リリースに当てはめると10件まで減る。

CHANGELOGから文書変更と内部変更の行が消える。

### 案B: 案Aに加えて、PATCHを日次にまとめる

`push` 契機のauto-mergeをMINOR以上に限定し、日次の `schedule` で
PATCHのリリースPRをまとめてマージする。`fix` が続いた日も1リリースに
収まる。

ワークフローに分岐と定期実行が増える。既存の
`prs_created` 依存も、cron実行時に既存のリリースPRを取りこぼすため
`autorelease: pending` ラベルの検索へ書き換えが要る。

### 案C: 現状維持

追加の作業がない。リリース一覧とバイナリの対応は崩れたままになる。

## Decision

案Aを採る。

決め手は、案Aだけでリリースが3分の1へ減ることである。案Bが追加で削るのは
`fix` が同日に複数入った場合だけで、直近30リリースでは9件が6件程度に
なるにすぎない。得られる差に対してワークフローの分岐と定期実行は重い。
`docs` と `chore` を外してもなおPATCHが多いと感じたら、そのとき案Bへ進む。

CHANGELOGから消える情報は、他の文書が持っている。効かなかった案の記録は
ADRの `rejected` 行が担い、索引が「これは試したか」の入口を用意している
（[docs/adr/README.md](README.md)）。個々のコミットはリリース間の
compareリンクから辿れる。CHANGELOGに再掲する読み手がいない。

### 挙動の確認をStatusの条件にする

`hidden: true` の型しかない期間でリリースPRが作られないことは、
まだ実測していない。上に引用した `changelogEmpty` はリリースノートの
本文が1行以下かで判定するが、本文にバージョンの見出し行が含まれるかを
ソースから確定できなかった。release-pleaseの公式ドキュメントにも
`hidden` の記述がない。

本ADRを入れるPRは `chore` と `docs` だけで構成されるので、そのマージで
そのまま確認できる。リリースPRが作られなければacceptedにする。
作られてしまったら案Bへ移り、日次のcronで頻度を抑える。

確認できた。#314のマージ後に走ったrelease-pleaseはリリースPRを作らず、
後続のCargo.lock同期とauto-mergeもスキップした。

```
✔ Building candidate release pull request for path: .
✔ No user facing commits found since 260db242 - skipping
```

`hidden: true` はCHANGELOGから外すだけでなく、その型しかない期間の
リリースを止める。ログの文言（user facing）が示すとおり、release-please
にとって「CHANGELOGへ載る変更」と「リリースする理由のある変更」は同義で
ある。案Bは要らない。

## Consequences

- リリースとバイナリが一対一で対応する。タグを打った版はすべて前版と
  中身が違う。ADR-0068とADR-0071が意図した状態になる
- リリースが直近の実績で3分の1へ減る。タグpushで走る5プラットフォーム
  ビルドもその分減る
- CHANGELOGは「棋力向上」と「その他の変更」の2節だけになる。棋力の
  履歴を追う用途では読みやすくなる
- 文書変更の履歴はCHANGELOGから消える。判断の経緯はADR、変更の一覧は
  git logとcompareリンクが持つ
- 文書のみのPRが続くあいだ、`Cargo.toml` のversionはmainのHEADより
  古いままになる。devビルドの識別子は `HIMAWARI_BUILD_ID` なので
  （[ADR-0068](0068-sprt-driven-versioning.md)）、版の特定には困らない
- 「1採択=1MINOR」は変わらない。`feat` は従来どおり即座にリリースされる
