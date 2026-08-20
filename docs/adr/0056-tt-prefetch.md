# 0056: TTのprefetchを導入する

- Status: rejected
- Date: 2026-07-24
- 関連ADR: [0022](0022-transposition-table.md), [0049](0049-eval-hash.md), [0054](0054-qsearch-tt.md)

## Context

探索改善キャンペーンの続行。ROADMAPの候補の「TTのbucket化・prefetch」
のうち、bucket化は4-way クラスタとしてADR-0022で実装済みである。
本ADRはprefetchの導入を扱う。

TTのprobeはメモリアクセスを伴い、キャッシュミス時のレイテンシ
（L2で10ns前後、L3で30ns以上）が探索のスループットを律速する。
ADR-0022の設計で1クラスタ=64B=1キャッシュラインに収まっているが、
probeの直前までアクセスしないため、キャッシュに載っていないことが多い。

SF系ではdo_moveの直後にprefetch命令を発行し、再帰呼び出し先で
probeに到達するまでの間にメモリフェッチを完了させる手法が
標準装備されている。qsearch TT（ADR-0054）の導入でTTアクセス
頻度が増えており、prefetchの恩恵はさらに大きい。

## 選択肢と比較

### 案A: 探索ループのdo_move直後にprefetch

search.rsのムーブループ内でdo_moveの直後、再帰探索の直前に
`tt.prefetch(pos.key())`を呼ぶ。再帰先の先頭でtt.probe()に
到達するまでに前処理（mate distance pruning等）が挟まり、
メモリフェッチの時間を確保できる。main searchとqsearchの
両方に挿入する。SF系の標準配置。

### 案B: MovePickerの手生成時にprefetch

手を生成した時点で次の局面キーを計算しprefetchする。
do_moveより早くフェッチが始まるが、枝刈りで試さない手の
prefetchが無駄になる。キー計算の追加コストもある。

### 案C: Position::do_move内でprefetch

do_moveのキー更新直後に呼び出しを埋め込む。変更箇所は
一箇所で済む。ただしPositionがTTを知る必要があり、依存関係は逆転する。

## Decision

案Aを採用する。

### 実装スケッチ

tt.rsにprefetchメソッドを追加する。1クラスタ=64B=1キャッシュ
ラインなので、1回のprefetchで4エントリすべてをカバーできる。

```rust
#[inline(always)]
pub fn prefetch(&self, key: u64) {
    let idx = self.cluster_index(key);
    let ptr = self.clusters.as_ptr().wrapping_add(idx) as *const u8;
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!("prfm pldl1keep, [{ptr}]", ptr = in(reg) ptr);
    }
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::x86_64::_mm_prefetch(
            ptr as *const i8,
            core::arch::x86_64::_MM_HINT_T0,
        );
    }
}
```

未対応アーキテクチャではどちらの`#[cfg]`にも合致せず、
メソッド本体が空になる。prefetchはCPUへのヒントであり、
空でも正しさに影響しない。

search.rsの挿入箇所（main search・qsearch両方のムーブループ）:

```rust
self.pos.do_move(mv, &gives_check);
self.shared.tt.prefetch(self.pos.key());
let value = -self.search(/* ... */);  // or -self.qsearch(/* ... */)
```

初期定数: なし（構造の追加のみ）。

### 検証

SPRTはADR-0028の既定条件。両エンジンに
`--option "EvalFile=data/nets/halfkp_180M.hmwr.best"`。
NPS改善率も別途計測する。

## Consequences

- L1ヒット率が上がってNPSは改善する。4-wayクラスタが
  1キャッシュラインに収まる設計（ADR-0022）との相乗効果がある
- prefetch命令はソフトウェアヒントであり、CPUが無視しても
  正しさに影響しない。棋力の劣化する経路がなく、失敗リスクは低い
- eval hash（ADR-0049）にも同じ手法で追加できる。本ADRで
  TT単体の効果を確認した後に検討する
- インラインアセンブリを使うため、対応アーキテクチャの追加時に
  `#[cfg]`分岐を増やす必要がある
