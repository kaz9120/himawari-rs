# エージェント向け作業規約

himawari-rsで作業するエージェントの規約。詳細は各文書へリンクする。
文書の役割分担は [ADR-0053](docs/adr/0053-docs-structure.md) を正とする。

## 前提: オートパイロットで進める

このリポジトリはエージェントが主体で進める。オーナーが求めるのは
**判断の記録が残ることと、棋力が上がること**の2つで、経過の逐一の
確認ではない（2026-07-29オーナー指示）。

- 方向を選ぶ場面で止まらない。根拠を書いて選び、外れたら棄却として
  記録する。棄却の記録も成果物である
- CIが緑のPRは自分でマージする。マージ条件は下の「開発フロー」の表に
  従う。棋力向上はSPRTでH1採択も要る
- ADRのacceptedもオーナーの返事を待たずに進めてよい。判断の根拠が
  ADRに残っていることが条件になる
- 確認するのは、破壊的で戻せない操作だけ。force push、履歴の書き換え、
  `data/` の削除など

効かなかった案の記録は次の判断材料になるので、必ず残す
（[ADR-0102](docs/adr/0102-move-horizon.md)・[ADR-0103](docs/adr/0103-root-score-gap.md)が例）。

この前提を守らせる仕組みは `.claude/settings.json` にある
（[ADR-0181](docs/adr/0181-agent-surface.md)）。読むだけの `hmwr` は確認なしで
通り、force pushと履歴の書き換えと `Cargo.toml` の編集は確認を通る。mainへの
直接pushは拒否される。セッションの終わりに `cargo fmt` とコミット本文の括弧を
検査する。

## 文書の役割分担

| 文書 | 読むとき | 持つ情報 |
|---|---|---|
| このファイル | 毎回 | 作業の規約 |
| [docs/adr/](docs/adr/README.md) | 判断の根拠を探すとき | 設計判断と経緯・測定の詳細 |
| [docs/ROADMAP.md](docs/ROADMAP.md) | 着手を決めるとき | 現行構成・次の方向・候補 |
| [docs/DATASETS.md](docs/DATASETS.md) | データを扱うとき | データの所在と前処理 |
| [README.md](README.md) | 使い方や環境を知るとき | 概要・対局での使い方・開発環境 |
| [CHANGELOG.md](CHANGELOG.md) | 何が入ったか見るとき | release-pleaseが生成 |
| `.claude/skills/` | 定型作業を回すとき | 手順の固定（SPRT運用は[ADR-0154](docs/adr/0154-sprt-ops.md)、CLIは[ADR-0180](docs/adr/0180-hmwr-cli-in-python.md)） |

**文書は「誰がいつ読むか」で決める**。読み手のいない文書は作らない。
同じ情報を2文書に書かない。何が入ったかはCHANGELOG.md、なぜそうしたかはADR、
次に何をするかはROADMAPが持つ。

案は ROADMAPの候補 → ADR → 完了 の順に動く。着手を決めたらADRを起こして
候補から消す。完了・棄却した案も候補には残さない。

ROADMAPは3節で構成する。現行構成・次の方向・候補。過去の経緯は書かない。

READMEには変わり続ける事実を書かない（[ADR-0182](docs/adr/0182-readme-audience.md)）。
棋力はROADMAP、配布物の版はReleases、既定のネットワーク次元は
`crates/engine/build.rs` が正になる。ADRの番号も書かない。

手順を書いたら実行して確かめる。実装が進んでも文書が追わないと、読み手を
誤った場所へ連れて行く。数値を書くときも同じで、出典（ネットのメタデータ、
ログ、ADR）に当たる。

文書はtextlintを通してからPRを出す（[ADR-0178](docs/adr/0178-textlint-gate.md)）。
一文の長さ・読点の数・助詞の重複・冗長表現・誇張表現をCIが見る。

```
hmwr doc lint        # docs/ と *.md と .claude/ を検査する
hmwr doc lint --fix  # 自動で直せるものだけ直す
```

