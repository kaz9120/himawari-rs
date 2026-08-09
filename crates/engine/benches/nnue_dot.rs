#![feature(portable_simd)]
//! NNUE隠れ層の内積の実装候補を比べる（ADR-0099の判断材料）。
//!
//! 候補1: 現行。i8とu8をi32へ広げて8レーンずつ積和
//! 候補2: レーン数だけ増やす（16レーン）。移植可能なまま幅を稼ぐ
//! 候補3: i16中間。積をi16で持ち、最後にi32へ畳む
//! 候補4: NEONのSDOT。1命令で16積和（aarch64のみ）
//!
//! 対象は forward_hidden の2層ぶん。第1層は32行×512列、第2層は
//! 32行×32列で、行ごとの内積を繰り返す。
//! ローカルで `RUSTFLAGS="-C target-cpu=native" cargo bench -p himawari-engine`
//! を実行して計測する（ADR-0003）。

use std::hint::black_box;
use std::simd::Simd;
use std::simd::num::{SimdInt, SimdUint};

use criterion::{Criterion, criterion_group, criterion_main};

const CONCAT: usize = 512;
const HIDDEN: usize = 32;

/// 候補1: 現行実装（nnue_simd::dot）。
fn dot_i32x8(w: &[i8], x: &[u8]) -> i32 {
    let mut acc = Simd::<i32, 8>::splat(0);
    for (wc, xc) in w.as_chunks::<8>().0.iter().zip(x.as_chunks::<8>().0) {
        let wv = Simd::<i8, 8>::from_array(*wc).cast::<i32>();
        let xv = Simd::<u8, 8>::from_array(*xc).cast::<i32>();
        acc += wv * xv;
    }
    acc.reduce_sum()
}

/// 候補2: 同じ形でレーン数を16へ。
fn dot_i32x16(w: &[i8], x: &[u8]) -> i32 {
    let mut acc = Simd::<i32, 16>::splat(0);
    for (wc, xc) in w.as_chunks::<16>().0.iter().zip(x.as_chunks::<16>().0) {
        let wv = Simd::<i8, 16>::from_array(*wc).cast::<i32>();
        let xv = Simd::<u8, 16>::from_array(*xc).cast::<i32>();
        acc += wv * xv;
    }
    acc.reduce_sum()
}

/// 候補3: i16で積を持ち、ブロックごとにi32へ畳む。
/// 活性は0..127、重みは-128..127なので積は|16256|以下。
/// 16要素ぶんの和でもi16に収まる範囲を超えるため、8要素ごとに畳む。
fn dot_i16(w: &[i8], x: &[u8]) -> i32 {
    let mut acc = Simd::<i32, 16>::splat(0);
    for (wc, xc) in w.as_chunks::<16>().0.iter().zip(x.as_chunks::<16>().0) {
        let wv = Simd::<i8, 16>::from_array(*wc).cast::<i16>();
        let xv = Simd::<u8, 16>::from_array(*xc).cast::<i16>();
        acc += (wv * xv).cast::<i32>();
    }
    acc.reduce_sum()
}

/// 候補4: NEONのSDOT。活性は0..127なのでi8として扱っても値が変わらない。
#[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
fn dot_sdot(w: &[i8], x: &[u8]) -> i32 {
    use std::arch::aarch64::{int32x4_t, vaddvq_s32, vdotq_s32, vdupq_n_s32, vld1q_s8};
    // SAFETY: dotprodはtarget_featureで有効。読み出しは16バイト境界の
    // チャンクに限り、長さはas_chunksが保証する
    unsafe {
        let mut acc: int32x4_t = vdupq_n_s32(0);
        for (wc, xc) in w.as_chunks::<16>().0.iter().zip(x.as_chunks::<16>().0) {
            let wv = vld1q_s8(wc.as_ptr());
            let xv = vld1q_s8(xc.as_ptr().cast::<i8>());
            acc = vdotq_s32(acc, wv, xv);
        }
        vaddvq_s32(acc)
    }
}

/// 候補5: SDOTで4行を同時に回す。活性のロードを4行で共有し、
/// 水平加算を行ごとの1回から4行まとめての1回に減らす。
#[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
fn layer_sdot4(w: &[i8], x: &[u8], cols: usize, out: &mut [i32]) {
    use std::arch::aarch64::{int32x4_t, vaddvq_s32, vdotq_s32, vdupq_n_s32, vld1q_s8};
    debug_assert!(out.len().is_multiple_of(4) && cols.is_multiple_of(16));
    // SAFETY: dotprodはtarget_featureで有効。読み出しは16バイトごとで、
    // 添字は cols と out.len() の倍数条件から範囲内に収まる
    unsafe {
        for o in (0..out.len()).step_by(4) {
            let mut acc: [int32x4_t; 4] = [vdupq_n_s32(0); 4];
            for k in (0..cols).step_by(16) {
                let xv = vld1q_s8(x.as_ptr().add(k).cast::<i8>());
                for (r, a) in acc.iter_mut().enumerate() {
                    *a = vdotq_s32(*a, vld1q_s8(w.as_ptr().add((o + r) * cols + k)), xv);
                }
            }
            for (r, a) in acc.iter().enumerate() {
                out[o + r] = vaddvq_s32(*a);
            }
        }
    }
}

