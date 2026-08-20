# 0179: 日常操作をhmwrコマンドひとつの入口にまとめる

- Status: superseded（[ADR-0180](0180-hmwr-cli-in-python.md)が置き換えた）
- Date: 2026-08-20
- 関連ADR: [0180](0180-hmwr-cli-in-python.md), [0122](0122-tooling-language-split.md), [0149](0149-experiment-runner.md), [0154](0154-sprt-ops.md), [0074](0074-feature-verification.md)

**入口の一貫性という目的は引き継ぐが、実処理を既存shellへ委譲する判断は
差し戻した**（2026-08-20オーナー指摘）。入口が2つ残る状態は統一ではなく
先送りであり、下のConsequencesに自分で書いた危険がそのまま残る。移行の
段取りは[ADR-0180](0180-hmwr-cli-in-python.md)にある。

## Context

日常の操作を始める入口が3系統ある。

| 系統 | 本数 | 呼び方 |
|---|---|---|
| shell | 20 | `scripts/build-pair.sh adr0179` |
| Rust | 11 | `cargo run --release -p himawari-tools --bin verify -- base cand` |
| Python | 5 | `python3 scripts/sprt-summary.py data/logs/sprt-adr0179.log` |

**問題は系統が3つあることではない**。[ADR-0122](0122-tooling-language-split.md)
の言語の境界は今も正しく、shellが得意な仕事をPythonへ書き直す理由はない。
問題は**系統ごとに、いや1本ごとに世界観が違うこと**にある。使う側が覚える対象は
コマンドの数だけある。

### 世界観のばらつき

- **オプションの渡し方が3通りある**。環境変数で渡すもの（`train-shapes.sh` は
  18個、`train-net.sh` は8個、`env.sh` は12個）、フラグで渡すもの（`--apply`・
  `--from`・`--tag`）、位置引数だけのものが混在する。`train-net.sh` は学習率を
  フラグで受けられず、`TRAIN_PEAK_LR=1e-4 scripts/train-net.sh ...` と書く
- **サブコマンドを持つのは `fetch-dataset.sh` だけ**である。ほかは1本1機能で、
  近い操作でも別のファイル名になる（`build-pair.sh`・`build-pgo.sh`・
  `build-shapes.sh`）
- **予行演習があるのは `release-*.sh` だけ**である。1時間かかる学習や、
  数日かかるSPRTには下見の口がない
- **`<名前>` の意味がスクリプトごとに違う**。SPRTでは実験名、`train-net.sh`
  ではネット名でありログ名でありチェックポイントの置き場である

### ログ名が規約から外れていく

[ADR-0149](0149-experiment-runner.md)が `data/logs/<名前>.log` を決め、`env.sh`
に `log_path` と `run_logged` を置いた。**置き場は統一されたが、名前は
呼び出し側が自由に決めるままだった。** 91ファイルの実態が次である。

| 状態 | 件数 |
|---|---|
| 領域プレフィックスがある（`sprt-`・`bench-`・`quiet-` など） | 62 |
| プレフィックスがない（`ft1024_2990M_q1.log`・`gen2_1e4.log` など） | 22 |
| そもそも `.log` でない（`.txt`・`.md`・`.tsv`） | 7 |

`sprt-` の36件だけが揃っている。[ADR-0154](0154-sprt-ops.md)がrunning-sprt
スキルへ手順を固定し、`sprt-run.sh` がログ名を自分で組み立てているためである。
**規約を守らせているのは文書ではなく、名前を組み立てるコードのほうである。**

## 選択肢と比較

### 案A: 現状のまま、規約を文書に書く

CLAUDE.mdへ「ログ名は `<領域>-<名前>.log` にする」と書く。実装を変えずに済む。
ただしADR-0149が同じことを既に書いており、それでも22件が外れた。
[ADR-0122](0122-tooling-language-split.md)が「思い出した人しか守られない」形を
既定で退けたのと同じ理由で、これは効かない。

### 案B: shellのディスパッチャを足す

`scripts/hmwr` をbashで書き、既存スクリプトへ振り分ける。実装が小さい。
ただし引数の検証と組み立てが要る仕事で、macOSの `/bin/bash` は3.2のため
連想配列が使えない。世界観を統一する層をshellで書くと、統一したい対象と
同じ落とし穴を踏む。

### 案C: Pythonで入口を作る

`scripts/hmwr` をPythonで書く。argparseがサブコマンド・ヘルプ・引数検証を持つ。
既存のPython資産（`sprt-summary.py`・`sprt-detach.py`）と同じ言語で、
`scripts/tests` のpytestがそのまま効く。実処理は既存スクリプトへ委譲する。

