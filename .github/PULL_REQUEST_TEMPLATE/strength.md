---
name: 棋力向上
about: 探索・評価関数・時間管理など、強さが変わる変更
labels: strength
---

## 何を変えたか

<!-- 変更の中身を1〜3文で。関連ADRがあればリンクする -->

## なぜ効くと考えたか

<!-- 仮説。SPRTが不採択でも、この記述が次の設計材料になる -->

## SPRT

条件（既定から変えた場合のみ記載）:

```
--tc 10+0.1 --concurrency 8 --adjudicate 2000,8
elo0=0 elo1=5 alpha=0.05 beta=0.05
```

結果:

| 項目 | 値 |
|---|---|
| baseline | |
| candidate | |
| 対局数 | |
| W-D-L | |
| Elo [95%CI] | |
| LLR | |
| 判定 | H1 / H0 / 打ち切り |

コミットに付けるトレーラ（ADR-0069）:

```
SPRT: <Elo> [<CI下限>,<CI上限>] <対局数>games <H0|H1>
```

## チェック

- [ ] CIが緑
- [ ] SPRTでH1採択（これがマージ条件。ADR-0070）
- [ ] `Cargo.toml` のMINORを上げた（ADR-0068）
- [ ] RESULTS.mdへ結果を追記した（append-only）
- [ ] 設計判断があればADRを起草し、acceptedにした
