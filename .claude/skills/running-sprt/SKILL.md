---
name: running-sprt
description: SPRT（対局ゲート）の起動・監視・終了後の記録を定型手順で行う。棋力が変わる変更の検証、sprt.shの実行、途中経過の確認、H1/H0後の後処理のとき必ず使う。
---

# SPRTの実行・監視・後処理

棋力が変わる変更はすべてこの手順で測る（ADR-0028・0154）。
毎回やることなので、起動方法・ログの場所・後処理を固定する。
条件の意味と例外（時間管理など既定で測れない変更）はCLAUDE.mdの
「SPRTゲート」を正とする。

## 名前の規則

`<名前>` は `adrNNNN-<slug>`（例: `adr0153-superior`）。この名前で
3つの場所が決まる。

| 置き場 | パス |
|---|---|
| 経過ログ | `data/logs/sprt-<名前>.log`（追記。sprt.shが必ず書く） |
| 棋譜 | `data/sprt/<名前>.jsonl` |
| バイナリ | `data/bin/base-<名前>` / `data/bin/cand-<名前>` |

## 1. 起動

```
scripts/build-pair.sh <名前>          # base=origin/main、cand=HEAD
cargo run --release -p himawari-tools --bin verify -- data/bin/base-<名前> data/bin/cand-<名前>
scripts/sprt.sh data/bin/base-<名前> data/bin/cand-<名前> <名前>
```

- verifyを先に行う（ADR-0074）。固定深さで全局面のノード数が一致する
  変更はSPRTにかけても中立にしかならない
- sprt.shは経過を端末と `data/logs/sprt-<名前>.log` の両方へ出す。
  バックグラウンドで起動しても経過ログは同じ場所に残る
- 条件を変えるときは環境変数（`SPRT_TC=` など）。変えた理由をADRに書く

## 2. 監視

```
python3 scripts/sprt-summary.py data/logs/sprt-<名前>.log   # 今の値を1回表示
tail -f data/logs/sprt-<名前>.log                            # 流し見
scripts/watch-sprt.sh data/logs/sprt-<名前>.log              # 判定まで待つ
```

- summaryは判定前でも最後のpairs行から途中経過を出す（判定欄は「判定前」）
- 経過の読み方: `LLR +2.94` でH1採択、`-2.94` でH0採択。中間で漂うのは
  効果がelo0とelo1の間にある徴候で、長期戦になる

## 3. 終了後

sprt.shの終了コード: 0=H1、1=H0、2=判定に至らず。

```
python3 scripts/sprt-summary.py data/logs/sprt-<名前>.log
```

が「コミットのトレーラ」「結果の表」を整形して出すので、これを使う。

- **H1採択**: featコミットの件名にEloを入れ、`SPRT:` トレーラを付けて
  PR（棋力向上テンプレート）。結果（対局数、W-D-L、Elo±CI、LLR）を
  ADRへ追記し、Statusをacceptedにする
- **H0採択**: 棄却として結果をADRへ記録する。棄却の記録も成果物
  （CLAUDE.md）。コードはmainに入れない
- **判定に至らず（max-pairs到達など）**: 数字と判断（非劣性へ落とす・
  条件を変えて測り直す・棄却）をADRへ記録する。非劣性は
  `SPRT_ELO0=-5 SPRT_ELO1=0` で別名（`<名前>-noninf` など）を付けて回す
- どの結果でも: `data/bin/` のバイナリと `data/sprt/` の棋譜は消さない
  （比較・再現の材料として残す。ADR-0053）
