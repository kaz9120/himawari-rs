//! Bitboardレイアウト候補のマイクロベンチマーク（ADRの判断材料）。
//!
//! 候補1: u128 単一ワード
//! 候補2: [u64; 2] 相当（lo = 1〜7筋の63マス、hi = 8〜9筋の18マス）
//!
//! 計測対象は盤面表現で頻出する6操作。段方向シフト（歩の利き）、
//! 筋方向シフト、popcount、ビット走査、Qugiy式の香の利き（増加方向）。
//! ローカルで `RUSTFLAGS="-C target-cpu=native" cargo bench -p himawari-core`
//! を実行して計測する（ADR-0003）。

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};

const BOARD: u128 = (1u128 << 81) - 1;

const fn rank_mask_u128(rank: u32) -> u128 {
    let mut m = 0u128;
    let mut f = 0;
    while f < 9 {
        m |= 1u128 << (f * 9 + rank);
        f += 1;
    }
    m
}

const RANK1: u128 = rank_mask_u128(0);
const RANK9: u128 = rank_mask_u128(8);
const DOWN_OK: u128 = BOARD & !RANK1;
const UP_OK: u128 = BOARD & !RANK9;

const LO_MASK: u64 = (1u64 << 63) - 1;
const HI_MASK: u64 = (1u64 << 18) - 1;
const RANK1_LO: u64 = (RANK1 & (LO_MASK as u128)) as u64;
const RANK9_LO: u64 = (RANK9 & (LO_MASK as u128)) as u64;
const RANK1_HI: u64 = (RANK1 >> 63) as u64;
const RANK9_HI: u64 = (RANK9 >> 63) as u64;
const DOWN_OK_LO: u64 = LO_MASK & !RANK1_LO;
const DOWN_OK_HI: u64 = HI_MASK & !RANK1_HI;
const UP_OK_LO: u64 = LO_MASK & !RANK9_LO;
const UP_OK_HI: u64 = HI_MASK & !RANK9_HI;

/// 候補1: u128 単一ワード。
#[derive(Copy, Clone, PartialEq, Eq)]
struct Bb1(u128);

impl Bb1 {
    fn shift_down(self) -> Self {
        Self((self.0 << 1) & DOWN_OK)
    }

    fn shift_up(self) -> Self {
        Self((self.0 >> 1) & UP_OK)
    }

    fn shift_left(self) -> Self {
        Self((self.0 << 9) & BOARD)
    }

    fn popcount(self) -> u32 {
        self.0.count_ones()
    }

    fn scan_sum(self) -> u32 {
        let mut v = self.0;
        let mut s = 0;
        while v != 0 {
            s += v.trailing_zeros();
            v &= v.wrapping_sub(1);
        }
        s
    }

    /// u128表現のまま、走査だけを64bitワード2本に分割して行う折衷実装。
    fn scan_sum_split(self) -> u32 {
        let mut s = 0;
        let mut v = self.0 as u64;
        while v != 0 {
            s += v.trailing_zeros();
            v &= v.wrapping_sub(1);
        }
        let mut v = (self.0 >> 64) as u64;
        while v != 0 {
            s += 64 + v.trailing_zeros();
            v &= v.wrapping_sub(1);
        }
        s
    }

    /// 増加方向（後手の香）の利き。Qugiy式（桁借りで最初の駒まで）。
    fn lance_attack(self, mask: u128) -> Self {
        let t = self.0 & mask;
        Self((t ^ t.wrapping_sub(1)) & mask)
    }
}

/// 候補2: 2ワード分割。lo = 1〜7筋（bit 0..=62）、hi = 8〜9筋（bit 0..=17）。
#[derive(Copy, Clone, PartialEq, Eq)]
struct Bb2 {
    lo: u64,
    hi: u64,
}

impl Bb2 {
    fn from_u128(v: u128) -> Self {
        Self {
            lo: (v as u64) & LO_MASK,
            hi: (v >> 63) as u64,
        }
    }

    fn to_u128(self) -> u128 {
        u128::from(self.lo) | (u128::from(self.hi) << 63)
    }