fn layer(dot: impl Fn(&[i8], &[u8]) -> i32, w: &[i8], x: &[u8], cols: usize, out: &mut [i32]) {
    for (o, v) in out.iter_mut().enumerate() {
        *v = dot(&w[o * cols..(o + 1) * cols], x);
    }
}

/// 候補6: SDOTで8行を同時に回す（ADR-0151群C。現行の第1層）。
#[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
fn layer_sdot8(w: &[i8], x: &[u8], cols: usize, out: &mut [i32]) {
    use std::arch::aarch64::{int32x4_t, vaddvq_s32, vdotq_s32, vdupq_n_s32, vld1q_s8};
    debug_assert!(out.len().is_multiple_of(8) && cols.is_multiple_of(16));
    // SAFETY: dotprodはtarget_featureで有効。添字は倍数条件から範囲内
    unsafe {
        for o in (0..out.len()).step_by(8) {
            let mut acc: [int32x4_t; 8] = [vdupq_n_s32(0); 8];
            for k in (0..cols).step_by(16) {
                let xv = vld1q_s8(x.as_ptr().add(k).cast::<i8>());
                for (r, a) in acc.iter_mut().enumerate() {
                    *a = vdotq_s32(*a, vld1q_s8(w.as_ptr().add((o + r) * cols + k)), xv);
                }
            }
            for (r, a) in acc.iter().enumerate() {
                out[o + r] = vaddvq_s32(*a);
            }
        }
    }
}

/// 4列チャンク単位のインターリーブ表（ADR-0151群L）。
/// `t[k * rows * 4 + o * 4 + j] = w[o * cols + 4k + j]`。
fn interleave(w: &[i8], rows: usize, cols: usize) -> Vec<i8> {
    let mut t = vec![0i8; rows * cols];
    for k in 0..cols / 4 {
        for o in 0..rows {
            for j in 0..4 {
                t[k * rows * 4 + o * 4 + j] = w[o * cols + 4 * k + j];
            }
        }
    }
    t
}

/// 非ゼロチャンクの添字表（バイトマスク→8個の位置）。
const NNZ_LUT: [[u16; 8]; 256] = {
    let mut t = [[0u16; 8]; 256];
    let mut b = 0usize;
    while b < 256 {
        let mut n = 0;
        let mut i = 0;
        while i < 8 {
            if b & (1 << i) != 0 {
                t[b][n] = i as u16;
                n += 1;
            }
            i += 1;
        }
        b += 1;
    }
    t
};

/// アキュムレータの本数（4出力=1本）。
#[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
const NACC: usize = HIDDEN / 4;

/// 列駆動の積和本体。`nnz[..count]` のチャンクだけを積む。
#[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
#[inline]
fn sparse_body(wt: &[i8], x: &[u8], nnz: &[u16], out: &mut [i32]) {
    use std::arch::aarch64::{
        int32x4_t, vdotq_s32, vdupq_n_s32, vdupq_n_u32, vld1q_s8, vreinterpretq_s8_u32, vst1q_s32,
    };
    // SAFETY: dotprodはtarget_featureで有効。nnzの要素は 0..x.len()/4 に
    // 収まり、表の1チャンクは HIDDEN*4 バイトで範囲内
    unsafe {
        let mut acc: [int32x4_t; NACC] = [vdupq_n_s32(0); NACC];
        for &k in nnz {
            let v = x
                .as_ptr()
                .add(k as usize * 4)
                .cast::<u32>()
                .read_unaligned();
            let xv = vreinterpretq_s8_u32(vdupq_n_u32(v));
            let base = wt.as_ptr().add(k as usize * HIDDEN * 4);
            for (r, a) in acc.iter_mut().enumerate() {
                *a = vdotq_s32(*a, vld1q_s8(base.add(r * 16)), xv);
            }
        }
        for (r, a) in acc.iter().enumerate() {
            vst1q_s32(out.as_mut_ptr().add(r * 4), *a);
        }
    }
}

