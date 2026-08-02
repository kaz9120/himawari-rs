//! NNUE推論のSIMD実装（ADR-0036）。
//!
//! `std::simd`（portable SIMD）による移植可能な実装。スカラー実装
//! （nnue.rs）が正解器で、両者のビット一致をテストで要求する。
//! 整数加算はスカラー側のwrapping_addと同じくラップ動作。

use std::simd::Simd;
use std::simd::cmp::SimdOrd;
use std::simd::num::{SimdInt, SimdUint};

use crate::nnue::{CONCAT, FT_OUT, L1_OUT, L1_PAD, L2_OUT, L2_PAD, L3_OUT, NnueNetwork};
use crate::value::Value;

const I16_LANES: usize = 16;

/// acc += 重み列（i16、ラップ加算）。
pub fn ft_add(acc: &mut [i16; FT_OUT], w: &[i16]) {
    debug_assert_eq!(w.len(), FT_OUT);
    for (a, wc) in acc
        .as_chunks_mut::<I16_LANES>()
        .0
        .iter_mut()
        .zip(w.as_chunks::<I16_LANES>().0)
    {
        *a = (Simd::from_array(*a) + Simd::from_array(*wc)).to_array();
    }
}

/// acc -= 重み列（i16、ラップ減算）。
pub fn ft_sub(acc: &mut [i16; FT_OUT], w: &[i16]) {
    debug_assert_eq!(w.len(), FT_OUT);
    for (a, wc) in acc
        .as_chunks_mut::<I16_LANES>()
        .0
        .iter_mut()
        .zip(w.as_chunks::<I16_LANES>().0)
    {
        *a = (Simd::from_array(*a) - Simd::from_array(*wc)).to_array();
    }
}

/// i16アキュムレータをclipped ReLU（0..127）でu8へ。
pub fn clip_to_u8(acc: &[i16; FT_OUT], out: &mut [u8]) {
    debug_assert_eq!(out.len(), FT_OUT);
    let zero = Simd::<i16, I16_LANES>::splat(0);
    let max = Simd::<i16, I16_LANES>::splat(127);
    for (oc, ac) in out
        .as_chunks_mut::<I16_LANES>()
        .0
        .iter_mut()
        .zip(acc.as_chunks::<I16_LANES>().0)
    {
        let v = Simd::from_array(*ac).simd_clamp(zero, max).cast::<u8>();
        *oc = v.to_array();
    }
}

/// i8重み×u8活性の内積（i32）。
fn dot(w: &[i8], x: &[u8]) -> i32 {
    debug_assert_eq!(w.len(), x.len());
    let mut acc = Simd::<i32, 8>::splat(0);
    for (wc, xc) in w.as_chunks::<8>().0.iter().zip(x.as_chunks::<8>().0) {
        let wv = Simd::<i8, 8>::from_array(*wc).cast::<i32>();
        let xv = Simd::<u8, 8>::from_array(*xc).cast::<i32>();
        acc += wv * xv;
    }
    acc.reduce_sum()
}

#[inline]
fn clip(v: i32) -> u8 {
    v.clamp(0, 127) as u8
}

/// 4行同時に回す実装で束ねる行数（ADR-0099）。
#[cfg(any(
    all(target_arch = "aarch64", target_feature = "dotprod"),
    all(target_arch = "x86_64", target_feature = "avx2")
))]
const ROWS: usize = 4;

/// 全結合層＋clipped ReLU。`out[o] = clip((b[o] + w[o行]·x) >> 6)`。
///
/// 行ごとに内積を取る素直な版。専用命令を使う版と同じ値を返す
/// （ADR-0099）。
#[cfg(not(any(
    all(target_arch = "aarch64", target_feature = "dotprod"),
    all(target_arch = "x86_64", target_feature = "avx2")
)))]
fn affine_relu(w: &[i8], b: &[i32], x: &[u8], out: &mut [u8]) {
    let cols = x.len();
    for (o, h) in out.iter_mut().enumerate() {
        *h = clip((b[o] + dot(&w[o * cols..(o + 1) * cols], x)) >> 6);
    }
}

