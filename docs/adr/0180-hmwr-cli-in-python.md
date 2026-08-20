# 0180: hmwrを独立コマンドにし、実処理をPythonへ移す

- Status: accepted
- Date: 2026-08-20
- 関連ADR: [0179](0179-hmwr-cli.md)（これを置き換える）, [0122](0122-tooling-language-split.md), [0149](0149-experiment-runner.md)

## Context

[ADR-0179](0179-hmwr-cli.md)が `scripts/hmwr` を作り、日常操作の入口を1つに
まとめた。ただし実処理は既存のshellへ委譲し、20本のスクリプトをそのまま
残す設計にした。同ADRのConsequencesには、こう書いてある。

> 入口が2つある期間は続く。CLIと既存スクリプトの両方が動く。**片方だけを
> 直して他方が古くなる危険をもつ**

2026-08-20のオーナー指摘で、この設計を差し戻した。要点は3つある。

1. **入口が2つ残る状態は統一ではなく先送りである。** 「既存を壊さない」を
   優先した結果、解こうとした問題（世界観の不統一）が形を変えて残った
2. **ヘルプに設計記録の番号が出るのは道具として正しくない。** コマンドは
   利用者のためのもので、`--help` にADRの番号を並べる理由がない。追跡は
   ソースのdocstringで足りる
3. **`scripts/` の下に置くと、スクリプトの1本に見える。** 独立したコマンド
   として使うなら `bin/` に置き、パスを通して `hmwr` と打てるべきである

## 選択肢と比較

### 言語をどれにするか

移行先の言語を決める必要がある。判断の材料は、このCLIが実際にやる仕事の
中身である。

| 仕事 | 例 | 量 |
|---|---|---|
| 外部プロセスの起動と待機 | `cargo build`・`gh release create`・`npm run lint` | 多い |
| 出力の解析と整形 | SPRTログの集計、プロファイルの表化、リーグの順位 | 多い |
| USIエンジンを起動して測る | ノード数の比較、NPSの計測 | Rust側に既にある |

**案A: Rustにする**。`crates/tools` にclapで書く。型が付き、単一バイナリに
なるのでパスを通すだけで動く。ただし出力の解析はすでにPythonで5本ある。
内訳は次のとおりで、合計1,143行ある。

```
sprt-summary.py 286 / floodgate-fetch.py 278 / profile-report.py 235
ft-reorder.py   198 / league-summary.py  146
```

Rustへ移すと、これらを書き直すかsubprocess越しに呼び続けるかになる。
後者なら統一の効果は薄い。学習側（`training/`）もPythonで、そちらとの
やり取りも増える。

**案B: shellで本格的に書く**。既存の資産をそのまま活かせる。ただし
macOSの `/bin/bash` は3.2で連想配列がない。引数の検証・サブコマンド・
テストのいずれも、shellで書くと[ADR-0122](0122-tooling-language-split.md)が
挙げた落とし穴（trapの漏れ、終了コードの握り潰し）へ戻る。統一する層が
統一したい対象と同じ弱点を持つ。

**案C: Pythonにする**。出力の解析5本をimportで取り込める。argparseが
サブコマンド・ヘルプ・引数検証を持ち、pytestがそのまま効く。shebangで
直接実行できるので、パスを通せば `hmwr` として動く。型はtype hintsとテストで
固める。

### 移行を一度にやるか、段階的にやるか

20本のshellと5本のPython、計3,296行を書き換える。一度に出すと差分が巨大で
レビューできず、途中で壊れてもmainが赤いまま止まる。領域ごとにPRを分け、
各PRでCIが緑を保つ形にできる。

## Decision

案Cを採る。**Pythonで書き、`bin/hmwr` を入口にして、実処理を段階的に移す**。