強調の中に句点を書かない。`**要点である**。` と書き、`**要点である。**`
とは書かない。textlintの文の分割器は `。**` で文を切らないため、後者だと
次の文まで1文として数えられる。原文を変えられない引用は
`<!-- textlint-disable ja-technical-writing/sentence-length -->` で外す。

## ADRプロセス

設計判断はすべてADRに記録する（[ADR-0001](docs/adr/0001-adr-process.md)）。
proposedで起草し、1アイデア1ADRとする。

Statusは実態に追従させる。実装がmainへ入り結果が出たらacceptedに、
測って捨てたらrejectedにする。索引（`docs/adr/README.md`）の更新は同じPRで
行う。索引だけを見て「まだ入っていない」と誤読する事故を防ぐ。

**「まだ無い」と判断する前に、コードで確かめる**。棄却の記録も、文書の記述も、
grepの件数も、現在の実装ではない。無いことを主張するときだけ、実装を読む手間を
惜しまない。あることの確認は間違えてもすぐ気づくが、無いことの誤りは作業を
丸ごと無駄にする。実装済みの機能を「作る」と提案した例は
[ADR-0160](docs/adr/0160-revisit-rejected-under-better-eval.md)にある。

## SPRTゲート（[ADR-0028](docs/adr/0028-pruning-extensions.md)）

- H1採択した変更だけをmainに取り込む
- 単発の変更は1機能=1SPRT。参照実装への追従は1群=1SPRT
  （[ADR-0109](docs/adr/0109-reference-parity.md)）
- 既定条件: `--tc 10+0.1 --concurrency 8 --adjudicate 2000,8`、
  elo0=0、elo1=5、α=β=0.05
- **対立仮説は着手時に決め、走行後に変えない**（[ADR-0163](docs/adr/0163-sprt-hypothesis-choice.md)）。
  棋力向上を主張する変更は既定、参照追従で「害がなければ入れたい」変更は
  非劣性（elo0=−5、elo1=0）を選ぶ。判定が出ないから緩める、は理由にならない
- 結果（対局数、W-D-L、Elo±CI、LLR）をコミットメッセージへ書く。
  `SPRT:` トレーラがCHANGELOGへ載り、詳細はADRが持つ
- 判定に至らない走行は[ADR-0087](docs/adr/0087-sprt-resume.md)の `--resume` で
  続ける。上限で捨てず、条件を変えずに局数を積む

既定条件で測れない変更がある。時間管理は参照実装の既定値が実戦の持ち時間
（floodgateの300+10）を前提にしており、10+0.1では床が配分を支配する。条件を
変えて測るか、非劣性検定（elo0=-5、elo1=0）へ落とす。条件を変えたら、なぜ既定で
測れないかをADRに書く（[ADR-0116](docs/adr/0116-g7-timeman.md)が例）。

実行・監視・後処理の手順は running-sprt スキルに固定してある
（[ADR-0154](docs/adr/0154-sprt-ops.md)）。SPRTを回すときは必ずスキルを使う。
経過は常に `data/logs/sprt-<名前>.log` にあり、`hmwr sprt show` で
途中経過も読める。

SPRTの前に機能検証を行う（[ADR-0074](docs/adr/0074-feature-verification.md)）。
固定深さで3局面以上のノード数を変更前後で比べ、変わることを確かめる。
全局面で一致したら探索に影響していない。枝刈り・延長は発動率も測り、
0.1%を下回るならSPRTにかけない。

他エンジンから移すときは、その値が何に支えられているかまで移す。確かめる
のは3つある。係数のスケールが本エンジンで成り立つか、発動頻度の設計点が同じか、
その値を守っている別の仕組みがないか。3つ目を落として失敗した例が2件ある
（[ADR-0116](docs/adr/0116-g7-timeman.md)・[ADR-0118](docs/adr/0118-g9-aspiration.md)）。
探索定数を足すADRには、出典・前提・成立の根拠を書く。