/// 候補7: 分岐つきの素朴な走査（列挙とdotを1パスで）。
#[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
fn layer_sparse_branch(wt: &[i8], x: &[u8], out: &mut [i32]) {
    use std::arch::aarch64::{
        int32x4_t, vdotq_s32, vdupq_n_s32, vdupq_n_u32, vld1q_s8, vreinterpretq_s8_u32, vst1q_s32,
    };
    // SAFETY: 上と同じ
    unsafe {
        let mut acc: [int32x4_t; NACC] = [vdupq_n_s32(0); NACC];
        for k in 0..x.len() / 4 {
            let v = x.as_ptr().add(k * 4).cast::<u32>().read_unaligned();
            if v != 0 {
                let xv = vreinterpretq_s8_u32(vdupq_n_u32(v));
                let base = wt.as_ptr().add(k * HIDDEN * 4);
                for (r, a) in acc.iter_mut().enumerate() {
                    *a = vdotq_s32(*a, vld1q_s8(base.add(r * 16)), xv);
                }
            }
        }
        for (r, a) in acc.iter().enumerate() {
            vst1q_s32(out.as_mut_ptr().add(r * 4), *a);
        }
    }
}

/// 候補8: 分岐なしの走査で添字を集めてから積む。
#[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
fn layer_sparse_scan(wt: &[i8], x: &[u8], out: &mut [i32]) {
    let mut nnz = [0u16; CONCAT / 4 + 8];
    let mut count = 0;
    for k in 0..x.len() / 4 {
        // SAFETY: kは範囲内。u8列をu32として非整列で読む
        let v = unsafe { x.as_ptr().add(k * 4).cast::<u32>().read_unaligned() };
        nnz[count] = k as u16;
        count += usize::from(v != 0);
    }
    sparse_body(wt, x, &nnz[..count], out);
}

/// 候補9: SIMDでマスクを作り、256エントリの表で添字を展開する。
#[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
fn layer_sparse_lut(wt: &[i8], x: &[u8], out: &mut [i32]) {
    use std::arch::aarch64::{
        vaddq_u16, vaddv_u8, vandq_u8, vceqzq_u32, vcombine_u8, vcombine_u16, vdupq_n_u16,
        vget_high_u8, vget_low_u8, vld1q_u8, vld1q_u16, vld1q_u32, vmovn_u16, vmovn_u32, vmvnq_u8,
        vst1q_u16,
    };
    const BITS: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];
    let mut nnz = [0u16; CONCAT / 4 + 8];
    let mut count = 0usize;
    let chunks = x.len() / 4;
    let mut k = 0usize;
    // SAFETY: 読み出しは16バイトごとで、k+16 <= chunks をループ条件が保証する。
    // 書き込みはcount+8 <= nnz.len() が chunks の上限から従う
    unsafe {
        let bits = vld1q_u8(BITS.as_ptr());
        while k + 16 <= chunks {
            let p = x.as_ptr().add(k * 4).cast::<u32>();
            let c0 = vceqzq_u32(vld1q_u32(p));
            let c1 = vceqzq_u32(vld1q_u32(p.add(4)));
            let c2 = vceqzq_u32(vld1q_u32(p.add(8)));
            let c3 = vceqzq_u32(vld1q_u32(p.add(12)));
            let m01 = vcombine_u16(vmovn_u32(c0), vmovn_u32(c1));
            let m23 = vcombine_u16(vmovn_u32(c2), vmovn_u32(c3));
            let m = vcombine_u8(vmovn_u16(m01), vmovn_u16(m23));
            // ceqzは「ゼロなら全1」なので反転して非ゼロのビットを立てる
            let b = vandq_u8(vmvnq_u8(m), bits);
            let lo = vaddv_u8(vget_low_u8(b)) as usize;
            let hi = vaddv_u8(vget_high_u8(b)) as usize;
            let idx = vld1q_u16(NNZ_LUT[lo].as_ptr());
            vst1q_u16(
                nnz.as_mut_ptr().add(count),
                vaddq_u16(idx, vdupq_n_u16(k as u16)),
            );
            count += lo.count_ones() as usize;
            let idx = vld1q_u16(NNZ_LUT[hi].as_ptr());
            vst1q_u16(
                nnz.as_mut_ptr().add(count),
                vaddq_u16(idx, vdupq_n_u16(k as u16 + 8)),
            );
            count += hi.count_ones() as usize;
            k += 16;
        }
    }
    while k < chunks {
        // SAFETY: kは範囲内
        let v = unsafe { x.as_ptr().add(k * 4).cast::<u32>().read_unaligned() };
        nnz[count] = k as u16;
        count += usize::from(v != 0);
        k += 1;
    }
    sparse_body(wt, x, &nnz[..count], out);
}

