//! 飛角香の利き生成方式のマイクロベンチマーク（ADRの判断材料）。
//!
//! Qugiy系（テーブル不要・分岐レス）を実装し、素朴なマス走査と比較する。
//! 増加方向のレイは減算の桁借り、減少方向のレイはMSB切り詰め（clz）で
//! 最初の駒までの利きを求める。レイマスクはビット順が単調なら
//! マスの間隔（±1/±8/±9/±10）によらず同じ式で書ける。
//!
//! ローカルで `RUSTFLAGS="-C target-cpu=native" cargo bench -p himawari-core`
//! を実行して計測する（ADR-0003）。

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};

const BOARD: u128 = (1u128 << 81) - 1;

/// 8方向。(筋の増分, 段の増分)。前半4つがビット増加方向、後半4つが減少方向。
const DIRS: [(i32, i32); 8] = [
    (0, 1),   // 下 (+1)
    (1, -1),  // 左上 (+8)
    (1, 0),   // 左 (+9)
    (1, 1),   // 左下 (+10)
    (0, -1),  // 上 (−1)
    (-1, 1),  // 右下 (−8)
    (-1, 0),  // 右 (−9)
    (-1, -1), // 右上 (−10)
];

/// sqからdir方向へ、盤端まで（sq自身を除く）のレイマスク。
fn ray_mask(sq: u32, dir: (i32, i32)) -> u128 {
    let mut m = 0u128;
    let mut f = (sq / 9) as i32 + dir.0;
    let mut r = (sq % 9) as i32 + dir.1;
    while (0..9).contains(&f) && (0..9).contains(&r) {
        m |= 1u128 << (f * 9 + r);
        f += dir.0;
        r += dir.1;
    }
    m
}

fn ray_masks() -> Vec<[u128; 8]> {
    (0..81u32)
        .map(|sq| std::array::from_fn(|d| ray_mask(sq, DIRS[d])))
        .collect()
}

/// 増加方向のレイ。桁借りで最初の駒まで（駒を含む）。
fn ray_inc(occ: u128, mask: u128) -> u128 {
    let t = occ & mask;
    (t ^ t.wrapping_sub(1)) & mask
}

/// 減少方向のレイ。最初の駒（=maskとoccの積のMSB）以上のビットを残す。
/// 駒がなければmask全体。`t | 1` はclzの引数を非ゼロにするための番兵で、
/// 駒がbit 0にある場合も結果は変わらない。
fn ray_dec(occ: u128, mask: u128) -> u128 {
    let t = occ & mask;
    mask & (u128::MAX << (127 - (t | 1).leading_zeros()))
}

fn rook_qugiy(occ: u128, rays: &[u128; 8]) -> u128 {
    ray_inc(occ, rays[0]) | ray_inc(occ, rays[2]) | ray_dec(occ, rays[4]) | ray_dec(occ, rays[6])
}

fn bishop_qugiy(occ: u128, rays: &[u128; 8]) -> u128 {
    ray_inc(occ, rays[1]) | ray_inc(occ, rays[3]) | ray_dec(occ, rays[5]) | ray_dec(occ, rays[7])
}

/// 先手の香（上方向 = 減少方向）。
fn lance_qugiy(occ: u128, rays: &[u128; 8]) -> u128 {
    ray_dec(occ, rays[4])
}

/// 素朴なマス走査による基準実装。
fn ray_naive(occ: u128, sq: u32, dir: (i32, i32)) -> u128 {
    let mut m = 0u128;
    let mut f = (sq / 9) as i32 + dir.0;
    let mut r = (sq % 9) as i32 + dir.1;
    while (0..9).contains(&f) && (0..9).contains(&r) {
        let bit = 1u128 << (f * 9 + r);
        m |= bit;
        if occ & bit != 0 {
            break;
        }
        f += dir.0;
        r += dir.1;
    }
    m
}

fn rook_naive(occ: u128, sq: u32) -> u128 {
    [DIRS[0], DIRS[2], DIRS[4], DIRS[6]]
        .iter()
        .fold(0, |a, &d| a | ray_naive(occ, sq, d))
}

fn bishop_naive(occ: u128, sq: u32) -> u128 {
    [DIRS[1], DIRS[3], DIRS[5], DIRS[7]]
        .iter()
        .fold(0, |a, &d| a | ray_naive(occ, sq, d))
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

fn bench_effect(c: &mut Criterion) {
    let masks = ray_masks();
    let mut rng = Xorshift(0x243F_6A88_85A3_08D3);
    let n = 1024;
    let occs: Vec<u128> = (0..n).map(|_| rng.next_u128() & BOARD).collect();
    let sqs: Vec<usize> = (0..n).map(|_| (rng.next() % 81) as usize).collect();

    // 全マス×全occで素朴実装との一致を確認してから計測する
    for &occ in occs.iter().take(64) {
        for sq in 0..81u32 {
            let rays = &masks[sq as usize];
            assert_eq!(rook_qugiy(occ, rays), rook_naive(occ, sq));
            assert_eq!(bishop_qugiy(occ, rays), bishop_naive(occ, sq));
            assert_eq!(lance_qugiy(occ, rays), ray_naive(occ, sq, DIRS[4]));
        }
    }

    let mut g = c.benchmark_group("effect_gen");
    g.warm_up_time(std::time::Duration::from_millis(500));
    g.measurement_time(std::time::Duration::from_millis(1500));

    g.bench_function("rook/qugiy", |b| {
        b.iter(|| {
            let mut acc = 0u128;
            for (i, &occ) in black_box(&occs).iter().enumerate() {
                acc ^= rook_qugiy(occ, &masks[sqs[i]]);
            }
            acc
        });
    });
    g.bench_function("rook/naive", |b| {
        b.iter(|| {
            let mut acc = 0u128;
            for (i, &occ) in black_box(&occs).iter().enumerate() {
                acc ^= rook_naive(occ, sqs[i] as u32);
            }
            acc
        });
    });

    g.bench_function("bishop/qugiy", |b| {
        b.iter(|| {
            let mut acc = 0u128;
            for (i, &occ) in black_box(&occs).iter().enumerate() {
                acc ^= bishop_qugiy(occ, &masks[sqs[i]]);
            }
            acc
        });
    });
    g.bench_function("bishop/naive", |b| {
        b.iter(|| {
            let mut acc = 0u128;
            for (i, &occ) in black_box(&occs).iter().enumerate() {
                acc ^= bishop_naive(occ, sqs[i] as u32);
            }
            acc
        });
    });

    g.bench_function("lance/qugiy", |b| {
        b.iter(|| {
            let mut acc = 0u128;
            for (i, &occ) in black_box(&occs).iter().enumerate() {
                acc ^= lance_qugiy(occ, &masks[sqs[i]]);
            }
            acc
        });
    });

    g.finish();
}

criterion_group!(benches, bench_effect);
criterion_main!(benches);
