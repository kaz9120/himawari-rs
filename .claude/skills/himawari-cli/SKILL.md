---
name: himawari-cli
description: himawari-rsの開発コマンドhmwrを使う。ビルド・機能検証・NPS計測・学習・教師データの前処理・文書のlintを実行するとき必ず使う。SPRTの運用手順はrunning-sprtスキルが持つ。
---

# hmwr: 開発コマンド

`hmwr` が開発操作の入口である（[ADR-0180](../../../docs/adr/0180-hmwr-cli-in-python.md)）。
ビルドも測定も学習もここから行う。**個別のスクリプトを直接叩く前に
`hmwr --help` を見る。**

```
hmwr --help              全体
hmwr <領域>              その領域のヘルプ
hmwr env                 並列度・評価関数・持ち時間の既定を表示する
hmwr --dry-run <...>     走るはずのコマンドを表示して終わる
```

パスが通っていなければ `./bin/hmwr` で呼ぶ。通し方は `scripts/setup.sh` が
案内する（`export PATH="<リポジトリ>/bin:$PATH"`）。

## 覚えることは3つ

1. **オプションはフラグで渡す**。環境変数を組み立てない
2. **ログの置き場は書かない**。`data/logs/<領域>-<名前>.log` へ決まる
3. **終了コードは4つ**。0=成功、1=判定結果、2=引数エラー、3=実行時エラー。
   `verify` の1は「全局面で一致した」を意味する

`--dry-run` はすべてのコマンドで効く。**時間のかかる操作や外から見える操作は、
先に下見する。**

## 領域ごとの使い方

### 棋力を測る

```
hmwr sprt run <名前>                  ペア作成→機能検証→起動
hmwr sprt run <名前> --noninferiority 非劣性で測る
hmwr sprt net <名前>                  ビルドを固定し、評価関数だけ差し替える
hmwr sprt show <名前>                 途中経過・結果
hmwr sprt show                        新しい順に10件
hmwr sprt wait <名前>                 判定が出るまで待つ
```

**SPRTの運用（条件の意味、判定の読み方、H1/H0後の後処理）はrunning-sprt
スキルを正とする。** ここには書かない。

### 挙動が変わったかを確かめる

```
hmwr verify <名前>                    base-<名前> と cand-<名前> を比べる
hmwr verify <base> <cand> --depth 15  バイナリを直接指定する
```

**全局面でノード数が一致したら終了コード1になる**。その変更は探索に影響して
おらず、対局にかけても中立にしかならない（ADR-0074）。ただし4局面は序盤に
偏っているので、終盤にしか出ない機能は別に測る。

### 速度を測る

```
hmwr bench <base> <cand> --log adr0180     交互に測る
hmwr bench <bin> --nodes 5000000           ノード数で打ち切る
```

**2本以上を並べて交互に測る**。1本ずつ別々に測った値を比べない。機体の温度や
背景の負荷でNPSは数%動く。評価関数をまたいで比べるときは `--nodes` を使う
（ADR-0127）。同じ深さでも探索木の大きさが変わるためである。

### ビルドする

```
hmwr build pair <名前>              比較用の2本を同条件で作る
hmwr build pair <名前> --baseline v0.12.0
hmwr build pgo                      配布・対局用の単体ビルド
hmwr build engine --arch 512x16x64  構成を変えてビルドする
hmwr build shapes 256x16 512x16x32  構成ごとにエンジンと評価ファイルを対で作る
```

**比較用のペアにPGOを使わない**。両側を同条件（PGOなし）で作るほうが公平で、
`build pair` の既定手順がそのまま使える。

### 評価関数を扱う

```
hmwr net train <名前> --data data/train/train_2990M_q1.psv \
                      --valid data/train/valid_385M_q1.psv
hmwr net train <名前> --data d.psv --init-ckpt <ckpt> --lr 1e-4
hmwr net shapes 256x16 512x16x32 --seed 1   構成ごとに小さく学習して比べる
hmwr net eval data/nets/*.hmwr.best
hmwr net release data/nets/x.hmwr.best 5 --apply
```

**検証データは学習データと同じ土俵に揃える**（ADR-0136）。静止化した教師で
学習するなら検証集合も静止化する。土俵がずれると最良チェックポイントの選択が
歪む。

**検証損失を足切りに使わない**（ADR-0158）。初期値の系列が違うだけで0.00136
動く。**採否は対局で決める。**

配布は既定で予行演習になる。`--apply` を付けたときだけ作る。

`--extra` にハイフンで始まる値を渡すときは `--extra=--mirror-factor` のように
= でつなぐ。

### 教師データを扱う

```
hmwr data fetch all               取得→検査→psv作成
hmwr data quiet <入力> <出力>     静止局面へ置き換える
```

`data quiet` は29.9億で7.0時間、3億で50分かかる。停止ファイルを持たないので、
途中で止めたら最初からやり直す。`--limit` で先頭だけ試せる。

### 文書を書いたら

```
hmwr doc lint          CIと同じ検査を回す
hmwr doc lint --fix    自動で直せるものを直す
```

PRを出す前に通す。CIが落ちてから直すより速い（ADR-0178）。

### 実戦を観測する

```
hmwr kifu cycle                回収→分析→定跡追加→網羅率
hmwr kifu fetch                棋譜だけ回収する
hmwr book seed --max-positions 100
hmwr book stats
hmwr book release <DB> <番号> --apply
```

定跡追加は1局面あたり深さ28で約34秒かかる。1回の追加数を絞り、残りは次回が
続きから足す（冪等なので何度回しても増えない）。

### 速度の内訳とリーグ戦

```
hmwr profile record <バイナリ>       プロファイルを取る
hmwr profile report <プロファイル>   self時間の上位を出す
hmwr league run <バイナリ>...        総当たり戦を回す
hmwr league summary <棋譜>           相対Eloを集計する
```

### CIを待つ

```
hmwr ci wait <PR番号>
```

読むだけの操作なので、マージはしない。

## 名前の付け方

実験名は `adrNNNN-<slug>`（例: `adr0180-cli`）を使う。ネットの名前は構成を
含める（例: `pairprod_2990M_q1`）。CLIが検証するので、空白やパス区切りを
含む名前は実行前に落ちる。

この名前から置き場が決まる。

| 置き場 | パス |
|---|---|
| ログ | `data/logs/<領域>-<名前>.log` |
| 棋譜 | `data/sprt/<名前>.jsonl` |
| 完了の印 | `data/sprt/<名前>.result` |
| バイナリ | `data/bin/base-<名前>` / `data/bin/cand-<名前>` |
| ネット | `data/nets/<名前>.hmwr` |
