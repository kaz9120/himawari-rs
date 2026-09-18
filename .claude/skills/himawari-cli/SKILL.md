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

対局の実体は `hmwr match run` にある。`sprt run` と `sprt net` はその別名で、
対局者の組み立て方だけが違う。SPRTの判定で止めない対局や、片側だけ条件を
変える対局は `match run` で書く。`selfplay` を直接叩かない。

```
hmwr match run <名前> --build <ビルド名> --base-net <ネット名> --cand-net <ネット名> \
  --stop pairs:1000                   固定ペア数を指し切り、Eloを推定する
hmwr match run <名前> --cand-opt Threads=8 --cand-odds 0.5   片側だけ条件を変える
hmwr match show <名前>                途中経過・結果（sprt show と同じ）
```

対局者はビルド・評価関数・USIオプション・時間オッズの組で表す。ビルドは
`data/bin/<名前>`、評価関数は `data/nets/<名前>.hmwr`、開始局面集は
`openings/<名前>.txt` に決まる。`/` を含む値はパスとして通る（リポジトリの
外にある参照エンジン用）。ビルドを省くと `build pair` の出力を使う。

最初の起動で対局の条件を `data/sprt/<名前>.cond` に控える。同じ名前で
条件の違う対局を始めると止まる。ペア数の上限だけは条件に数えないので、
`--stop pairs:500` の走行を `pairs:1000` で続きから積める。記録のない
古い棋譜を続けるときは、ログの起動行で条件を確かめてから `--adopt` を付ける。

### 探索定数をSPSAで回す（ADR-0143）

```
hmwr spsa init <名前>                 tuneビルドと対象一覧の雛形を作る
hmwr spsa run <名前> --pairs 15000    切り離して走る。判定はなくペア数で止まる
hmwr spsa show <名前>                 途中経過（θの現在値）
hmwr spsa stop <名前>                 次のバッチの前で止める。runで再開
```

対象・可動域・摂動幅は `data/spsa/<名前>.params.json` を編集して絞る。
完了後は結果の定数を `crates/engine/src/tunables.rs` へ焼き込み、
SPRT既定条件で検収する。**SPSAは1サイクル数万局を使うので、SPRTと
時期を分けて回す。**

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
hmwr build pair <名前> --candidate <SHA>   候補をrefから作る（実験のspec用）
hmwr build pgo                      配布・対局用の単体ビルド
hmwr build engine --arch 512x16x64  構成を変えてビルドする
hmwr build shapes 256x16 512x16x32  構成ごとにエンジンと評価ファイルを対で作る
```

棋力が変わる変更をキューで測るときは、候補のブランチをpushし、specの
`build pair` に**コミットのSHA**を渡す。キューの作業ツリーは常にorigin/mainで、
候補はマージ前だからである。ブランチ名ではなくSHAにするのは、事前登録の後に
候補が動かないようにするためである。specの形は次のとおり。

```toml
[[step]]
id = "pair"
run = "hmwr build pair adr0211-x --candidate 0cd8205aa6a0…"

[[step]]
id = "verify"
run = "hmwr verify adr0211-x"

[[step]]
id = "sprt"
run = "hmwr match run adr0211-x --foreground"
```

**比較用のペアにPGOを使わない**。両側を同条件（PGOなし）で作るほうが公平で、
`build pair` の既定手順がそのまま使える。

### 評価関数を扱う

```
hmwr net train <名前> --data train_300M_q1 --valid valid_385M_q1 \
                      --rank rank_300M_100M       3億の測定台のレシピ