**決め手は、このCLIの仕事の半分が「出力の解析と整形」だったことである**。
それがすでにPythonで1,143行ある。USIエンジンを起動して測る仕事はRustの
`bench`・`verify`・`profile` が持ち続けるので、ADR-0122の言語の境界は
そこに残る。境界が動くのは「外部コマンドを順に並べる」仕事だけで、それが
shellからPythonへ移る。

### 置き場と呼び方

```
bin/hmwr            入口（shebangつき。パスを通して hmwr と打つ）
hmwr/               パッケージ本体
  cli.py            引数の解析と振り分け
  paths.py          置き場と名前の検証
  config.py         マシン設定と測定の既定条件
  proc.py           プロセス起動・ログ・予行演習・終了コード
  commands/         領域ごとの実装
tests/              引数処理のテスト
```

`scripts/setup.sh` がパスの通し方を案内する。リポジトリのどこから呼んでも、
`bin/hmwr` の位置からリポジトリルートを決めるので動く。

### コマンド体系

「領域 → 操作」で揃える。領域だけ渡すとその領域のヘルプが出る。

```
hmwr env                          設定を表示する
hmwr build pair|pgo|engine        ビルドする
hmwr sprt run|show|wait           対局で検定する
hmwr verify <名前 | バイナリ...>  挙動が変わったかを見る
hmwr bench <バイナリ>...          速度を測る
hmwr net train|eval|release       評価関数を扱う
hmwr data fetch|quiet             教師データを扱う
hmwr doc lint                     文書を検査する
```

`verify` と `bench` は使う頻度が高いので領域を挟まない。

### ヘルプには設計記録の番号を書かない

**`--help` の読み手は道具を使う人であって、設計の経緯を追う人ではない。**
ADRの番号はソースのdocstringとコメントに置く。テストで、ヘルプ全体に
「ADR」の文字列が出ないことを固定した。

### 移行の段取り

領域ごとに5本のPRへ分ける。各PRでCIが緑を保ち、移した領域のshellを同じPRで
削除する。**削除まで含めて1つのPRにするのは、消し忘れが「入口が2つ」を
生むからである。**

| PR | 範囲 | 消えるもの |
|---|---|---|
| A | 骨格・体系・パス通し | `scripts/hmwr` |
| B | 対局ゲート | `sprt.sh`・`sprt-run.sh`・`sprt-net.sh`・`sprt-detach.py`・`watch-sprt.sh` |
| C | ビルド | `build-pair.sh`・`build-pgo.sh`・`build-shapes.sh` |
| D | 学習とデータ | `train-net.sh`・`train-shapes.sh`・`eval-net.sh`・`quiet.sh`・`fetch-dataset.sh` |
| E | 配布・棋譜・集計 | `release-*.sh`・`floodgate-cycle.sh`・`watch-ci.sh`・`env.sh` |

`setup.sh` は残す。パスを通す前に走らせるものなので、CLIの中に置けない。

## Consequences

- 入口が1つになる。移行が終われば `scripts/` に残るのは `setup.sh` だけになる
- `hmwr` としてどこからでも呼べる。`cargo run --release -p ... --bin ... --` を
  打つ必要がなくなる
- 移行中は `hmwr` が既存shellを呼ぶ期間が続く。**PRごとに、移した領域の
  shellを同じPRで消す**ことで、二重に存在する時間を領域単位に閉じ込める
- `env.sh` は最後まで残る。shellのスクリプトが値を読んでいるためで、
  移行中は `config.py` がそこから値を取り込む。**値を二重に持たない**
- Pythonの依存は標準ライブラリだけである。CIで `pytest tests` が引数処理を
  検査する
- ADR-0179はsupersededにする。委譲方式の判断そのものは残す。**「既存を
  壊さない」を優先すると統一が先送りになる**という記録に意味がある
- 見直しのトリガーは、CLIの起動が体感で遅くなることである。Pythonの起動は
  50ms前後で、日常の入口として問題にならない。設定の読み込みで外部プロセスを
  呼ぶ箇所（移行中の `env.sh`）が増えたら測り直す