fn bench_dot(c: &mut Criterion) {
    // 実際の重みに近い分布を作る（値そのものは速度に影響しない）
    let w2: Vec<i8> = (0..HIDDEN * CONCAT)
        .map(|i| ((i * 37) % 128) as i8 - 64)
        .collect();
    let w3: Vec<i8> = (0..HIDDEN * HIDDEN)
        .map(|i| ((i * 53) % 128) as i8 - 64)
        .collect();
    let concat: Vec<u8> = (0..CONCAT).map(|i| ((i * 17) % 128) as u8).collect();
    let h2: Vec<u8> = (0..HIDDEN).map(|i| ((i * 11) % 128) as u8).collect();
    let mut out2 = vec![0i32; HIDDEN];
    let mut out3 = vec![0i32; HIDDEN];

    // 正しさの確認。候補が食い違ったら計測しても意味がない
    let base = dot_i32x8(&w2[..CONCAT], &concat);
    assert_eq!(dot_i32x16(&w2[..CONCAT], &concat), base);
    assert_eq!(dot_i16(&w2[..CONCAT], &concat), base);
    #[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
    assert_eq!(dot_sdot(&w2[..CONCAT], &concat), base);

    let mut g = c.benchmark_group("nnue_dot");

    macro_rules! variants {
        ($($name:literal => $f:expr),* $(,)?) => {$(
            g.bench_function(concat!($name, "/layer1_32x512"), |b| {
                b.iter(|| layer($f, black_box(&w2), black_box(&concat), CONCAT, &mut out2))
            });
            g.bench_function(concat!($name, "/layer2_32x32"), |b| {
                b.iter(|| layer($f, black_box(&w3), black_box(&h2), HIDDEN, &mut out3))
            });
        )*};
    }

    variants! {
        "i32x8" => dot_i32x8,
        "i32x16" => dot_i32x16,
        "i16" => dot_i16,
    }
    #[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
    variants! { "sdot" => dot_sdot }

    #[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
    {
        let mut ref2 = vec![0i32; HIDDEN];
        layer(dot_i32x8, &w2, &concat, CONCAT, &mut ref2);
        layer_sdot4(&w2, &concat, CONCAT, &mut out2);
        assert_eq!(out2, ref2);
        g.bench_function("sdot4/layer1_32x512", |b| {
            b.iter(|| layer_sdot4(black_box(&w2), black_box(&concat), CONCAT, &mut out2))
        });
        g.bench_function("sdot4/layer2_32x32", |b| {
            b.iter(|| layer_sdot4(black_box(&w3), black_box(&h2), HIDDEN, &mut out3))
        });
    }

    // 第1層の列駆動（ADR-0151群L）。活性のゼロ率は実測の0.731に合わせる
    #[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
    {
        let mut seed = 0x1234_5678_9ABC_DEF0u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        // **1本の活性を使い回すと分岐予測がその並びを覚える。** 実際の探索は
        // 呼び出しごとに別の並びになるので、64本を順に回して覚えられなくする
        const PATTERNS: usize = 1024;
        let sparse_xs: Vec<Vec<u8>> = (0..PATTERNS)
            .map(|_| {
                (0..CONCAT)
                    .map(|_| {
                        if next() % 1000 < 731 {
                            0
                        } else {
                            (next() % 127 + 1) as u8
                        }
                    })
                    .collect()
            })
            .collect();
        let wt = interleave(&w2, HIDDEN, CONCAT);
        let mut ref2 = vec![0i32; HIDDEN];
        for x in &sparse_xs {
            layer(dot_i32x8, &w2, x, CONCAT, &mut ref2);
            for f in [
                layer_sparse_branch as fn(&[i8], &[u8], &mut [i32]),
                layer_sparse_scan,
                layer_sparse_lut,
            ] {
                out2.fill(0);
                f(&wt, x, &mut out2);
                assert_eq!(out2, ref2);
            }
        }
        let mut i = 0usize;
        let mut rotate = move || {
            i = (i + 1) % PATTERNS;
            i
        };
        g.bench_function("sdot8/layer1_sparse73", |b| {
            b.iter(|| {
                let x = &sparse_xs[rotate()];
                layer_sdot8(black_box(&w2), black_box(x), CONCAT, &mut out2);
            })
        });
        g.bench_function("sparse_branch/layer1_sparse73", |b| {
            b.iter(|| {
                let x = &sparse_xs[rotate()];
                layer_sparse_branch(black_box(&wt), black_box(x), &mut out2);
            })
        });
        g.bench_function("sparse_scan/layer1_sparse73", |b| {
            b.iter(|| {
                let x = &sparse_xs[rotate()];
                layer_sparse_scan(black_box(&wt), black_box(x), &mut out2);
            })
        });
        g.bench_function("sparse_lut/layer1_sparse73", |b| {
            b.iter(|| {
                let x = &sparse_xs[rotate()];
                layer_sparse_lut(black_box(&wt), black_box(x), &mut out2);
            })
        });
    }

    g.finish();
}

criterion_group!(benches, bench_dot);
criterion_main!(benches);
