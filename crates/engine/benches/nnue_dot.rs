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

    g.finish();
}

criterion_group!(benches, bench_dot);
criterion_main!(benches);
