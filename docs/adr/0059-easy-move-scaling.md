# 0059: 思考時間を局面の難易度でスケールする

- Status: accepted
- Date: 2026-07-25
- 関連ADR: [0021](0021-time-management.md), [0058](0058-iteration-start-cutoff.md), [0062](0062-root-move-nodes.md), [0032](0032-multipv.md), [0055](0055-lmr-terms.md)

## Context

現在の反復深化は、局面の難易度を時間配分に反映しない。
search.rs:319 の終了判定は `stopped() || over_optimum()` で、経過時間
だけを見る。有力手が1つしかない局面でも、一手詰めでも、序盤の定跡形
でも、同じ時間を使う。

floodgateの実戦（2026-07-24〜25）では、910戦でHimawariの1手平均が
21.1秒だった。optimumは16.4秒なので1.3倍である。26手で549秒を使い、
使用可能な総量560秒をほぼ使い切った。

ADR-0058では、この超過を止めるために「打ち切り判定を
`elapsed >= optimum × 0.6` に変える」案を検討した。しかしStockfishと
やねうら王の実装を確認したところ、両者は比率で判定していない。
optimum自体を係数の積で伸縮させ、比率1.0で判定する。

```cpp
totalTime = optimum * fallingEval * reduction * bestMoveInstability
          * highBestMoveEffort;
if (elapsedTime > std::min(totalTime, double(maximum)))
    → stop
```

係数の値域は次のとおりである。

| 係数 | 値域 |
|---|---|
| fallingEval | 0.576〜1.728 |
| reduction | 約0.76〜1.10 |
| bestMoveInstability | 1.077〜 |
| highBestMoveEffort | 0.693〜0.838 |最善手が安定して評価も動いていない局面では積が0.33前後まで
下がり、荒れている局面では3倍を超える。ADR-0058の固定0.6は、この
分布の中央付近を定数で近似したものにすぎない。よって0058は不採択と
し、論点を本ADRへ統合した。

早期打ち切りの設計で難しいのは、読み抜けとの両立である。最善手が
安定して見えても、読めていないだけの可能性がある。評価の下落し続けて
いる局面では、次のイテレーションで最善手がひっくり返りやすい。
縮める指標と伸ばす安全弁を対で持たなければ機能しない。係数の積は
まさにその構造になっている。

## 選択肢と比較

### 案A: 最善手の連続不変回数だけでスケールする

状態はカウンタ1個で済む。ただし安全弁がない。評価が下落中でも、
最善手が変わっていなければ打ち切る。読み抜けが進行している局面を
すり抜ける。

### 案B: 縮める指標と伸ばす安全弁を係数の積で合成する

Stockfish・やねうら王と同じ構造を採る。判定は比率1.0で、optimumを
係数の積で伸縮させる。ADR-0062のroot手ごとノード数集計が前提になる。

### 案C: 1位と2位のスコア差で判定する

MultiPVを内部的に2に上げ、スコア差が大きければ簡単と見なす。
常時MultiPV=2で探索するとノード数が増え、時間短縮の利得を打ち消す。

## Decision

案Bを採用する。

当初は案Aから始める設計にしていた。ADR-0055でLMRの条件項を2項同時に
等重みで入れて不採択になった教訓を適用したためである。これは誤った
適用だった。ADR-0055の2項は独立した調整項で、分離して測る意味が
あった。本ADRの係数は、安全弁を含む1つの条件セットである。分離して
SPRTを回すと、H0採択が「機能自体の否定」なのか「安全弁の欠如による
事故」なのか切り分けられない。

Stockfishの4係数のうち、`reduction`（最善手が最後に変わった深さからの
経過）と `bestMoveInstability`（最善手の変化回数）は、どちらも最善手の
揺れを測るものである。Himawariでは1つの連続量にまとめ、係数を3つに
する。定数の数を抑えるためで、値域は両者を合わせた範囲に取る。

### 実装スケッチ

#### 係数の定義

```rust
// 評価の下落（伸ばす）
const FALLING_UNIT: f64 = 200.0;
const FALLING_MIN: f64 = 0.6;
const FALLING_MAX: f64 = 1.7;
// 最善手の揺れ（変わった直後は伸ばし、長く不変なら縮める）
const STABILITY_BASE: f64 = 1.5;
const STABILITY_STEP: f64 = 0.15;
const STABILITY_MIN: f64 = 0.75;
// ノードの集中（常に縮める方向。集中しているほど強く）
const EFFORT_LO: f64 = 0.75;
const EFFORT_HI: f64 = 1.0;
const EFFORT_SCALE_LO: f64 = 0.85;
const EFFORT_SCALE_HI: f64 = 0.70;
// 適用の下限深さ
const SCALE_MIN_DEPTH: u32 = 8;
```

#### 反復深化ループでの合成

ループの外で状態を持つ。

```rust
let mut prev_best = Move::NONE;
let mut prev_iter_score = VALUE_ZERO;
let mut stable_iters: u32 = 0;
```

イテレーション完了時、既存の打ち切り判定（search.rs:319）の位置で
更新する。

