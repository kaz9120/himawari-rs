//! ロックレス置換表（ADR-0022）。
//!
//! エントリはAtomicU64×2のHyatt式XOR自己検証。word1にデータ、
//! word0に `key ^ word1` を置き、読み出し時の照合でtornエントリを弾く。
//! すべてRelaxedで、データレースは構造的に存在しない。

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use himawari_core::Move16;

use crate::value::Value;

/// eval hashのエントリ数（既定2^23 = 約840万、AtomicU64で64MB）。
/// USIオプションは設けない（ADR-0049）。小メモリの的に向けては
/// ビルド時に `HIMAWARI_EVAL_HASH_BITS` で絞る（issue #429）。
const EVAL_HASH_BITS: usize = crate::nnue::EVAL_HASH_BITS;
const EVAL_HASH_SIZE: usize = 1 << EVAL_HASH_BITS;

/// 評価値キャッシュ（ADR-0049）。局面キー→生評価のロックレス共有表。
///
/// エントリは `(key上位32bit << 32) | (eval as u32)`。probeは上位32bit
/// 一致で採用する。偽ヒット確率はprobeあたり2^-32で、NNUE評価の±1違い
/// と同水準として無視する（やねうら王系と同じ割り切り）。
/// 生値のみを持ち、correction history補正（ADR-0046）はキャッシュの外側。
pub struct EvalHash {
    table: Vec<AtomicU64>,
}

impl EvalHash {
    pub fn new() -> EvalHash {
        // vec![]はAtomicがCloneでないため使えない。1本ずつ確保する
        EvalHash {
            table: (0..EVAL_HASH_SIZE).map(|_| AtomicU64::new(0)).collect(),
        }
    }

    /// 検証用の無効化インスタンス（ADR-0049の機能検証）。probeは常にミス、
    /// storeは無視する。eval hash有無で探索が変わらないことの対照に使う。
    #[cfg(test)]
    pub fn disabled() -> EvalHash {
        EvalHash { table: Vec::new() }
    }

    /// key下位23bitのスロットを引き、上位32bit一致なら下位32bitを評価値
    /// （i32）として返す。0エントリは通常キー不一致で弾かれる。
    #[inline]
    pub fn probe(&self, key: u64) -> Option<Value> {
        if self.table.is_empty() {
            return None;
        }
        let slot = key as usize & (EVAL_HASH_SIZE - 1);
        let entry = self.table[slot].load(Ordering::Relaxed);
        if entry >> 32 == key >> 32 {
            Some(entry as u32 as i32)
        } else {
            None
        }
    }

    /// 生評価をkey下位23bitのスロットへ格納する（上書き）。
    #[inline]
    pub fn store(&self, key: u64, eval: Value) {
        if self.table.is_empty() {
            return;
        }
        let slot = key as usize & (EVAL_HASH_SIZE - 1);
        let entry = (key >> 32) << 32 | u64::from(eval as u32);
        self.table[slot].store(entry, Ordering::Relaxed);
    }

    /// 全エントリの消去（usinewgameで呼ぶ）。対局間の独立性を保つ。
    pub fn clear(&self) {
        for e in &self.table {
            e.store(0, Ordering::Relaxed);
        }
    }
}