    fn shift_down(self) -> Self {
        // 筋がワードをまたがないため、桁上がりの受け渡しが不要
        Self {
            lo: (self.lo << 1) & DOWN_OK_LO,
            hi: (self.hi << 1) & DOWN_OK_HI,
        }
    }

    fn shift_up(self) -> Self {
        Self {
            lo: (self.lo >> 1) & UP_OK_LO,
            hi: (self.hi >> 1) & UP_OK_HI,
        }
    }

    fn shift_left(self) -> Self {
        // 7筋（bit 54..=62）が8筋（hiのbit 0..=8）へ移る
        Self {
            lo: (self.lo << 9) & LO_MASK,
            hi: ((self.hi << 9) | (self.lo >> 54)) & HI_MASK,
        }
    }

    fn popcount(self) -> u32 {
        self.lo.count_ones() + self.hi.count_ones()
    }

    fn scan_sum(self) -> u32 {
        let mut s = 0;
        let mut v = self.lo;
        while v != 0 {
            s += v.trailing_zeros();
            v &= v.wrapping_sub(1);
        }
        let mut v = self.hi;
        while v != 0 {
            s += 63 + v.trailing_zeros();
            v &= v.wrapping_sub(1);
        }
        s
    }

    fn lance_attack(self, mask: u128) -> Self {
        // 筋マスクはワードをまたがないため、片側のワードだけ処理する
        if mask & (LO_MASK as u128) != 0 {
            let m = mask as u64;
            let t = self.lo & m;
            Self {
                lo: (t ^ t.wrapping_sub(1)) & m,
                hi: 0,
            }
        } else {
            let m = (mask >> 63) as u64;
            let t = self.hi & m;
            Self {
                lo: 0,
                hi: (t ^ t.wrapping_sub(1)) & m,
            }
        }
    }
}

struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn next_u128(&mut self) -> u128 {
        u128::from(self.next()) | (u128::from(self.next()) << 64)
    }
}

/// sqの1つ先から筋の端までのマスク（増加方向）。
fn lance_masks() -> Vec<u128> {
    (0..81u32)
        .map(|sq| {
            let file = sq / 9;
            let rank = sq % 9;
            let mut m = 0u128;
            for r in (rank + 1)..9 {
                m |= 1u128 << (file * 9 + r);
            }
            m
        })
        .collect()
}

struct Data {
    boards: Vec<u128>,
    occs: Vec<u128>,
    sqs: Vec<usize>,
    masks: Vec<u128>,
}

fn make_data() -> Data {
    let mut rng = Xorshift(0x9E37_79B9_7F4A_7C15);
    let n = 1024;
    let boards: Vec<u128> = (0..n)
        .map(|_| rng.next_u128() & rng.next_u128() & BOARD)
        .collect();
    let occs: Vec<u128> = (0..n).map(|_| rng.next_u128() & BOARD).collect();
    let sqs: Vec<usize> = (0..n).map(|_| (rng.next() % 81) as usize).collect();
    Data {
        boards,
        occs,
        sqs,
        masks: lance_masks(),
    }
}

/// 2実装が同じ結果を返すことを計測前に確認する。
fn verify(data: &Data) {
    for (i, &b) in data.boards.iter().enumerate() {
        let b1 = Bb1(b);
        let b2 = Bb2::from_u128(b);
        assert_eq!(b1.shift_down().0, b2.shift_down().to_u128());
        assert_eq!(b1.shift_up().0, b2.shift_up().to_u128());
        assert_eq!(b1.shift_left().0, b2.shift_left().to_u128());
        assert_eq!(b1.popcount(), b2.popcount());
        assert_eq!(b1.scan_sum(), b2.scan_sum());
        assert_eq!(b1.scan_sum(), b1.scan_sum_split());
        let occ = data.occs[i];
        let mask = data.masks[data.sqs[i]];
        assert_eq!(
            Bb1(occ).lance_attack(mask).0,
            Bb2::from_u128(occ).lance_attack(mask).to_u128()
        );
    }
}