/// SDOT版（ADR-0099）。1命令で16要素ぶんの積和を進め、4行を同時に回す。
///
/// 活性は `clip_to_u8` が0..127へ丸めた値なので、i8として読んでも
/// 値が変わらない。積和の順序はportable版と異なるが、i32の範囲で
/// オーバーフローしないため結果は一致する。
#[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
fn affine_relu(w: &[i8], b: &[i32], x: &[u8], out: &mut [u8]) {
    use std::arch::aarch64::{int32x4_t, vaddvq_s32, vdotq_s32, vdupq_n_s32, vld1q_s8};

    let cols = x.len();
    debug_assert!(cols.is_multiple_of(16));
    debug_assert!(out.len().is_multiple_of(ROWS));
    debug_assert_eq!(w.len(), out.len() * cols);
    debug_assert_eq!(b.len(), out.len());

    for o in (0..out.len()).step_by(ROWS) {
        let mut acc: [int32x4_t; ROWS] = [
            // SAFETY: dotprodはtarget_featureで有効。定数生成のみ
            unsafe { vdupq_n_s32(0) };
            ROWS
        ];
        for k in (0..cols).step_by(16) {
            // SAFETY: k+16 <= cols かつ (o+r)*cols+k+16 <= w.len() が
            // 上のdebug_assertとループ範囲から従う。u8の読み出しを
            // i8として解釈するが、値域0..127では同じビット列を指す
            unsafe {
                let xv = vld1q_s8(x.as_ptr().add(k).cast::<i8>());
                for (r, a) in acc.iter_mut().enumerate() {
                    *a = vdotq_s32(*a, vld1q_s8(w.as_ptr().add((o + r) * cols + k)), xv);
                }
            }
        }
        for (r, a) in acc.iter().enumerate() {
            // SAFETY: dotprodはtarget_featureで有効。水平加算のみ
            let sum = unsafe { vaddvq_s32(*a) };
            out[o + r] = clip((b[o + r] + sum) >> 6);
        }
    }
}

/// AVX2版（ADR-0099）。SDOT版と同じ構造で、内側を
/// `maddubs`（u8×i8をi16へ積和）と `madd`（i16をi32へ畳む）の組で作る。
///
/// 活性は0..127、重みは-128..127なので、隣接2要素の積の和は
/// -32,512..32,258に収まる。`maddubs` はi16で飽和するが、この範囲では
/// 飽和が起きないためportable版と同じ値になる。
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
fn affine_relu(w: &[i8], b: &[i32], x: &[u8], out: &mut [u8]) {
    use std::arch::x86_64::{
        __m256i, _mm_add_epi32, _mm_cvtsi128_si32, _mm_shuffle_epi32, _mm256_add_epi32,
        _mm256_castsi256_si128, _mm256_extracti128_si256, _mm256_loadu_si256, _mm256_madd_epi16,
        _mm256_maddubs_epi16, _mm256_set1_epi16, _mm256_setzero_si256,
    };

    /// 8レーンのi32を1つに畳む。
    #[inline]
    fn hsum(v: __m256i) -> i32 {
        // SAFETY: avx2はtarget_featureで有効。レジスタ内の畳み込みのみ
        unsafe {
            let s = _mm_add_epi32(_mm256_castsi256_si128(v), _mm256_extracti128_si256::<1>(v));
            let s = _mm_add_epi32(s, _mm_shuffle_epi32::<0b01_00_11_10>(s));
            let s = _mm_add_epi32(s, _mm_shuffle_epi32::<0b10_11_00_01>(s));
            _mm_cvtsi128_si32(s)
        }
    }

    /// 1レジスタで扱う要素数（32バイト）。
    const LANES: usize = 32;
    let cols = x.len();
    debug_assert!(cols.is_multiple_of(LANES));
    debug_assert!(out.len().is_multiple_of(ROWS));
    debug_assert_eq!(w.len(), out.len() * cols);
    debug_assert_eq!(b.len(), out.len());

    // SAFETY: avx2はtarget_featureで有効。読み出しは32バイトごとで、
    // k+LANES <= cols と (o+r)*cols+k+LANES <= w.len() が上の
    // debug_assertとループ範囲から従う。loaduは境界整列を要求しない
    unsafe {
        let ones = _mm256_set1_epi16(1);
        for o in (0..out.len()).step_by(ROWS) {
            let mut acc: [__m256i; ROWS] = [_mm256_setzero_si256(); ROWS];
            for k in (0..cols).step_by(LANES) {
                let xv = _mm256_loadu_si256(x.as_ptr().add(k).cast::<__m256i>());
                for (r, a) in acc.iter_mut().enumerate() {
                    let wv =
                        _mm256_loadu_si256(w.as_ptr().add((o + r) * cols + k).cast::<__m256i>());
                    let p = _mm256_maddubs_epi16(xv, wv);
                    *a = _mm256_add_epi32(*a, _mm256_madd_epi16(p, ones));
                }
            }
            for (r, a) in acc.iter().enumerate() {
                out[o + r] = clip((b[o + r] + hsum(*a)) >> 6);
            }
        }
    }
}

