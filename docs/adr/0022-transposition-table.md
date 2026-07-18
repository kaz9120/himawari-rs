# 0022: ロックレス置換表

- Status: accepted
- Date: 2026-07-18
- 関連ADR: [0004](0004-unsafe-policy.md), [0015](0015-zobrist-hash.md), [0020](0020-search-threading.md)

## Context

置換表はLazy SMPで全スレッドが共有し、ロックなしで読み書きする。
Stockfishの「データレースを許容し壊れたエントリを検出で弾く」流儀は、
C++でも未定義動作すれすれであり、Rustでは明確にUB（コンパイラが
レース不在を前提に最適化する）。atomicだけで組む必要がある。
エントリ幅・クラスタ構成・世代管理・置換方針もここで決める。

## 選択肢と比較

### 案A: エントリ1ワード（AtomicU64 1個）

key16 | move16 | value16 | depth8 | 世代・bound8 で64bitに収まる。
1ワードのatomic読み書きは破損（torn read/write）が原理的に
起きず、検証機構が不要で最速。ただしstatic eval（評価値の
キャッシュ）を持てない。NNUE（P4）では葉のeval再計算が高価で、
evalキャッシュの有無が実効NPSに効く。

### 案B: エントリ2ワード（AtomicU64×2、XOR自己検証）

word1にデータ（move16 | value16 | eval16 | depth8 | 世代5+bound2+PV1）、
word0に `key64 ^ word1` を置く（Hyatt方式）。読みは2ワードを
Relaxedでloadし、`word0 ^ word1 == key64` で整合を検証する。
別スレッドの書き込みと交錯した（torn）エントリは検証で必ず弾ける。
key照合が64bit精度になり、eval16も持てる。書き込みは2 store、
読みは2 load＋XOR比較で、案Aとの差はわずか。

## Decision

案Bを採用する。決め手はevalキャッシュ（P4のNNUEで効く）と
key64精度で、追加コストが2命令程度に収まることだ。定義は次のとおり。

### エントリとクラスタ

```
Entry  = [AtomicU64; 2]                       // 16バイト
  word0 = key ^ word1
  word1 = move16 | value16 | eval16 | depth8 | gen5+bound2+pv1
Cluster = #[repr(align(64))] [Entry; 4]        // 64バイト = 1キャッシュライン
```

- キーはPosition::key()（board_key ^ 拡散hand_key。ADR-0015）
- クラスタ添字は `(key × クラスタ数) >> 64` の乗算写像
  （2冪制約なしでサイズを自由に選べる）
- 本体は `Box<[Cluster]>`。`&TT` を全スレッドに配る（ADR-0020）。
  確保・クリアはisready時（ADR-0019）。クリアはスレッド並列のmemset

### 読み書き

- probe: クラスタ4エントリをRelaxed loadし、XOR検証つきで
  key一致を探す。一致すればhit
- store: 置換対象を選び、word1→word0の順にRelaxed storeする
  （順序はどちらでもよい。torn状態は検証で弾かれる）
- 置換方針: 同一keyは常に上書き（深い方優先で内容マージはしない）。
  空きがなければ `depth − 世代差ペナルティ` が最小のエントリを置換

### 世代管理

- 5bitの世代カウンタをTT本体が持ち、`go` ごとに+1する
- 探索スコアのply補正（詰みスコア）はTT境界で行う（探索ADRで詳細化）

## Consequences

- データレースが構造的に存在せず、Miri/TSan（ADR-0004）にかけられる。
  「壊れた値を読んで誤動作」の類のバグがクラス単位で消える
- torn writeされたエントリは検証失敗＝missとして扱われ、
  情報が失われるだけで正しさに影響しない
- ハッシュ分布の確認（クラスタ占有率、ADR-0015のテストポイント）を
  hashfull計数として実装し、USIのinfo hashfullにも流用する
- エントリ16B×4/クラスタのため、同一クラスタに入る局面数は
  Stockfish（3エントリ）より1多い。置換方針の優劣はP3のSPRTで
  検証可能になってから調整する