3つを満たしても弱くなることがある。参照実装の構造と定数は、参照の評価関数と
生態系で最適化されている。本エンジンには独自の平衡があり、参照の設計点へ寄せる
ほど弱くなった例が2件ある。singular率の較正は−81.2 Elo
（[ADR-0141](docs/adr/0141-singular-rate-calibration.md)）、参照にない自傷の除去は
利得ゼロだった（[ADR-0155](docs/adr/0155-reference-walkthrough.md)）。
**「参照にあるから正しい」は棋力の根拠にならない**。乖離を見つけたら、直す前に
「本エンジンでそれが成り立つ理由」を書く。

## 学習の測定

検証損失を足切りに使わない（[ADR-0158](docs/adr/0158-mirror-factorizer.md)）。
初期値の系列が違うだけで0.00136動く。モデルの構造を変えると乱数の消費順序が
変わるので、0.001〜0.002規模の差は誤差と区別がつかない。

教師データの分布を変える実験では、物差しも一緒に動く
（[ADR-0136](docs/adr/0136-quiet-teacher-positions.md)）。静止化した教師で学習した
ネットは、非静止の検証集合で0.0285悪い値を出しながら、対局では
+20.3 Elo [+9.4, +31.2] で勝った。検証集合を学習データと同じ土俵へ揃えてから
読む（`train.py --eval-only` で土俵を裏返して測れる）。**採否は対局で決める**。

## 開発フロー（[ADR-0070](docs/adr/0070-pr-based-workflow.md)）

変更はすべてPR経由で入れる。mainへ直接pushしない。
PRテンプレートで種別を選び、種別ごとにマージ条件とバージョンが決まる。

| 種別 | 対象 | マージ条件 |
|---|---|---|
| 棋力向上 | 探索・評価関数・時間管理など、強さが変わる変更 | CIが緑、かつSPRTでH1採択 |
| その他 | リファクタ、文書、ツール、CI、依存更新 | CIが緑 |

迷う変更は「その他」に倒す。バージョンとタグはrelease-pleaseが作る
（[ADR-0071](docs/adr/0071-release-please.md)）。`Cargo.toml` は触らない。

ブランチはorigin/mainから切り、1ブランチ1PRとする。マージ済みブランチは
再利用しない。名前は着手時に `<型>-adrNNNN-<slug>` で決め、途中で改名
しない。rebaseで競合したら解決を終えてから次の操作へ移る（競合の途中で
ビルドや計測をしない）。詳細は[ADR-0070](docs/adr/0070-pr-based-workflow.md)
の「ブランチ運用」にある。

PRの作成はテンプレートを明示して行う。

```
gh pr create --template strength.md   # 棋力向上
gh pr create --template chore.md      # その他
```

## バージョニング（[ADR-0068](docs/adr/0068-sprt-driven-versioning.md)・[ADR-0071](docs/adr/0071-release-please.md)）

コミットの型からrelease-pleaseが算出する。

| 型 | 対象 | bump |
|---|---|---|
| `feat` | SPRTでH1採択した変更 | MINOR |
| `fix` | 棋力に影響しないコードの変更 | PATCH |
| `docs` | 文書のみの変更 | なし |
| `chore` | CI・設定・テスト・依存更新 | なし |

判断はバイナリが変わるか、棋力が変わるかの2問で決まる。
MAJOR（選手権への参加。次回2027年5月を1.0.0）は
`Release-As: 1.0.0` トレーラで明示する。

## コミット規律

- 1コミット=単一の論理変更
- 件名はConventional Commitsの型で始める（`feat:` / `fix:` / `docs:` /
  `chore:`）。本文は日本語の平叙文のままでよい
- 棋力向上のコミットは件名にEloを入れる。CHANGELOGに数値が残る
  （例: `feat: razoringを導入する（+184.8 Elo、ADR-0057）`）
- コミット前に `cargo fmt` を必須とする（CIが `rustfmt --check` を強制）
- 棋力が変わる変更には `SPRT:` トレーラを付ける。書式は
  `SPRT: <Elo> [<CI下限>,<CI上限>] <対局数>games <H0|H1>`。
  `Co-Authored-By` と同じ位置に書く。release-pleaseがCHANGELOGへ載せる