hmwr net train <名前> --data <名前> --init-ckpt <ckpt> --lr 1e-4
hmwr net shapes 256x16 512x16x32 --seed 1   構成ごとに小さく学習して比べる
hmwr net eval data/nets/*.hmwr.best
hmwr net probe <名前> --focus focus_1M      FTを凍結して焦点の熱地図を当てる
hmwr net release data/nets/x.hmwr.best 5 --apply
```

同じ名前・同じ条件で学習済みなら、何もせず成功で終わる。途中で止まった学習は、
同じコマンドで `latest.ckpt` から再開する。同じ名前で条件が違うと止まるので、
名前を変えるか `--force` で最初から学習し直す。

**検証データは学習データと同じ土俵に揃える**（ADR-0136）。静止化した教師で
学習するなら検証集合も静止化する。土俵がずれると最良チェックポイントの選択が
歪む。

**検証損失を足切りに使わない**（ADR-0158）。初期値の系列が違うだけで0.00136
動く。**採否は対局で決める。**

`net probe` は表現の測定で、ネットを作らない。FTを凍結して熱地図を当てる
後段だけを学習し、上位5マスの的中率を検証行へ出す。**自明解と並べて読む。**
頻度事前は同じ行に並び、乱数初期値のFTは `--init-net none` で別に測る。
probeの的中率は学習の刻みで動くので、条件を揃えて比べる（ADR-0213）。

配布は既定で予行演習になる。`--apply` を付けたときだけ作る。

`--extra` にハイフンで始まる値を渡すときは `--extra=--mirror-factor` のように
= でつなぐ。

### 実験を手順ごと走らせる

```
hmwr exp check                 作業ツリーのspecを検査する（CIと同じ）
hmwr exp run <名前>            origin/mainのspecを上から順に実行する
hmwr exp show <名前>           ステップごとの進み具合
hmwr exp report <名前>         対局・学習・データの数値をMarkdownの表にする
hmwr exp report --match <対局名> --net <ネット名>   specの無い実験の表
```

specは `experiments/<名前>.toml` に置き、ADRと同じPRで入れる。ステップは
`hmwr` のコマンド1つで、シェルは通らない。対局のステップには `--foreground` を
付ける。**`exp run` が読むのはorigin/mainのspecである**。事前登録のPRを
マージしてから走らせる。

```toml
adr = "0210"

[[step]]
id = "mix"
run = "hmwr data mix mixhao20_300M --in train_300M_q1 --in hao_extra_60M_q1"
```

**結果をADRへ書くときは `exp report` の表を貼る**。ログを目で読んで数値を
書き写さない。対局は結果ファイル、学習は実験台帳、データは完了印から読むので、
転記の誤りが構造的に起きない。

止まったら同じコマンドで続きから走る。完了したステップは飛ばし、学習と対局は
それぞれの再開の口から続く。完了したステップのコマンドをspecで書き換えると
止まる。手順を変えるなら、別の名前のspecにする。

### 実験をキューで無人実行する

```
hmwr queue status      待ち行列と一時停止の状態
hmwr queue pause       次のステップを始めさせない（開発機を空けたいとき）
hmwr queue resume      一時停止を解く
hmwr queue install     専用のworktreeとlaunchdの常駐を用意する
hmwr exp reset <名前>  完了印を消し、最初から走り直せるようにする
```

実験を積むには、specとADRをmainへマージしてから、Issueフォーム「実験」で
Issueを出す。launchdが5分おきに待ち行列を見て、古い順に1件ずつ実行する。
状態はラベルで読める（`queued`→`running`→`done` か `failed`）。失敗したら
ログの末尾がIssueへコメントされる。原因を直してラベルを `queued` へ戻すと、
続きから走る。

**計測や対話セッションのSPRTの前に `hmwr queue pause` を打つ**。キューの実験と
同時に走ると、どちらの対局も持ち時間の消化が乱れる。走っているステップは
止まらないので、`hmwr queue status` で実行中の実験が無いことを確かめてから測る。

### 単発の診断

```
hmwr diag rank <重み> <群>      ランキング損失のヒンジ発火を分ける
hmwr diag dead <重み> <局面>    FT出力の対が死ぬ原因を分ける
hmwr diag phase <局面>          進行度の指標の候補を比べる
```

`diag` は1本のADRのために作った測定の置き場で、安定した表面ではない。
引数の互換は保たず、使ったADRが閉じて90日たったら消してよい。
**診断を足すときは `diag` に置く**。2本目のADRで使われたら、正規の領域への
昇格を検討する。

### 教師データを扱う

```
hmwr data fetch all                                  取得→検査→psv作成
hmwr data shuffle <出力名> --raw <データセット>      生データを全体シャッフルする
hmwr data split <出力名> --in <入力名> --count N     先頭から区間を切り出す
hmwr data mix <出力名> --in <入力名> --in <入力名>   複数の教師を混ぜる
hmwr data quiet <出力名> --in <入力名>               静止局面へ置き換える
hmwr data rank <出力名> --in <入力名> --limit N --jobs 4   兄弟局面の群を作る
hmwr data openings <出力名> --in <入力名> --min-ply 40 --count 2000   開始局面集を作る
hmwr data relabel <出力名> --in <入力名> --scale 600    DL系モデルの推論1回でscoreを付け直す
hmwr data focus <出力名> --raw <データセット> --count N   先8手の焦点の熱地図を付けた局面集を作る
hmwr data stats <名前>                               局面数・評価値の分布を見る
hmwr data rm <名前>...                               中間ファイルを消す
```

**引数は名前で渡す**。`<名前>` は `data/train/<名前>.psv` に決まる（rankの出力は
`.rankpsv`、focusの出力は `.focus`）。`psv` を直接叩かない。既定値（シャッフルのseed 1、静止化の
1手・並列8）がここに集まっているためである。足りない操作は
`hmwr/dataops.py` の対応表へ1行足す。

出力は `.part` へ書いてから改名し、`<出力>.done` に走らせたコマンドを控える。
同じ条件の再実行は何もせず成功で終わる。同じ名前で条件が違うと止まるので、
名前を変えるか `--force` で作り直す。完了印のない古い出力も同じ扱いになる。

`data rm` が消すのは完了印のある出力だけである。完了印に作り方が残っているので、
消しても同じコマンドで作り直せる。由来の記録がないファイルには触らない。
**学習が読んでいる最中のpsvを消さない**。

`data quiet` は並列8で6,000万局面に2〜12分かかる（ADR-0210・ADR-0206の実測。
取り合いの多い局面ほど遅い）。停止ファイルを持たないので、途中で止めたら最初からやり直す。`--limit` で先頭だけ試せる。並列数を変えると出力が変わる。

`data relabel` はdlshogiのONNXで勝率を取り、`cp = scale × logit(p)` をscoreへ書く。
モデルは配布元のライセンスに同意して `data/models/dlshogi/` へ置く。cshogiと
onnxruntimeが要る。MacのCoreMLで約2,600局面/秒、1億局面に約11時間かかる。
進み具合は `data/logs/relabel-<名前>.log` へ1分ごとに出る。

`data focus` は対局順のままの生データだけを読む。シャッフル済みの教師は続きの
手を持たないので使えない。出力は202バイト固定長で、元のpsvの40バイトに熱地図の
81バイトと関与フラグの81バイトが続く。読み手は
`np.fromfile(パス, dtype=np.uint8).reshape(-1, 202)` で読める。復元にcshogiが要る。

### 掃除する

```
hmwr clean            消せるものの一覧と合計を出す（下見）
hmwr clean --apply    30日を過ぎた成果物を消す（ADR-0189）
```

現行の評価関数の系列と `.result` は残る。教師データ（data/train）は
対象外で、消すなら個別判断になる。月1回を目安に回す。

### PRを出す

```
hmwr pr template chore > body.md     本文のひな形（chore か strength）
hmwr pr create --kind chore --title "chore: …" --body-file body.md
hmwr ci wait <PR番号>                CIが確定するまで待つ
```

本文にテンプレートの見出しが揃っていなければ、PRは作られない。本文のファイルは
スクラッチパッドに置く。リポジトリの中に置くとコミットへ紛れる。

### 文書を書いたら

```
hmwr doc lint          CIと同じ検査を回す
hmwr doc lint --fix    自動で直せるものを直す
hmwr doc check         ADRのStatusと索引の一致、相対リンクの行き先を見る
```

ADRを足したりStatusを変えたりしたら `doc check` も通す。索引の更新漏れと、
リンク先のファイル名の間違いがここで落ちる。

PRを出す前に通す。CIが落ちてから直すより速い（ADR-0178）。

### 実戦を観測する

```
hmwr kifu cycle                回収→分析→定跡追加→網羅率
hmwr kifu fetch                棋譜だけ回収する
hmwr kifu rate                 レートの推移（日毎と2週間）を出す
hmwr book seed --max-positions 100
hmwr book stats
hmwr book release <DB> <番号> --apply
```

`kifu rate` は対局者ページのグラフの元データを読む。初日からの全履歴が入って
いるので、いつ取っても同じ系列になる。`--csv` で全履歴、`--days N` で直近の
日数を変えられる。棋力の日々の物差しはこの値である。

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