fn bench_layout(c: &mut Criterion) {
    let data = make_data();
    verify(&data);

    let boards1: Vec<Bb1> = data.boards.iter().map(|&b| Bb1(b)).collect();
    let boards2: Vec<Bb2> = data.boards.iter().map(|&b| Bb2::from_u128(b)).collect();
    let occs1: Vec<Bb1> = data.occs.iter().map(|&b| Bb1(b)).collect();
    let occs2: Vec<Bb2> = data.occs.iter().map(|&b| Bb2::from_u128(b)).collect();

    let mut g = c.benchmark_group("bb_layout");
    g.warm_up_time(std::time::Duration::from_millis(500));
    g.measurement_time(std::time::Duration::from_millis(1500));

    g.bench_function("u128/shift_down", |b| {
        b.iter(|| {
            let mut acc = 0u128;
            for &bb in black_box(&boards1) {
                acc ^= bb.shift_down().0;
            }
            acc
        });
    });
    g.bench_function("u64x2/shift_down", |b| {
        b.iter(|| {
            let mut acc = (0u64, 0u64);
            for &bb in black_box(&boards2) {
                let r = bb.shift_down();
                acc = (acc.0 ^ r.lo, acc.1 ^ r.hi);
            }
            acc
        });
    });

    g.bench_function("u128/shift_up", |b| {
        b.iter(|| {
            let mut acc = 0u128;
            for &bb in black_box(&boards1) {
                acc ^= bb.shift_up().0;
            }
            acc
        });
    });
    g.bench_function("u64x2/shift_up", |b| {
        b.iter(|| {
            let mut acc = (0u64, 0u64);
            for &bb in black_box(&boards2) {
                let r = bb.shift_up();
                acc = (acc.0 ^ r.lo, acc.1 ^ r.hi);
            }
            acc
        });
    });

    g.bench_function("u128/shift_left", |b| {
        b.iter(|| {
            let mut acc = 0u128;
            for &bb in black_box(&boards1) {
                acc ^= bb.shift_left().0;
            }
            acc
        });
    });
    g.bench_function("u64x2/shift_left", |b| {
        b.iter(|| {
            let mut acc = (0u64, 0u64);
            for &bb in black_box(&boards2) {
                let r = bb.shift_left();
                acc = (acc.0 ^ r.lo, acc.1 ^ r.hi);
            }
            acc
        });
    });

    g.bench_function("u128/popcount", |b| {
        b.iter(|| {
            let mut s = 0u32;
            for &bb in black_box(&boards1) {
                s = s.wrapping_add(bb.popcount());
            }
            s
        });
    });
    g.bench_function("u64x2/popcount", |b| {
        b.iter(|| {
            let mut s = 0u32;
            for &bb in black_box(&boards2) {
                s = s.wrapping_add(bb.popcount());
            }
            s
        });
    });

    g.bench_function("u128/scan", |b| {
        b.iter(|| {
            let mut s = 0u32;
            for &bb in black_box(&boards1) {
                s = s.wrapping_add(bb.scan_sum());
            }
            s
        });
    });
    g.bench_function("u128/scan_split", |b| {
        b.iter(|| {
            let mut s = 0u32;
            for &bb in black_box(&boards1) {
                s = s.wrapping_add(bb.scan_sum_split());
            }
            s
        });
    });
    g.bench_function("u64x2/scan", |b| {
        b.iter(|| {
            let mut s = 0u32;
            for &bb in black_box(&boards2) {
                s = s.wrapping_add(bb.scan_sum());
            }
            s
        });
    });

    g.bench_function("u128/lance", |b| {
        b.iter(|| {
            let mut acc = 0u128;
            for (i, &occ) in black_box(&occs1).iter().enumerate() {
                acc ^= occ.lance_attack(data.masks[data.sqs[i]]).0;
            }
            acc
        });
    });
    g.bench_function("u64x2/lance", |b| {
        b.iter(|| {
            let mut acc = (0u64, 0u64);
            for (i, &occ) in black_box(&occs2).iter().enumerate() {
                let r = occ.lance_attack(data.masks[data.sqs[i]]);
                acc = (acc.0 ^ r.lo, acc.1 ^ r.hi);
            }
            acc
        });
    });

    g.finish();
}

criterion_group!(benches, bench_layout);
criterion_main!(benches);