```rust
let cur_best = self.root_moves[0].mv;
stable_iters = if cur_best == prev_best { stable_iters + 1 } else { 0 };
prev_best = cur_best;

let scale = if self.multi_pv == 1 && depth >= SCALE_MIN_DEPTH {
    // 評価が下がっているほど伸ばす
    let drop = f64::from(prev_iter_score - last_score);
    let falling = (1.0 + drop / FALLING_UNIT).clamp(FALLING_MIN, FALLING_MAX);

    // 最善手が変わった直後は伸ばし、連続で不変なら縮める
    let stability = (STABILITY_BASE - STABILITY_STEP * f64::from(stable_iters))
        .clamp(STABILITY_MIN, STABILITY_BASE);

    // 最善手にノードが集中しているほど縮める（ADR-0062）
    let total: u64 = self.root_moves.iter().map(|rm| rm.nodes).sum();
    let ratio = self.root_moves[0].nodes as f64 / total.max(1) as f64;
    let t = ((ratio - EFFORT_LO) / (EFFORT_HI - EFFORT_LO)).clamp(0.0, 1.0);
    let effort = EFFORT_SCALE_LO + (EFFORT_SCALE_HI - EFFORT_SCALE_LO) * t;

    falling * stability * effort
} else {
    1.0
};
prev_iter_score = last_score;

if self.stopped() || self.tm.over_total(scale) {
    break;
}
```

#### TimeManagerへの追加

optimumを係数倍した値と、maximumの小さいほうを閾値にする。

```rust
/// optimumのscale倍とmaximumの小さいほうを超えたか
#[inline]
pub fn over_total(&self, scale: f64) -> bool {
    let Some(opt) = self.optimum else { return false };
    let mut t = opt.as_secs_f64() * scale;
    if let Some(m) = self.maximum {
        t = t.min(m.as_secs_f64());
    }
    self.elapsed().as_secs_f64() >= t
}
```

ヘルパースレッドとponder探索はtmが無制限（thread.rs:169〜186）で
optimumがNoneなので、常にfalseになる。挙動は変わらない。

#### 係数の値域

初期定数（チューニングしない）はStockfish・やねうら王の値域に
合わせた。積の範囲は次のようになる。

| 局面 | falling | stability | effort | 積 |
|---|---|---|---|---|
| 安定・評価横ばい・ノード集中 | 1.0 | 0.75 | 0.70 | 0.53 |
| 安定・評価横ばい・分散 | 1.0 | 0.75 | 0.85 | 0.64 |
| 最善手が変わった直後 | 1.0 | 1.5 | 0.85 | 1.28 |
| 評価が140cp下落・最善手も変化 | 1.7 | 1.5 | 0.85 | 2.17 |

上限2.17はmaximum（`avail × 3`）の内側に収まる。深さ8未満と
MultiPV>1では係数を適用せず、従来どおりoptimumで判定する。浅い
イテレーションでは最善手が偶然一致しやすく、安定と誤認するためで
ある。MultiPVは解析用途（ADR-0032）なので対象外にする。

### 検証

SPRTはADR-0028の既定条件（`--tc 10+0.1 --concurrency 8
--adjudicate 2000,8`、elo0=0、elo1=5、α=β=0.05）。両エンジンに
`--option "EvalFile=data/nets/halfkp_180M.hmwr.best"`。
ADR-0062を入れたあとのmainをベースラインにする。

既定条件（1手約0.2秒）では深さ8に届く手が少なく、SCALE_MIN_DEPTHの
ガードで発動しない可能性を懸念したが、実際には効果が出た。
`--tc 60+1` での追試は不要だった。

結果: **+69.3 Elo [+48.4,+90.6]**（1098局、549ペア、LLR 3.08でH1採択）。

SPRTとは別に、floodgate条件（300秒＋10秒加算）で1手の消費時間と到達
深さを実測した。

| 局面 | 適用前 | 適用後 | 深さ |
|---|---|---|---|
| 序盤（初手、残り300s） | 16.84s | 12.65s | 21→20 |
| 序盤（26手目、残り240s） | 21.43s | 11.61s | 24→23 |
| 中盤（残り200s） | 18.53s | 21.18s | 21→20 |
| 終盤（残り100s） | 16.61s | 15.17s | 14→**15** |
| 合計 | 73.41s | 60.61s | |

序盤で最大46%節約する一方、中盤では14%伸びている。一律に縮める実装
ではなく、安全弁が働いていることの確認になる。終盤は消費を9%減らし
ながら到達深さが1段深くなった。合計17%の節約である。

時間制限のない探索（`go depth N`、ヘルパースレッド、ponder）は
optimumがNoneのため影響を受けない。深さ14固定の3局面でノード数が
31,550,453と完全一致することを確認した。

## Consequences

- 簡単な局面ではoptimumの0.53〜0.64で指し、荒れた局面では2倍以上
  伸びる。浮いた時間が終盤に回る一方、危険な局面では従来より長く
  読む。floodgateの実測（平均21.1秒、optimumの1.3倍）は再測定する
- ADR-0062（root手ごとのノード数集計）が前提になる。先に入れる
- 定数が9個に増える。すべて初期値のままSPRTに掛け、採択後も
  チューニングしない（CLAUDE.mdの運用）。H0採択の場合は、係数を
  1つずつ1.0に固定して切り分ける
- Stockfishの `bestPreviousAverageScore`（前の手の探索結果）は
  取り込まない。探索間で状態を持つ必要があり、thread.rsの変更が
  必要になる。評価下落の判定は同一探索内の前イテレーションとの
  比較だけで行う。前の手からの急落は捉えられない
- ノード集中度はメインスレッドのローカルノード数で測る（ADR-0062）。
  マルチスレッド時は全体の配分と一致しない
- 早期打ち切りにより、GUI表示の探索深さが局面によって浅くなる
