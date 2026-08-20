---
name: himawari-cli
description: himawari-rsの日常操作をhmwrコマンドで行う。ビルド・機能検証・NPS計測・学習・教師データの前処理・実戦棋譜の回収・配布・文書のlintを実行するとき必ず使う。SPRTの運用手順はrunning-sprtスキルが持つ。
---

# hmwr: 日常操作の入口

`scripts/hmwr` が日常操作の入口である（ADR-0179）。ビルドも計測も学習も、
まずここを見る。**個別のスクリプトを直接叩く前に `hmwr --help` を見る。**

```
scripts/hmwr --help              全体
scripts/hmwr <領域> --help       その領域の操作
scripts/hmwr --dry-run <...>     走るはずのコマンドを表示して終わる
```

`--dry-run` はすべてのコマンドで効く。**時間のかかる操作や外から見える操作は、
先に下見する。**

## 覚えることは3つ

1. **オプションはフラグで渡す**。環境変数を組み立てない。
   `hmwr train x --data d.psv --lr 1e-4` と書けばCLIが `TRAIN_PEAK_LR` へ畳む
2. **ログの置き場は書かない**。CLIが `data/logs/<領域>-<名前>.log` へ決める
3. **終了コードは4つ**（ADR-0122）。0=成功、1=判定結果、2=引数エラー、
   3=実行時エラー。`verify` の1は「全局面で一致した」を意味する

## 領域ごとの使い方

### 棋力を測る

```
scripts/hmwr sprt start <名前>          ペア作成→機能検証→切り離し起動
scripts/hmwr sprt status <名前>         途中経過・結果
scripts/hmwr sprt status                新しい順に10件
```

**SPRTの運用（条件の意味、判定の読み方、H1/H0後の後処理）はrunning-sprt
スキルを正とする。** ここには書かない。

### 挙動が変わったかを確かめる

```
scripts/hmwr verify <名前>                    base-<名前> と cand-<名前> を比べる
scripts/hmwr verify <base> <cand> --depth 15  バイナリを直接指定する
```

**全局面でノード数が一致したら終了コード1になる**（ADR-0074）。その変更は
探索に影響しておらず、SPRTにかけても中立にしかならない。ただし4局面は序盤に
偏っているので、終盤にしか出ない機能は別に測る。

### 速度を測る

```
scripts/hmwr bench <base> <cand> --log adr0179     交互に測る
scripts/hmwr bench <bin> --nodes 5000000           ノード数で打ち切る
```

**2本以上を並べて交互に測る**。1本ずつ別々に測った値を比べない。機体の温度や
背景の負荷でNPSは数%動く。`--log <名前>` を付けると
`data/logs/bench-<名前>.log` へ残る。

評価関数をまたいで比べるときは `--nodes` を使う（ADR-0127）。同じ深さでも
探索木の大きさが変わるためである。

### ビルドする

```
scripts/hmwr build pair <名前>              SPRT用の2本を同条件で作る
scripts/hmwr build pair <名前> --baseline v0.12.0
scripts/hmwr build pgo                      配布・対局用の単体ビルド
```

**SPRTのペアにPGOを使わない**。両側を同条件（PGOなし）で作るほうが公平で、
`build pair` の既定手順がそのまま使える。

### 学習する

```
scripts/hmwr train <名前> --data data/train/train_2990M_q1.psv \
                         --valid data/train/valid_385M_q1.psv
scripts/hmwr train <名前> --data d.psv --init-ckpt <ckpt> --lr 1e-4
scripts/hmwr eval data/nets/*.hmwr.best
```

**検証データは学習データと同じ土俵に揃える**（ADR-0136）。静止化した教師で
学習するなら検証集合も静止化する。土俵がずれるとbest checkpointの選択が歪む。

**valid lossを足切りに使わない**（ADR-0158）。初期値の系列が違うだけで
0.00136動く。**採否は対局で決める。**

`--extra` にハイフンで始まる値を渡すときは `--extra=--mirror-factor` のように
= でつなぐ。

### 教師データを扱う

```
scripts/hmwr data fetch all               取得→検査→psv作成
scripts/hmwr quiet <入力> <出力>          qsearchの静止局面へ置き換える
```

`quiet` は29.9億で7.0時間、3億で50分かかる。停止ファイルを持たないので、
途中で止めたら最初からやり直す。`--limit` で先頭だけ試せる。

### 実戦を観測する

```
scripts/hmwr floodgate cycle              回収→分析→定跡追加→網羅率
scripts/hmwr floodgate cycle --seed-max 100
```

定跡追加は1局面あたり深さ28で約34秒かかる。1回の追加数を `--seed-max` で絞り、
残りは次回が続きから足す（冪等なので何度回しても増えない）。

### 配る

```
scripts/hmwr release net data/nets/x.hmwr.best 5           予行演習
scripts/hmwr release net data/nets/x.hmwr.best 5 --apply   実際に作る
```

**既定は予行演習である**。走るはずのコマンドとリリースノートを出して終わる。
`--apply` を付けたときだけ作る。リリースは消しても「あった」ことが残る
（ADR-0122）。

### 文書を書いたら

```
scripts/hmwr doc lint          CIと同じlintを回す
scripts/hmwr doc lint --fix    自動で直せるものを直す
```

PRを出す前に通す。CIが落ちてから直すより速い（ADR-0178）。

## CLIが覆っていない操作

次は今までどおり直接叩く。頻度が低く、環境変数が多い実験用のものである。

| 操作 | コマンド |
|---|---|
| 構成ごとのビルド | `scripts/build-shapes.sh 256x16 512x16x32` |
| 構成ごとの小規模学習 | `scripts/train-shapes.sh 256x16` |
| 総当たり戦 | `cargo run --release -p himawari-tools --bin league -- ...` |
| プロファイル | `cargo run --release -p himawari-tools --bin profile -- ...` |

**日常で使う操作がCLIの外に3つを超えて増えたら、載せる範囲を決め直す**
（ADR-0179）。

## 名前の付け方

実験名は `adrNNNN-<slug>`（例: `adr0179-cli`）を使う。ネットの名前は構成を
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
