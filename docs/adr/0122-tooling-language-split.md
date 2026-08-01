# 0122: 開発スクリプトを役割で3言語に分ける

- Status: proposed
- Date: 2026-08-01
- 関連ADR: [0027](0027-sprt-framework.md), [0074](0074-feature-verification.md), [0081](0081-portability.md), [0098](0098-agent-permissions.md)

## Context

`scripts/` の16本はすべてshellで書いてある。場当たりに増やしてきた結果、
毎回使うものほど壊れやすい状態になった。棚卸しで次が分かった。

**USIエンジンを起動して測る一式が3本に丸ごと複製されている。**
`bench-nps.sh:62-118`、`profile.sh:71-105`、`verify-feature.sh:61-112` が、
`mktemp -d` → `mkfifo` → バックグラウンド起動 → `exec 3>fifo` → ハンドシェイク送信
→ `grep -c '^bestmove'` でポーリング → `rm -rf` を各自に持つ。**3本ともtrapがない。**
中断すると一時ディレクトリとエンジンプロセスが残る。検証局面の4つのSFENも3本に
複製されている。

ほかに、`bench-nps.sh:135` の `nps=$((nodes * 1000 / ms))` は `ms=0` でゼロ除算に
なる。`IFS=$'\t' read -r ... < <(run_once ...)` はプロセス置換なので、関数内の
`exit 3` が呼び出し側に伝わらない。usageの終了コードは1・2・3に割れている。
どれもshellで構造的に避けにくい種類の失敗である。

**同じ処理がRustに既にある。** `crates/tools/src/bin/selfplay/engine.rs` の
`UsiEngine` は、タイムアウト付きの待ち受けと `Drop` でのプロセス回収を持つ
（[ADR-0027](0027-sprt-framework.md)）。ただし `selfplay` バイナリのサブモジュール
なので、他から使えない。

## 選択肢と比較

### 案A: shellのまま共通化する

`env.sh` に共通ロガーとUSIランナーを足し、6本のsourceで共有する。変更が小さく、
既存の慣習を壊さない。ただしtrapによる後始末、終了コードの伝播、ゼロ除算といった
shell固有の落とし穴は「気をつけて書く」以上の対策が取れない。テストも書けない。

### 案B: 全部Rustにする

16本すべてを `crates/tools` のbinにする。cargoで一元管理でき、テストが全部に効く。
ただし `gh release create` や `apt install` を並べるだけのスクリプトが
`Command::new` の羅列になる。shellが1行で書くことに10行かかり、変更のたびに
コンパイルが要る。

### 案C: 全部Pythonにする

1言語で完結し、書き換えの敷居は低い。標準ライブラリ（argparse・subprocess・
logging・pathlib）で今の不足は埋まる。ただし既にRustにある `UsiEngine` を捨てて
書き直すことになる。エンジンとツールで同じUSIクライアントを2つ持つ。

### 案D: 役割で分ける

「プロセスを起動して構造化された結果を取り出す」仕事をRust、「ログやプロファイルを
解析する」仕事をPython、「外部コマンドを順に並べる」仕事をshellに割り当てる。

## Decision

案Dを採る。

**決め手は `UsiEngine` が既にあることである。** 3本のshellが抱えていた問題は、
ほぼそのまま `UsiEngine` の設計で解けている。`Drop` がプロセスを必ず回収するので
trapが要らない。`recv_timeout` で待つので `sleep 0.1` のポーリングが要らない。
`Result` で失敗が伝わるのでプロセス置換の握り潰しが起きない。案Aで手当てするより、
既にあるものを使うほうが短く、確実である。

案Bを採らないのは、shellが得意な領域があるからである。`gh release create` に
オプションを並べる処理をRustで書いても、読みやすさは上がらない。案Cを採らないのは、
USIクライアントを2つ持つことになるからである。

### 言語の境界

| 仕事 | 言語 | 対象 |
|---|---|---|
| USIエンジンを起動して測る | Rust | `bench`（NPS）・`verify`（機能検証）・`profile`（samply起動） |
| ログ・プロファイルの解析 | Python | `profile-report.py`・SPRTログの集計 |
| 外部コマンドを順に並べる | shell | `setup`・`fetch-dataset`・`build-pair`・`release-*`・`sprt*`・`watch-*` |

境界の判定は「構造化されたデータを持ち回るか」で行う。ノード数・NPS・評価値を
集計して表にする仕事はRustかPython、`gh` や `cargo` を呼んで終わる仕事はshellである。

### Rust側の構成

`crates/tools/src/lib.rs` を新設し、binから共有できるようにする。

- `usi_engine`: `selfplay/engine.rs` を移す。起動コマンドに引数を渡せるよう拡張する
  （`profile` が `samply record -- <engine>` として起動するため）
- `positions`: 3本のshellに複製されていた検証4局面と深さ調整（`[0,0,-3,0]`）を1か所に置く

binは `bench`・`verify`・`profile` の3つを足す。既存の
`bench-nps.sh`・`verify-feature.sh`・`profile.sh` は削除する。

### 依存クレートの追加

`crates/tools` にのみ足す。`clap`（CLI）、`anyhow`（エラー）、`serde` /
`serde_json`（jsonl）である。

**エンジン本体（`core`・`engine`・`usi`）は依存ゼロを保つ。** 配布するのは
`himawari` バイナリだけで、そこに外部クレートを入れる理由がない。ビルド時間と
監査範囲も本体に効く。`crates/tools` は開発用で配布しないので、この制約を
かけない。

### 終了コードの規約

| コード | 意味 |
|---|---|
| 0 | 成功 |
| 1 | 判定結果（`verify` でノード数が全一致した等、正常だが「進むな」を意味する） |
| 2 | 引数エラー |
| 3 | 実行時エラー |

現状は同じ引数エラーが1・2・3に割れている。`verify-feature.sh:158` の
「全一致で `exit 1`」は判定結果を返す設計なので、1の意味として残す。

**`sprt-summary` はこの表に従わない。** SPRTの判定そのものを終了コードで返す
（0=H1採択、1=H0採択、2=判定に至らず、3=読めない。[ADR-0081](0081-portability.md)）。
シェルから判定で分岐するための設計で、汎用の成否とは別の意味を持つ。表に合わせると
運用が壊れるので、例外として残す。

## Consequences

- 計測系の3本からtrap漏れ・ゼロ除算・終了コードの握り潰しが構造的に消える
- 検証局面が1か所になる。今は3本に複製されており、片方だけ足すと条件がずれる
- 計測系にテストが書けるようになる。今の16本にテストは1つもない
- `crates/tools` のビルド時間が伸びる。`clap` と `serde` の分で初回が数十秒増える。
  配布バイナリは変わらない
- 3言語を跨ぐことになる。どれで書くかを都度考える必要が出る。境界の判定基準を
  上の表に書いたのはそのためである
- shellに残る10本は `env.sh` を共通ライブラリとして使う。ログ関数（`log_step` /
  `log_info` / `log_warn` / `log_error` / `die`）、前提チェック（`require_file` /
  `require_executable` / `require_command`）、リリース処理の骨格をここへ集めた
- **外から見える操作には予行演習の口を用意する。** `release_create` は
  `RELEASE_DRY_RUN=1` で、実行せずコマンドとノート本文だけを出す。この整備の作業中に、
  動作確認のつもりで `gh release create` を実際に走らせ、`book-v99999` を作ってしまった
  （直後に削除し、既存のリリースとタグは無事）。リリースは消しても「あった」ことは残る。
  確認したいのは組み立てたコマンドの中身であって、リリースが作られることではない
