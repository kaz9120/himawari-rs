//! ロックレス置換表（ADR-0022）。
//!
//! エントリはAtomicU64×2のHyatt式XOR自己検証。word1にデータ、
//! word0に `key ^ word1` を置き、読み出し時の照合でtornエントリを弾く。
//! すべてRelaxedで、データレースは構造的に存在しない。

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use himawari_core::Move16;

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
        // 同一キーがあれば上書き、なければ「depth − 世代差ペナルティ」最小を置換
        let mut victim = 0;
        let mut victim_score = i32::MAX;
        for (i, e) in cluster.0.iter().enumerate() {
            let w0 = e.word0.load(Ordering::Relaxed);
            let w1 = e.word1.load(Ordering::Relaxed);
            if w1 == 0 || w0 ^ w1 == key {
                victim = i;
                break;
            }
            let (data, entry_gen) = unpack(w1);
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