impl Default for EvalHash {
    fn default() -> Self {
        EvalHash::new()
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Bound {
    None = 0,
    Upper = 1,
    Lower = 2,
    Exact = 3,
}

#[derive(Copy, Clone, Debug)]
pub struct TtData {
    pub mv: Move16,
    pub value: i16,
    pub eval: i16,
    pub depth: u8,
    pub bound: Bound,
    pub pv: bool,
}

#[derive(Default)]
struct Entry {
    word0: AtomicU64,
    word1: AtomicU64,
}

/// 64バイト = 1キャッシュライン。
#[repr(align(64))]
#[derive(Default)]
struct Cluster([Entry; 4]);

pub struct Tt {
    clusters: Box<[Cluster]>,
    generation: AtomicU8,
}

/// word1のレイアウト:
/// bit 0..16 move16 | 16..32 value | 32..48 eval | 48..56 depth |
/// bit 56..61 世代 | 61..63 bound | 63 pv
fn pack(
    mv: Move16,
    value: i16,
    eval: i16,
    depth: u8,
    generation: u8,
    bound: Bound,
    pv: bool,
) -> u64 {
    u64::from(mv.0)
        | (u64::from(value as u16) << 16)
        | (u64::from(eval as u16) << 32)
        | (u64::from(depth) << 48)
        | (u64::from(generation & 31) << 56)
        | (u64::from(bound as u8) << 61)
        | (u64::from(pv) << 63)
}

fn unpack(w: u64) -> (TtData, u8) {
    let bound = match (w >> 61) & 3 {
        1 => Bound::Upper,
        2 => Bound::Lower,
        3 => Bound::Exact,
        _ => Bound::None,
    };
    (
        TtData {
            mv: Move16((w & 0xFFFF) as u16),
            value: (w >> 16) as u16 as i16,
            eval: (w >> 32) as u16 as i16,
            depth: (w >> 48) as u8,
            bound,
            pv: (w >> 63) != 0,
        },
        ((w >> 56) & 31) as u8,
    )
}

impl Tt {
    pub fn new(mb: usize) -> Tt {
        let clusters = (mb.max(1) * 1024 * 1024 / std::mem::size_of::<Cluster>()).max(1);
        let mut v = Vec::with_capacity(clusters);
        v.resize_with(clusters, Cluster::default);
        Tt {
            clusters: v.into_boxed_slice(),
            generation: AtomicU8::new(0),
        }
    }

    /// 新しい探索の開始（goごとに呼ぶ）。
    pub fn new_search(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// 全エントリの消去（usinewgameで呼ぶ）。対局間の独立性を保つ。
    pub fn clear(&self) {
        for cluster in &self.clusters {
            for e in &cluster.0 {
                e.word0.store(0, Ordering::Relaxed);
                e.word1.store(0, Ordering::Relaxed);
            }
        }
        self.generation.store(0, Ordering::Relaxed);
    }

    #[inline]
    fn generation(&self) -> u8 {
        self.generation.load(Ordering::Relaxed) & 31
    }

    #[inline]
    fn cluster_index(&self, key: u64) -> usize {
        ((u128::from(key) * self.clusters.len() as u128) >> 64) as usize
    }

    pub fn probe(&self, key: u64) -> Option<TtData> {
        let cluster = &self.clusters[self.cluster_index(key)];
        for e in &cluster.0 {
            let w0 = e.word0.load(Ordering::Relaxed);
            let w1 = e.word1.load(Ordering::Relaxed);
            if w1 != 0 && w0 ^ w1 == key {
                return Some(unpack(w1).0);
            }
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    pub fn store(
        &self,
        key: u64,
        mv: Move16,
        value: i16,
        eval: i16,
        depth: u8,
        bound: Bound,
        pv: bool,
    ) {
        let generation = self.generation();
        let cluster = &self.clusters[self.cluster_index(key)];
        // 空きがあればそこへ、なければ「depth − 世代差ペナルティ」最小を置換。
        // 同一キーはSF流の上書き判定を通す（ADR-0054）
        let mut mv = mv;
        let mut victim = 0;
        let mut victim_score = i32::MAX;
        for (i, e) in cluster.0.iter().enumerate() {
            let w0 = e.word0.load(Ordering::Relaxed);
            let w1 = e.word1.load(Ordering::Relaxed);
            if w1 == 0 {
                victim = i;
                break;
            }
            let (data, entry_gen) = unpack(w1);
            if w0 ^ w1 == key {
                // 同一キー。浅いエントリで深いエントリを潰さない（ADR-0054）。
                // Exact・深さ僅差（新+4≧既存）・世代違いのいずれかでのみ上書きする。
                // qsearchのdepth 0 storeがmain searchの深いエントリを消すのを防ぐ
                let overwrite = bound == Bound::Exact
                    || i32::from(depth) + 4 >= i32::from(data.depth)
                    || entry_gen != generation;
                if !overwrite {
                    return;
                }
                // 新しい手がなければ既存の手を温存する
                if mv == Move16::NONE {
                    mv = data.mv;
                }
                victim = i;
                break;
            }
            let age = i32::from((32 + generation - entry_gen) & 31);
            let score = i32::from(data.depth) - 8 * age;
            if score < victim_score {
                victim_score = score;
                victim = i;
            }
        }
        let w1 = pack(mv, value, eval, depth, generation, bound, pv);
        let e = &cluster.0[victim];
        e.word1.store(w1, Ordering::Relaxed);
        e.word0.store(key ^ w1, Ordering::Relaxed);
    }

    /// 現世代エントリの占有率（千分率）。先頭1000クラスタのサンプリング。
    pub fn hashfull(&self) -> usize {
        let generation = self.generation();
        let n = self.clusters.len().min(1000);
        let mut filled = 0;
        for cluster in &self.clusters[..n] {
            for e in &cluster.0 {
                let w1 = e.word1.load(Ordering::Relaxed);
                if w1 != 0 && unpack(w1).1 == generation {
                    filled += 1;
                }
            }
        }
        filled * 1000 / (n * 4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_probe_roundtrip() {
        let tt = Tt::new(1);
        let key = 0xDEAD_BEEF_1234_5678u64;
        assert!(tt.probe(key).is_none());
        tt.store(key, Move16(0x1234), -321, 456, 7, Bound::Lower, true);
        let d = tt.probe(key).unwrap();
        assert_eq!(d.mv, Move16(0x1234));
        assert_eq!(d.value, -321);
        assert_eq!(d.eval, 456);
        assert_eq!(d.depth, 7);
        assert_eq!(d.bound, Bound::Lower);
        assert!(d.pv);
        // 別キーはmiss
        assert!(tt.probe(key ^ 0xFFFF_0000).is_none());
    }

    #[test]
    fn same_key_overwrites() {
        let tt = Tt::new(1);
        let key = 42u64;
        tt.store(key, Move16(1), 10, 0, 3, Bound::Upper, false);
        tt.store(key, Move16(2), 20, 0, 5, Bound::Exact, false);
        let d = tt.probe(key).unwrap();
        assert_eq!(d.mv, Move16(2));
        assert_eq!(d.depth, 5);
    }

    #[test]
    fn shallow_store_keeps_deep_same_key_entry() {
        // 深いエントリ（depth 10）を、浅いqsearch風store（depth 0, Upper）で潰さない
        let tt = Tt::new(1);
        let key = 7u64;
        tt.store(key, Move16(0x11), 100, 0, 10, Bound::Lower, false);
        tt.store(key, Move16(0x22), -50, 0, 0, Bound::Upper, false);
        let d = tt.probe(key).unwrap();
        assert_eq!(d.depth, 10, "深いエントリが浅いstoreで消えた");
        assert_eq!(d.mv, Move16(0x11));
        assert_eq!(d.value, 100);
    }

    #[test]
    fn shallow_exact_store_overwrites_deep_entry() {
        // Exactは深さに関わらず上書きする（ADR-0054の条件a）
        let tt = Tt::new(1);
        let key = 8u64;
        tt.store(key, Move16(0x11), 100, 0, 10, Bound::Lower, false);
        tt.store(key, Move16(0x22), 42, 0, 0, Bound::Exact, false);
        let d = tt.probe(key).unwrap();
        assert_eq!(d.depth, 0);
        assert_eq!(d.bound, Bound::Exact);
        assert_eq!(d.mv, Move16(0x22));
    }

    #[test]
    fn overwrite_without_move_preserves_existing_move() {
        // 手なし（Move16::NONE）で上書きするとき、既存の手を温存する（条件b: 同depth）
        let tt = Tt::new(1);
        let key = 9u64;
        tt.store(key, Move16(0x33), 10, 0, 5, Bound::Lower, false);
        tt.store(key, Move16::NONE, 20, 0, 5, Bound::Upper, false);
        let d = tt.probe(key).unwrap();
        assert_eq!(d.mv, Move16(0x33), "既存の手が温存されていない");
        assert_eq!(d.value, 20);
        assert_eq!(d.bound, Bound::Upper);
    }

    #[test]
    fn stale_generation_entry_is_overwritten() {
        // 世代が違えば浅くても上書きする（条件c）
        let tt = Tt::new(1);
        let key = 10u64;
        tt.store(key, Move16(0x44), 100, 0, 10, Bound::Lower, false);
        tt.new_search();
        tt.store(key, Move16(0x55), -30, 0, 0, Bound::Upper, false);
        let d = tt.probe(key).unwrap();
        assert_eq!(d.depth, 0, "旧世代の深いエントリが更新されていない");
        assert_eq!(d.mv, Move16(0x55));
    }

    #[test]
    fn eval_hash_store_probe_roundtrip() {
        let eh = EvalHash::new();
        let key = 0xDEAD_BEEF_1234_5678u64;
        assert!(eh.probe(key).is_none());
        eh.store(key, -321);
        assert_eq!(eh.probe(key), Some(-321));
        // 正のスコアも往復する
        eh.store(key, 456);
        assert_eq!(eh.probe(key), Some(456));
        // 上位32bitが異なるキーはミス（下位23bitは同一）
        let other = key ^ (1u64 << 40);
        assert!(eh.probe(other).is_none());
    }

    #[test]
    fn eval_hash_clear_empties_table() {
        let eh = EvalHash::new();
        let key = 0x0102_0304_0506_0708u64;
        eh.store(key, 100);
        assert_eq!(eh.probe(key), Some(100));
        eh.clear();
        assert!(eh.probe(key).is_none());
    }

    #[test]
    fn eval_hash_disabled_never_hits() {
        let eh = EvalHash::disabled();
        let key = 0xABCD_1234_5678_9ABCu64;
        eh.store(key, 42);
        assert!(eh.probe(key).is_none());
    }

    #[test]
    fn generation_ages_out_old_entries() {
        let tt = Tt::new(1);
        // depth 5のエントリは、4世代経つとdepth 1の新エントリより置換優先度が低い
        tt.store(1, Move16(1), 0, 0, 5, Bound::Exact, false);
        for _ in 0..4 {
            tt.new_search();
        }
        let base = tt.cluster_index(1);
        let mut stored = 0;
        let mut k = 2u64;
        while stored < 4 {
            if tt.cluster_index(k) == base {
                tt.store(k, Move16(9), 0, 0, 1, Bound::Exact, false);
                stored += 1;
            }
            k += 1;
        }
        assert!(tt.probe(1).is_none(), "古い世代が追い出されていない");
    }
}