/// 連結ベクトルから評価値まで（SIMD版）。
pub fn forward_hidden(net: &NnueNetwork, concat: &[u8; CONCAT]) -> Value {
    // 次の層の入力はパディングした幅で渡す。実次元より後ろはゼロのままで、
    // 重みの対応する列もゼロなので積和に効かない（ADR-0127）
    let mut h2 = [0u8; L1_PAD];
    affine_relu(&net.w2, &net.b2, concat, &mut h2[..L1_OUT]);
    // 隠れ層は書いたぶんだけ挟む。次元は定数なので使わない分岐は消える
    let mut h3 = [0u8; L2_PAD];
    let mut h4 = [0u8; L3_OUT];
    let last: &[u8] = if L2_OUT == 0 {
        &h2[..L1_OUT]
    } else {
        affine_relu(&net.w3, &net.b3, &h2, &mut h3[..L2_OUT]);
        if L3_OUT != 0 {
            affine_relu(&net.w4, &net.b4, &h3, &mut h4);
            &h4
        } else {
            &h3[..L2_OUT]
        }
    };
    // 出力層は1行なので4行同時の対象にならない
    let out = net.b_out + dot(&net.w_out, last);
    out / crate::nnue::FV_SCALE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nnue::NnueNetwork;

    /// SIMDの部品がスカラー等価であること（境界値・ラップ動作を含む）。
    #[test]
    fn primitives_match_scalar() {
        let net = NnueNetwork::random(5);
        // ft_add/sub: ラップ動作の一致
        let mut a = [i16::MAX - 3; FT_OUT];
        let mut b = a;
        let w = &net.ft_w[..FT_OUT];
        ft_add(&mut a, w);
        for (x, &wv) in b.iter_mut().zip(w) {
            *x = x.wrapping_add(wv);
        }
        assert_eq!(a, b);
        ft_sub(&mut a, w);
        for (x, &wv) in b.iter_mut().zip(w) {
            *x = x.wrapping_sub(wv);
        }
        assert_eq!(a, b);
        // dot: スカラー積和一致
        let x: Vec<u8> = (0..CONCAT).map(|i| (i % 128) as u8).collect();
        let w8 = &net.w2[..CONCAT];
        let scalar: i32 = w8
            .iter()
            .zip(&x)
            .map(|(&w, &v)| i32::from(w) * i32::from(v))
            .sum();
        assert_eq!(dot(w8, &x), scalar);
    }

    /// 隠れ層の推論がスカラー実装とビット一致すること（ADR-0099）。
    /// SDOT経路は積和の順序がスカラーと異なるため、境界値も含めて照合する。
    #[test]
    fn forward_hidden_matches_scalar() {
        let net = NnueNetwork::random(11);
        let mut concat = [0u8; CONCAT];
        // 全0・全127・鋸歯の3パターン。活性の値域は0..127（clip_to_u8）
        for pattern in 0..3 {
            for (i, v) in concat.iter_mut().enumerate() {
                *v = match pattern {
                    0 => 0,
                    1 => 127,
                    _ => ((i * 31) % 128) as u8,
                };
            }
            assert_eq!(
                forward_hidden(&net, &concat),
                crate::nnue::forward_hidden(&net, &concat),
                "pattern={pattern}"
            );
        }
    }
}