### 案D: すべてRustのバイナリにする

`crates/tools` にclapでhmwrを作る。型安全でテストも書ける。ただし日常の入口が
`cargo run --release -p himawari-tools --bin hmwr --` になり、変更のたびに
コンパイルが要る。ADR-0122が案Bとして退けた理由（`gh release create` を
`Command::new` の羅列で書いても読みやすくならない）がそのまま当てはまる。

## Decision

案Cを採る。`scripts/hmwr` をPythonで書き、日常操作の入口をここへ集める。

**決め手は、統一したいものが引数の扱いとログの命名だったことである**。
どちらもコードで強制しないと守られない。ログ名の36対22という実績が、規約を
書くだけでは効かないことと、名前を組み立てるコードが効くことの両方を示している。

### このCLIが統一する4つ

1. **オプションはフラグで渡す**。環境変数への変換はCLIが行う。
   `hmwr train x --data d.psv --lr 1e-4` と書けば `TRAIN_PEAK_LR=1e-4` へ畳む
2. **実験名を検証し、ログの置き場をCLIが決める**。`data/logs/<領域>-<名前>.log`
   になり、領域はサブコマンドから決まる。呼び出し側はリダイレクト先を書かない
3. **`--dry-run` ですべてのコマンドを下見できる**。走るはずのコマンド列を
   そのまま表示する。長時間の操作を始める前に条件を確かめられる
4. **終了コードはADR-0122の表に従う**（0成功 / 1判定 / 2引数 / 3実行時）

### 手順のまとまりを持たせる

**単なるディスパッチャなら価値は薄い。** CLIが持つべきは「その操作の正しい
順番」である。`hmwr sprt start <名前>` は3つを1コマンドで行う。

1. `build-pair.sh` でbaselineとcandidateを同条件で作る（ADR-0081）
2. `verify` で機能検証する。**全局面でノード数が一致したら起動せずに止まる**
   （ADR-0074）。飛ばすには `--no-verify` を明示する
3. `sprt-detach.py` で新しいセッションへ切り離して起動する（ADR-0175）

running-sprt スキルが文章で固定していた手順が、コマンド1つになる。順番を
飛ばすには明示が要る形になった。

### 実処理は委譲する

**既存の20本のshellと11本のRustバイナリはそのまま残す。** CLIが足すのは入口の
一貫性だけで、中身は書き直さない。ADR-0122の言語の境界は変わらない。

直接呼ぶ道も塞がない。条件を変えて1回だけ試すときや、CLIがまだ覆っていない
操作（`build-shapes.sh`・`train-shapes.sh`・`league`）は今までどおり叩く。
**「本当に必要なものは何か」で載せる範囲を決めた**ので、たまにしか使わない
操作を無理に通さない。

### 載せた操作と、載せなかった操作

実績（`data/sprt` 112件、`data/nets` 263件、`data/bin` 252件、`data/book` 9件）
から、毎日使うものを載せた。

| 載せた | 理由 |
|---|---|
| `sprt start` / `sprt status` | 棋力向上の変更はすべてここを通る |
| `build pair` / `build pgo` | SPRTと配布の前段 |
| `verify` / `bench` | 長い `cargo run` を短くする |
| `train` / `eval` / `quiet` | 学習の3点セット |
| `data fetch` / `floodgate cycle` / `release` / `doc lint` | 入口を揃える価値がある |
| `env` | 測る前に条件を確かめる |

載せなかったのは `build-shapes.sh`・`train-shapes.sh`（構成比較の実験用で、
環境変数が18個ある）と `league`・`profile`（頻度が低い）である。
**必要になったら足す。使わない入口を先に作らない。**

## Consequences

- 覚えるのが `hmwr <領域> <操作>` の1系統になる。`hmwr --help` から全体を辿れる
- ログ名が `<領域>-<名前>.log` へ揃う。**既存の91ファイルは改名しない**。
  過去の記録であり、参照しているADRがある
- 引数処理にpytestが効く。34件のテストで、同じ入力から同じコマンド列が出ることを
  固定した。**このCLIの価値は再現性なので、そこを測る**
- Pythonの依存は標準ライブラリだけである。CIで動く必要はないが、
  `scripts/tests` のpytestが引数処理を検査する
- 入口が2つある期間は続く。CLIと既存スクリプトの両方が動く。**片方だけを
  直して他方が古くなる危険をもつ**ため、既存スクリプトのオプションを変えたら
  CLI側のフラグも合わせる。引数の対応はテストが固定しており、ずれると落ちる
- 見直しのトリガーは、CLIを通さない操作が日常に増えることである。3つを超えたら
  載せる範囲を決め直す
