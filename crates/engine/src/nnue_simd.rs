//! NNUE推論のSIMD実装（ADR-0036）。
//!
//! `std::simd`（portable SIMD）による移植可能な実装。スカラー実装
//! （nnue.rs）が正解器で、両者のビット一致をテストで要求する。
//! 整数加算はスカラー側のwrapping_addと同じくラップ動作。

use std::simd::Simd;
use std::simd::cmp::SimdOrd;
use std::simd::num::{SimdInt, SimdUint};

use crate::nnue::{CONCAT, FT_OUT, HIDDEN, NnueNetwork};
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

/// 連結ベクトルから評価値まで（SIMD版）。
pub fn forward_hidden(net: &NnueNetwork, concat: &[u8; CONCAT]) -> Value {
    let mut h2 = [0u8; HIDDEN];
    for (o, h) in h2.iter_mut().enumerate() {
        let sum = net.b2[o] + dot(&net.w2[o * CONCAT..(o + 1) * CONCAT], concat);
        *h = clip(sum >> 6);
    }
    let mut h3 = [0u8; HIDDEN];
    for (o, h) in h3.iter_mut().enumerate() {
        let sum = net.b3[o] + dot(&net.w3[o * HIDDEN..(o + 1) * HIDDEN], &h2);
        *h = clip(sum >> 6);
    }
    let mut out = net.b4;
    out += dot(&net.w4, &h3);
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
}