- 非劣性で採択した変更は、末尾へ条件を書く（[ADR-0163](docs/adr/0163-sprt-hypothesis-choice.md)）。
  `SPRT: +8.9 [-0.9,+18.8] 5000games H1（非劣性 elo0=-5 elo1=0）` の形で、
  件名にEloを入れない。棋力の向上を主張しないためである
- 本文に入れ子の半角括弧を書かない。release-pleaseのパーサが解析に失敗し、
  **その回のリリースが黙って止まる**。1件でも失敗すると全コミットを捨てて
  「対象なし」で終わるので、緑のCIとマージ済みのPRだけが残り、気づくのが
  遅れる。`(clip(a)*clip(b)+64)` のような書き方が該当する。コードを示すなら
  バッククォートで囲むか、文で書く

## テスト

**CIはローカルより遅い。時間に依存するテストを書かない**。`sleep` で待つと
遅い環境で落ちる。待つならポーリングにし、判定はマシンの速度に依存しない量で
行う。「反復深化が再起動したか」は経過時間ではなく「同じ深さの確定infoが2回
出たか」で判定できる。

## 日常操作（[ADR-0180](docs/adr/0180-hmwr-cli-in-python.md)）

**入口は `hmwr` コマンドである**。ビルド・機能検証・NPS計測・学習・教師データの
前処理・文書のlintがここから動く。個別のスクリプトを直接叩く前に
`hmwr --help` を見る。パスが通っていなければ `./bin/hmwr` で呼ぶ。

```
hmwr sprt run <名前>                ペア作成→機能検証→SPRT起動
hmwr verify <名前>                  固定深さで探索の変化を比べる
hmwr net train <名前> --data <psv>  ネットを学習する
hmwr --dry-run <...>                走るはずのコマンドを表示する
```

覚えることは3つある。

- オプションはフラグで渡す。環境変数への変換はCLIが行う
- ログの置き場は書かない。`data/logs/<領域>-<名前>.log` へ決まる
- 終了コードは0=成功・1=判定結果・2=引数・3=実行時（ADR-0122）

使い方の詳細は himawari-cli スキルにある。実装は `hmwr/` のPythonパッケージに
あり、`scripts/` に残るのは環境構築の `setup.sh` だけである。

## ビルド

計測・対局は `-C target-cpu=native` で行う（[ADR-0003](docs/adr/0003-toolchain.md)）。

```
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

配布・対局用の単体ビルドはPGOで作る（`hmwr build pgo`、+10%前後のNPS。
[ADR-0151](docs/adr/0151-speedup-sweep.md)）。**SPRTのペアには使わない**。
両側を同条件（PGOなし）で作るほうが公平で、`hmwr build pair` の既定手順が
そのまま使える。

## data/ の配置（[ADR-0053](docs/adr/0053-docs-structure.md)）

すべてgitignore対象で、置き場は次のように決まる。

| 置き場 | 中身 |
|---|---|
| `data/raw/<データセット名>/` | 生データ |
| `data/train/` | 加工済みpsv |
| `data/nets/` | 学習済みネット |
| `data/book/` | 定跡 |
| `data/sprt/` | selfplayの棋譜ログ（jsonl） |
| `data/bin/` | 比較用に残すビルド済みバイナリ |
| `data/profile/` | プロファイル結果 |
| `data/logs/` | hmwrの実行ログ |

ログのリダイレクト先を手で決めない。`hmwr` が `data/logs/<領域>-<名前>.log`
へ追記する（[ADR-0149](docs/adr/0149-experiment-runner.md)・[ADR-0180](docs/adr/0180-hmwr-cli-in-python.md)）。
チェックポイント（*.pt）は `training/checkpoints/` に置く。
ネットのファイル名は実験名を含める（例: `halfkp_180M.hmwr.best`）。
