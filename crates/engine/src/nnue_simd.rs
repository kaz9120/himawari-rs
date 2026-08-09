//! NNUE推論のSIMD実装（ADR-0036）。
//!
//! `std::simd`（portable SIMD）による移植可能な実装。スカラー実装
//! （nnue.rs）が正解器で、両者のビット一致をテストで要求する。
//! 整数加算はスカラー側のwrapping_addと同じくラップ動作。

use std::simd::Simd;
use std::simd::cmp::SimdOrd;
use std::simd::num::SimdInt;

use crate::nnue::{CONCAT, FT_OUT, L1_OUT, L1_PAD, L2_OUT, L2_PAD, L3_OUT, NnueNetwork};
use crate::value::Value;

const I16_LANES: usize = 16;

/// FT重みをi8で持つときの1回あたりのレーン数（ADR-0138）。
/// 読み出しが半分になり、1命令で扱える要素が倍になる。
#[cfg(ft_i8)]
const FT_I8_LANES: usize = 32;

/// accを1パスで舐めるときのチャンク数（i16重み）。
#[cfg(not(ft_i8))]
const FT_CHUNKS: usize = FT_OUT / I16_LANES;

/// 同上（i8重み。1チャンクが32要素になる）。
#[cfg(ft_i8)]
const FT_CHUNKS: usize = FT_OUT / FT_I8_LANES;

/// i8経路は32要素ずつ舐めるので、FT_OUTが32の倍数でないと末尾が落ちる。
/// build.rsが要求するのは16の倍数までなので、ここで止める。
#[cfg(ft_i8)]
const _: () = assert!(FT_OUT.is_multiple_of(FT_I8_LANES));

/// `dst[k] = src[k] - Σsubs[·][k] + Σadds[·][k]`（i16、ラップ加減算。
/// ADR-0151群A・群N）。
///
/// 親のaccを読みながら全差分を適用し、自分のaccへ書く。accへの往復が
/// 1回で済む。i16のラップ加減算は可換かつ結合的なので、1行ずつ足し引き
/// した結果とビット一致する。行数は定数なので内側の展開はコンパイル時に
/// 決まる。
///
/// 視点の本数 `V` も定数で受ける。両視点で同時に更新できる区間は `V = 2`
/// で呼び、accのチャンク1本ぶんの中で2色を続けて処理する。連鎖の走査と
/// dirtyのデコードが1回で済み、依存の切れた2本のストリームが並ぶ。
/// 各視点の演算順序は `V = 1` のときと同じなので結果はビット一致する。
#[cfg(not(ft_i8))]
pub fn ft_apply<const V: usize, const NS: usize, const NA: usize>(
    dst: [&mut [i16; FT_OUT]; V],
    src: [&[i16; FT_OUT]; V],
    subs: [[&[i16]; V]; NS],
    adds: [[&[i16]; V]; NA],
) {
    let subs = subs.map(|ws| {
        ws.map(|w| {
            debug_assert_eq!(w.len(), FT_OUT);
            &w.as_chunks::<I16_LANES>().0[..FT_CHUNKS]
        })
    });
    let adds = adds.map(|ws| {
        ws.map(|w| {
            debug_assert_eq!(w.len(), FT_OUT);
            &w.as_chunks::<I16_LANES>().0[..FT_CHUNKS]
        })
    });
    let dst = dst.map(|d| d.as_chunks_mut::<I16_LANES>().0);
    let src = src.map(|s| &s.as_chunks::<I16_LANES>().0[..FT_CHUNKS]);
    for i in 0..FT_CHUNKS {
        let mut acc: [Simd<i16, I16_LANES>; V] =
            std::array::from_fn(|k| Simd::from_array(src[k][i]));
        for w in &subs {
            for (k, a) in acc.iter_mut().enumerate() {
                *a -= Simd::from_array(w[k][i]);
            }
        }
        for w in &adds {
            for (k, a) in acc.iter_mut().enumerate() {
                *a += Simd::from_array(w[k][i]);
            }
        }
        for (k, a) in acc.iter().enumerate() {
            dst[k][i] = a.to_array();
        }
    }
}

/// 同上（i8重みを符号拡張してから足し引きする。ADR-0138）。
///
/// accumulatorはi16のままなので、飽和は新たに起こらない。変わるのは
/// 重みの読み出し幅だけである。
#[cfg(ft_i8)]
pub fn ft_apply<const V: usize, const NS: usize, const NA: usize>(
    dst: [&mut [i16; FT_OUT]; V],
    src: [&[i16; FT_OUT]; V],
    subs: [[&[i8]; V]; NS],
    adds: [[&[i8]; V]; NA],
) {
    let subs = subs.map(|ws| {
        ws.map(|w| {
            debug_assert_eq!(w.len(), FT_OUT);
            &w.as_chunks::<FT_I8_LANES>().0[..FT_CHUNKS]
        })
    });
    let adds = adds.map(|ws| {
        ws.map(|w| {
            debug_assert_eq!(w.len(), FT_OUT);
            &w.as_chunks::<FT_I8_LANES>().0[..FT_CHUNKS]
        })
    });
    let dst = dst.map(|d| d.as_chunks_mut::<FT_I8_LANES>().0);
    let src = src.map(|s| &s.as_chunks::<FT_I8_LANES>().0[..FT_CHUNKS]);
    for i in 0..FT_CHUNKS {
        let mut acc: [Simd<i16, FT_I8_LANES>; V] =
            std::array::from_fn(|k| Simd::from_array(src[k][i]));
        for w in &subs {
            for (k, a) in acc.iter_mut().enumerate() {
                let wide: Simd<i16, FT_I8_LANES> =
                    Simd::<i8, FT_I8_LANES>::from_array(w[k][i]).cast();
                *a -= wide;
            }
        }
        for w in &adds {
            for (k, a) in acc.iter_mut().enumerate() {
                let wide: Simd<i16, FT_I8_LANES> =
                    Simd::<i8, FT_I8_LANES>::from_array(w[k][i]).cast();
                *a += wide;
            }
        }
        for (k, a) in acc.iter().enumerate() {
            dst[k][i] = a.to_array();
        }
    }
}

/// `acc = bias + Σ特徴行`（i16、ラップ加算。ADR-0151群A）。
///
/// 外側をaccのチャンク、内側を特徴にする。accへの書きが1回になり、
/// バイアスの初期化も同じパスに入る。加算の順序は特徴ごとに足す版と
/// 同じで、結果もビット一致する。
#[cfg(not(ft_i8))]
pub fn ft_refresh(acc: &mut [i16; FT_OUT], bias: &[i16], w: &[i16], features: &[u32]) {
    debug_assert_eq!(bias.len(), FT_OUT);
    for (i, (a, b)) in acc
        .as_chunks_mut::<I16_LANES>()
        .0
        .iter_mut()
        .zip(bias.as_chunks::<I16_LANES>().0)
        .enumerate()
    {
        let mut v = Simd::from_array(*b);
        for &f in features {
            let base = f as usize * FT_OUT + i * I16_LANES;
            v += Simd::<i16, I16_LANES>::from_slice(&w[base..base + I16_LANES]);
        }
        *a = v.to_array();
    }
}

/// 同上（i8重み。ADR-0138）。
#[cfg(ft_i8)]
pub fn ft_refresh(acc: &mut [i16; FT_OUT], bias: &[i16], w: &[i8], features: &[u32]) {
    debug_assert_eq!(bias.len(), FT_OUT);
    for (i, (a, b)) in acc
        .as_chunks_mut::<FT_I8_LANES>()
        .0
        .iter_mut()
        .zip(bias.as_chunks::<FT_I8_LANES>().0)
        .enumerate()
    {
        let mut v = Simd::from_array(*b);
        for &f in features {
            let base = f as usize * FT_OUT + i * FT_I8_LANES;
            let wide: Simd<i16, FT_I8_LANES> =
                Simd::<i8, FT_I8_LANES>::from_slice(&w[base..base + FT_I8_LANES]).cast();
            v += wide;
        }
        *a = v.to_array();
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

/// i8重み×u8活性の内積（i32）。portable版。
#[cfg(not(any(
    all(target_arch = "aarch64", target_feature = "dotprod"),
    all(target_arch = "x86_64", target_feature = "avx2")
)))]
fn dot(w: &[i8], x: &[u8]) -> i32 {
    use std::simd::num::SimdUint as _;

    debug_assert_eq!(w.len(), x.len());
    let mut acc = Simd::<i32, 8>::splat(0);
    for (wc, xc) in w.as_chunks::<8>().0.iter().zip(x.as_chunks::<8>().0) {
        let wv = Simd::<i8, 8>::from_array(*wc).cast::<i32>();
        let xv = Simd::<u8, 8>::from_array(*xc).cast::<i32>();
        acc += wv * xv;
    }
    acc.reduce_sum()
}

/// 端数の内積をスカラーで畳む（専用命令版の末尾処理）。
#[cfg(any(
    all(target_arch = "aarch64", target_feature = "dotprod"),
    all(target_arch = "x86_64", target_feature = "avx2")
))]
#[inline]
fn dot_tail(w: &[i8], x: &[u8]) -> i32 {
    w.iter()
        .zip(x)
        .map(|(&wv, &xv)| i32::from(wv) * i32::from(xv))
        .sum()
}

/// SDOT版の内積（ADR-0151群C）。1命令で16要素ぶんの積和を進める。
///
/// 出力層は1行なので行束ねの対象にならず、portable版では8レーンへ
/// 広げる形で回っていた。アキュムレータを2本に分けて依存を切り、
/// 16の倍数に満たない端数はスカラーで畳む。活性は0..127なのでi8として
/// 読んでも値が変わらない（ADR-0099）。
#[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
fn dot(w: &[i8], x: &[u8]) -> i32 {
    use std::arch::aarch64::{vaddq_s32, vaddvq_s32, vdotq_s32, vdupq_n_s32, vld1q_s8};

    debug_assert_eq!(w.len(), x.len());
    let n = w.len();
    let mut k = 0;
    // SAFETY: 読み出しは16バイト単位で、範囲内に収まることを各ループの
    // 条件が保証する。u8の並びをi8として読むが、値域0..127では同じ
    // ビット列を指す
    let sum = unsafe {
        let mut acc0 = vdupq_n_s32(0);
        let mut acc1 = vdupq_n_s32(0);
        while k + 32 <= n {
            let w0 = vld1q_s8(w.as_ptr().add(k));
            let x0 = vld1q_s8(x.as_ptr().add(k).cast::<i8>());
            let w1 = vld1q_s8(w.as_ptr().add(k + 16));
            let x1 = vld1q_s8(x.as_ptr().add(k + 16).cast::<i8>());
            acc0 = vdotq_s32(acc0, w0, x0);
            acc1 = vdotq_s32(acc1, w1, x1);
            k += 32;
        }
        if k + 16 <= n {
            let wv = vld1q_s8(w.as_ptr().add(k));
            let xv = vld1q_s8(x.as_ptr().add(k).cast::<i8>());
            acc0 = vdotq_s32(acc0, wv, xv);
            k += 16;
        }
        vaddvq_s32(vaddq_s32(acc0, acc1))
    };
    sum + dot_tail(&w[k..], &x[k..])
}

/// AVX2版の内積（ADR-0151群C）。`affine_relu` と同じく
/// `maddubs`＋`madd` の組で32要素ずつ積和する。
///
/// 隣接2要素の積の和は-32,512..32,258で、`maddubs` のi16飽和は起きない
/// （ADR-0099）。32の倍数に満たない端数はスカラーで畳む。
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
fn dot(w: &[i8], x: &[u8]) -> i32 {
    use std::arch::x86_64::{
        __m256i, _mm256_add_epi32, _mm256_loadu_si256, _mm256_madd_epi16, _mm256_maddubs_epi16,
        _mm256_set1_epi16, _mm256_setzero_si256,
    };

    debug_assert_eq!(w.len(), x.len());
    let n = w.len();
    let mut k = 0;
    // SAFETY: avx2はtarget_featureで有効。読み出しは32バイト単位で、
    // k+32 <= n をループ条件が保証する。loaduは境界整列を要求しない
    let sum = unsafe {
        let ones = _mm256_set1_epi16(1);
        let mut acc = _mm256_setzero_si256();
        while k + 32 <= n {
            let xv = _mm256_loadu_si256(x.as_ptr().add(k).cast::<__m256i>());
            let wv = _mm256_loadu_si256(w.as_ptr().add(k).cast::<__m256i>());
            let p = _mm256_maddubs_epi16(xv, wv);
            acc = _mm256_add_epi32(acc, _mm256_madd_epi16(p, ones));
            k += 32;
        }
        hsum_i32x8(acc)
    };
    sum + dot_tail(&w[k..], &x[k..])
}

/// 8レーンのi32を1つに畳む（AVX2）。
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[inline]
fn hsum_i32x8(v: std::arch::x86_64::__m256i) -> i32 {
    use std::arch::x86_64::{
        _mm_add_epi32, _mm_cvtsi128_si32, _mm_shuffle_epi32, _mm256_castsi256_si128,
        _mm256_extracti128_si256,
    };
    // SAFETY: avx2はtarget_featureで有効。レジスタ内の畳み込みのみ
    unsafe {
        let s = _mm_add_epi32(_mm256_castsi256_si128(v), _mm256_extracti128_si256::<1>(v));
        let s = _mm_add_epi32(s, _mm_shuffle_epi32::<0b01_00_11_10>(s));
        let s = _mm_add_epi32(s, _mm_shuffle_epi32::<0b10_11_00_01>(s));
        _mm_cvtsi128_si32(s)
    }
}

#[inline]
fn clip(v: i32) -> u8 {
    v.clamp(0, 127) as u8
}

/// AVX2版で束ねる行数（ADR-0099）。
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
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

/// SDOT版の行グループ1つ（`out[o..o+R]`）を計算する。
///
/// アキュムレータをR本並べ、入力ベクトル `x` のロードをR行で共有する。
/// Rはconst genericなので内側は展開され、アキュムレータはレジスタに
/// 載る。活性は `clip_to_u8` が0..127へ丸めた値なので、i8として読んでも
/// 値が変わらない。
#[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
#[inline]
fn affine_rows<const R: usize>(w: &[i8], b: &[i32], x: &[u8], out: &mut [u8], o: usize) {
    use std::arch::aarch64::{int32x4_t, vaddvq_s32, vdotq_s32, vdupq_n_s32, vld1q_s8};

    let cols = x.len();
    let mut acc: [int32x4_t; R] = [
        // SAFETY: dotprodはtarget_featureで有効。定数生成のみ
        unsafe { vdupq_n_s32(0) };
        R
    ];
    for k in (0..cols).step_by(16) {
        // SAFETY: k+16 <= cols かつ (o+r)*cols+k+16 <= w.len() が
        // 呼び出し側のdebug_assertとループ範囲から従う。u8の読み出しを
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

/// SDOT版（ADR-0099。行束ねを8へ広げた。ADR-0151群C）。
///
/// aarch64はSIMDレジスタが32本あるので、8行ぶんのアキュムレータを
/// 並べても溢れない。入力ベクトルのロードが8行で共有され、4行束ねの
/// 半分になる。**出力次元はビルド時に変わる**（`HIMAWARI_ARCH`）ため、
/// 8行が取れるだけ取り、端数は4行・1行へ落とす。
///
/// 積和の順序はportable版と異なるが、i32の範囲でオーバーフローしない
/// ため結果は一致する。
#[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
fn affine_relu(w: &[i8], b: &[i32], x: &[u8], out: &mut [u8]) {
    let cols = x.len();
    debug_assert!(cols.is_multiple_of(16));
    debug_assert_eq!(w.len(), out.len() * cols);
    debug_assert_eq!(b.len(), out.len());

    let n = out.len();
    let mut o = 0;
    while o + 8 <= n {
        affine_rows::<8>(w, b, x, out, o);
        o += 8;
    }
    while o + 4 <= n {
        affine_rows::<4>(w, b, x, out, o);
        o += 4;
    }
    while o < n {
        affine_rows::<1>(w, b, x, out, o);
        o += 1;
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
        __m256i, _mm256_add_epi32, _mm256_loadu_si256, _mm256_madd_epi16, _mm256_maddubs_epi16,
        _mm256_set1_epi16, _mm256_setzero_si256,
    };

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
                out[o + r] = clip((b[o + r] + hsum_i32x8(*a)) >> 6);
            }
        }
    }
}

/// 4列チャンクの数（第1層の入力）。
#[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
const NNZ_CHUNKS: usize = CONCAT / 4;

/// 第1層のアキュムレータ本数。1本（int32x4）が4出力を持つ。
#[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
const L1_ACCS: usize = L1_OUT / 4;

/// 列駆動に載せる上限（アキュムレータの本数）。これを超えるとレジスタが
/// 溢れてスピルするので、密のまま計算する。aarch64のSIMDレジスタは32本で、
/// 重みと入力に数本残す。
#[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
const L1_ACCS_MAX: usize = 16;

/// バイトマスクから非ゼロ位置8個を引く表（ADR-0151群L）。
///
/// `NNZ_LUT[m]` は、mのビットが立っている位置を前へ詰めた並びになる。
/// 立っていないぶんは使わない（`count_ones` の数だけ読む）。4KB。
#[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
static NNZ_LUT: [[u16; 8]; 256] = {
    let mut t = [[0u16; 8]; 256];
    let mut m = 0usize;
    while m < 256 {
        let mut n = 0;
        let mut i = 0;
        while i < 8 {
            if m & (1 << i) != 0 {
                t[m][n] = i as u16;
                n += 1;
            }
            i += 1;
        }
        m += 1;
    }
    t
};

/// 活性のうち非ゼロを含む4列チャンクを列挙する（ADR-0151群L）。
///
/// 16チャンク（64バイト）ずつ見て、チャンクごとの非ゼロ判定を1ビットに
/// 潰し、2バイトのマスクを `NNZ_LUT` で添字へ展開する。**分岐を持たない。**
/// 活性の並びは呼び出しごとに変わるので、分岐で書くと予測が外れ続ける
/// （ベンチで実測: 分岐版112.7ns、この版99.6ns、密127.6ns）。
///
/// 戻り値は `nnz` に書いた個数。
#[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
#[inline]
fn find_nnz(x: &[u8; CONCAT], nnz: &mut [u16; NNZ_CHUNKS + 8]) -> usize {
    use std::arch::aarch64::{
        vaddq_u16, vaddv_u8, vandq_u8, vceqzq_u32, vcombine_u8, vcombine_u16, vdupq_n_u16,
        vget_high_u8, vget_low_u8, vld1q_u8, vld1q_u16, vld1q_u32, vmovn_u16, vmovn_u32, vmvnq_u8,
        vst1q_u16,
    };

    /// バイトへ潰すときの重み。前半8レーンと後半8レーンで同じ並びにし、
    /// `vaddv_u8` を上下half別に取って2バイトのマスクにする。
    const BITS: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];

    let mut count = 0usize;
    let mut k = 0usize;
    // SAFETY: 読み出しは16チャンク（64バイト）ごとで、k+16 <= NNZ_CHUNKS を
    // ループ条件が保証する。vld1q_u32は整列を要求しない。書き込みは8要素
    // ずつで、countは高々 NNZ_CHUNKS-8 までしか進まないので末尾8要素の
    // 余白に収まる
    unsafe {
        let bits = vld1q_u8(BITS.as_ptr());
        while k + 16 <= NNZ_CHUNKS {
            let p = x.as_ptr().add(k * 4).cast::<u32>();
            // ceqzは「ゼロなら全1」。4本のu32マスクをバイトへ潰す
            let m01 = vcombine_u16(
                vmovn_u32(vceqzq_u32(vld1q_u32(p))),
                vmovn_u32(vceqzq_u32(vld1q_u32(p.add(4)))),
            );
            let m23 = vcombine_u16(
                vmovn_u32(vceqzq_u32(vld1q_u32(p.add(8)))),
                vmovn_u32(vceqzq_u32(vld1q_u32(p.add(12)))),
            );
            let m = vcombine_u8(vmovn_u16(m01), vmovn_u16(m23));
            // 反転して「非ゼロなら全1」にし、重みを掛けて畳む
            let b = vandq_u8(vmvnq_u8(m), bits);
            for (half, mask) in [vget_low_u8(b), vget_high_u8(b)].into_iter().enumerate() {
                let mask = usize::from(vaddv_u8(mask));
                let base = vdupq_n_u16((k + half * 8) as u16);
                vst1q_u16(
                    nnz.as_mut_ptr().add(count),
                    vaddq_u16(vld1q_u16(NNZ_LUT[mask].as_ptr()), base),
                );
                count += mask.count_ones() as usize;
            }
            k += 16;
        }
    }
    // 16で割り切れない端数。CONCATは32の倍数なのでチャンク数は8の倍数に
    // なり、ここへ来るのは高々8チャンクである
    while k < NNZ_CHUNKS {
        // SAFETY: k < NNZ_CHUNKS なので4バイトの読み出しは範囲内。
        // 活性の並びは整列を保証しないので非整列で読む
        let v = unsafe { x.as_ptr().add(k * 4).cast::<u32>().read_unaligned() };
        nnz[count] = k as u16;
        count += usize::from(v != 0);
        k += 1;
    }
    count
}

/// 第1層を列駆動で計算する（ADR-0151群L）。
///
/// 活性の73.1%はゼロなので、4列チャンクの28.4%は丸ごとゼロになる。
/// そのチャンクを飛ばすと積和がその分だけ減る。**i32の和は正確なので、
/// 加算順序を変えてもゼロ項を飛ばしても結果はビット一致する。**
///
/// `wt` は `nnue::interleave_w2` が作る4列チャンク単位の表で、チャンクkの
/// 16バイトごとに出力4行ぶんの重みが並ぶ。入力4バイトをブロードキャストし、
/// `vdotq_s32` の4レーンで4行を同時に進める。
#[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
fn affine_relu_l1_sparse(wt: &[i8], b: &[i32], x: &[u8; CONCAT], out: &mut [u8]) {
    use std::arch::aarch64::{
        int32x4_t, vdotq_s32, vdupq_n_s32, vdupq_n_u32, vld1q_s8, vreinterpretq_s8_u32, vst1q_s32,
    };

    debug_assert_eq!(wt.len(), L1_OUT * CONCAT);
    debug_assert_eq!(b.len(), L1_OUT);
    debug_assert_eq!(out.len(), L1_OUT);

    let mut nnz = [0u16; NNZ_CHUNKS + 8];
    let count = find_nnz(x, &mut nnz);

    let mut sums = [0i32; L1_OUT];
    // SAFETY: nnzの要素は 0..NNZ_CHUNKS で、チャンクkの重みは
    // wt[k*L1_OUT*4 .. (k+1)*L1_OUT*4] にあり、wt.len()==L1_OUT*CONCAT から
    // 範囲内に収まる。活性の4バイト読みも同じ理由で範囲内。書き出しは
    // sumsの L1_OUT 要素ちょうど
    unsafe {
        let mut acc: [int32x4_t; L1_ACCS] = [vdupq_n_s32(0); L1_ACCS];
        for &k in &nnz[..count] {
            let k = usize::from(k);
            let v = x.as_ptr().add(k * 4).cast::<u32>().read_unaligned();
            let xv = vreinterpretq_s8_u32(vdupq_n_u32(v));
            let base = wt.as_ptr().add(k * L1_OUT * 4);
            for (r, a) in acc.iter_mut().enumerate() {
                *a = vdotq_s32(*a, vld1q_s8(base.add(r * 16)), xv);
            }
        }
        for (r, a) in acc.iter().enumerate() {
            vst1q_s32(sums.as_mut_ptr().add(r * 4), *a);
        }
    }
    for (o, h) in out.iter_mut().enumerate() {
        *h = clip((b[o] + sums[o]) >> 6);
    }
}

/// 第1層。SDOT経路だけ列駆動へ回す（ADR-0151群L）。
///
/// 派生表が空のネット（`NnueNetwork::finish` を呼ばずに組んだ場合）と、
/// アキュムレータが溢れるほど出力が広い構成では密のまま計算する。
/// どちらの経路も同じ値を返すので、落ちても速度が戻るだけである。
#[cfg(all(target_arch = "aarch64", target_feature = "dotprod"))]
#[inline]
fn l1(net: &NnueNetwork, x: &[u8; CONCAT], out: &mut [u8]) {
    let wt = if L1_ACCS <= L1_ACCS_MAX {
        net.w2_sparse.get(..L1_OUT * CONCAT)
    } else {
        None
    };
    match wt {
        Some(wt) => affine_relu_l1_sparse(wt, &net.b2, x, out),
        None => affine_relu(&net.w2, &net.b2, x, out),
    }
}

/// 同上（SDOT以外の経路。密のまま計算する）。
#[cfg(not(all(target_arch = "aarch64", target_feature = "dotprod")))]
#[inline]
fn l1(net: &NnueNetwork, x: &[u8; CONCAT], out: &mut [u8]) {
    affine_relu(&net.w2, &net.b2, x, out);
}

/// 連結ベクトルから評価値まで（SIMD版）。
pub fn forward_hidden(net: &NnueNetwork, concat: &[u8; CONCAT]) -> Value {
    // 次の層の入力はパディングした幅で渡す。実次元より後ろはゼロのままで、
    // 重みの対応する列もゼロなので積和に効かない（ADR-0127）
    let mut h2 = [0u8; L1_PAD];
    l1(net, concat, &mut h2[..L1_OUT]);
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
    // 出力層は1行なので行束ねの対象にならない。専用命令版のdotで畳む
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
        let row = |i: usize| &net.ft_w[i * FT_OUT..(i + 1) * FT_OUT];
        // 重みの格納型はビルドで変わる（ADR-0138）。参照側はi16へ揃える。
        // 既定ビルドでは型が同じで変換が恒等になるが、i8ビルドでは必要
        #[allow(clippy::useless_conversion)]
        {
            // ft_apply: 引く行2・足す行2の最大構成でラップ動作が一致する
            let src = [i16::MAX - 3; FT_OUT];
            let mut dst = [0i16; FT_OUT];
            ft_apply(
                [&mut dst],
                [&src],
                [[row(0)], [row(1)]],
                [[row(2)], [row(3)]],
            );
            let mut want = src;
            for (o, x) in want.iter_mut().enumerate() {
                *x = x
                    .wrapping_sub(i16::from(row(0)[o]))
                    .wrapping_sub(i16::from(row(1)[o]))
                    .wrapping_add(i16::from(row(2)[o]))
                    .wrapping_add(i16::from(row(3)[o]));
            }
            assert_eq!(dst, want);
            // 差分が空なら親のaccをそのまま写す
            ft_apply([&mut dst], [&src], [], []);
            assert_eq!(dst, src);
            // ft_refresh: バイアス＋特徴行の総和が一致する
            let features = [3u32, 11, 29];
            let mut acc = [0i16; FT_OUT];
            ft_refresh(&mut acc, &net.ft_b[..FT_OUT], &net.ft_w, &features);
            let mut want = [0i16; FT_OUT];
            for (o, x) in want.iter_mut().enumerate() {
                *x = net.ft_b[o];
                for &f in &features {
                    *x = x.wrapping_add(i16::from(net.ft_w[f as usize * FT_OUT + o]));
                }
            }
            assert_eq!(acc, want);
        }
        // dot: スカラー積和一致。専用命令版は16または32要素ずつ進むので、
        // 端数の畳み込みも含めて長さを変えて照合する
        let x: Vec<u8> = (0..CONCAT).map(|i| (i % 128) as u8).collect();
        for len in [8, 16, 24, 32, 40, CONCAT] {
            let w8 = &net.w2[..len];
            let xs = &x[..len];
            let scalar: i32 = w8
                .iter()
                .zip(xs)
                .map(|(&w, &v)| i32::from(w) * i32::from(v))
                .sum();
            assert_eq!(dot(w8, xs), scalar, "len={len}");
        }
    }

    /// 両視点1パスが片視点2回とビット一致すること（ADR-0151群N）。
    ///
    /// 融合しても視点ごとの演算順序は変わらない。行数の組み合わせを
    /// すべて回し、ラップの起きる境界値で照合する。
    #[test]
    fn ft_apply_two_views_matches_single_view() {
        let net = NnueNetwork::random(11);
        let row = |i: usize| &net.ft_w[i * FT_OUT..(i + 1) * FT_OUT];
        // 視点ごとに別の行・別の親accを使い、取り違えを検出できるようにする
        let src: [[i16; FT_OUT]; 2] = [[i16::MAX - 5; FT_OUT], [i16::MIN + 5; FT_OUT]];
        let s = [[row(0), row(4)], [row(1), row(5)]];
        let a = [[row(2), row(6)], [row(3), row(7)]];
        for ns in 0..=2usize {
            for na in 0..=2usize {
                let mut want = [[0i16; FT_OUT]; 2];
                for k in 0..2 {
                    let mut d = [0i16; FT_OUT];
                    match (ns, na) {
                        (0, 0) => ft_apply([&mut d], [&src[k]], [], []),
                        (0, 1) => ft_apply([&mut d], [&src[k]], [], [[a[0][k]]]),
                        (0, 2) => ft_apply([&mut d], [&src[k]], [], [[a[0][k]], [a[1][k]]]),
                        (1, 0) => ft_apply([&mut d], [&src[k]], [[s[0][k]]], []),
                        (1, 1) => ft_apply([&mut d], [&src[k]], [[s[0][k]]], [[a[0][k]]]),
                        (1, 2) => {
                            ft_apply([&mut d], [&src[k]], [[s[0][k]]], [[a[0][k]], [a[1][k]]])
                        }
                        (2, 0) => ft_apply([&mut d], [&src[k]], [[s[0][k]], [s[1][k]]], []),
                        (2, 1) => {
                            ft_apply([&mut d], [&src[k]], [[s[0][k]], [s[1][k]]], [[a[0][k]]])
                        }
                        _ => ft_apply(
                            [&mut d],
                            [&src[k]],
                            [[s[0][k]], [s[1][k]]],
                            [[a[0][k]], [a[1][k]]],
                        ),
                    }
                    want[k] = d;
                }
                let mut got = [[0i16; FT_OUT]; 2];
                {
                    let [d0, d1] = &mut got;
                    let [s0, s1] = &src;
                    match (ns, na) {
                        (0, 0) => ft_apply([d0, d1], [s0, s1], [], []),
                        (0, 1) => ft_apply([d0, d1], [s0, s1], [], [a[0]]),
                        (0, 2) => ft_apply([d0, d1], [s0, s1], [], [a[0], a[1]]),
                        (1, 0) => ft_apply([d0, d1], [s0, s1], [s[0]], []),
                        (1, 1) => ft_apply([d0, d1], [s0, s1], [s[0]], [a[0]]),
                        (1, 2) => ft_apply([d0, d1], [s0, s1], [s[0]], [a[0], a[1]]),
                        (2, 0) => ft_apply([d0, d1], [s0, s1], [s[0], s[1]], []),
                        (2, 1) => ft_apply([d0, d1], [s0, s1], [s[0], s[1]], [a[0]]),
                        _ => ft_apply([d0, d1], [s0, s1], [s[0], s[1]], [a[0], a[1]]),
                    }
                }
                assert_eq!(got, want, "ns={ns}, na={na}");
            }
        }
    }

    /// 行束ねがどの行数でもスカラーと一致すること（ADR-0151群C）。
    ///
    /// 出力次元は `HIMAWARI_ARCH` でビルド時に変わる。8行束ねが割り切れず
    /// 4行束ねへ落ちる行数（12・20）も含めて照合する。行数を4の倍数に
    /// 限るのは、AVX2版が4行束ね前提のままだからである（build.rsの
    /// `L1_MULTIPLE`）。
    #[test]
    fn affine_relu_matches_scalar_for_row_counts() {
        let net = NnueNetwork::random(7);
        // 32はaarch64（16の倍数）とAVX2（32の倍数）の両方を満たす列数
        const COLS: usize = 32;
        let x: Vec<u8> = (0..COLS).map(|i| ((i * 17) % 128) as u8).collect();
        for rows in [4usize, 8, 12, 16, 20, 24, 32] {
            let w = &net.w2[..rows * COLS];
            let b: Vec<i32> = (0..rows).map(|o| o as i32 * 977 - 2000).collect();
            let mut out = vec![0u8; rows];
            affine_relu(w, &b, &x, &mut out);
            for (o, &h) in out.iter().enumerate() {
                let sum: i32 = (0..COLS)
                    .map(|i| i32::from(w[o * COLS + i]) * i32::from(x[i]))
                    .sum();
                assert_eq!(h, clip((b[o] + sum) >> 6), "rows={rows} o={o}");
            }
        }
    }

    /// 隠れ層の推論がスカラー実装とビット一致すること（ADR-0099・0151群L）。
    ///
    /// SDOT経路は積和の順序がスカラーと異なり、第1層は列駆動でゼロの列を
    /// 飛ばす。**飛ばす列の並びで結果が変わらないこと**を見るため、
    /// 境界値（全0・全127）に加えてゼロ率を振ったランダムパターンを照合する。
    /// ゼロ率0.731は探索での実測値である。
    #[test]
    fn forward_hidden_matches_scalar() {
        let net = NnueNetwork::random(11);
        let mut concat = [0u8; CONCAT];
        // 全0・全127・鋸歯（全列が非ゼロ）の3パターン
        for pattern in 0..3 {
            for (i, v) in concat.iter_mut().enumerate() {
                *v = match pattern {
                    0 => 0,
                    1 => 127,
                    _ => ((i * 31) % 128 + 1).min(127) as u8,
                };
            }
            assert_eq!(
                forward_hidden(&net, &concat),
                crate::nnue::forward_hidden(&net, &concat),
                "pattern={pattern}"
            );
        }
        // ゼロ率を振ったランダムパターン。ゼロの位置が毎回変わるので、
        // 4列チャンクの飛ばし方（全ゼロ・一部ゼロ・非ゼロ）が総当たりに近く出る
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for zero_permille in [1u64, 250, 500, 731, 900, 999] {
            for _ in 0..32 {
                fill_activations(&mut concat, zero_permille, &mut state);
                assert_eq!(
                    forward_hidden(&net, &concat),
                    crate::nnue::forward_hidden(&net, &concat),
                    "zero_permille={zero_permille}"
                );
            }
        }
    }

    /// 第1層が飽和しないネット。乱数ネットのままだと和が大きく、出力の
    /// 多くがclipの端（0か127）に張り付くため、**列駆動の誤りが潰されて
    /// 見えなくなる。** 重みを小さくして値の違いが出るようにする。
    fn small_l1_net(seed: u64) -> NnueNetwork {
        let mut net = NnueNetwork::random(seed);
        for w in &mut net.w2 {
            *w = *w % 5 - 2;
        }
        net.finish()
    }

    /// ゼロ率を指定した活性を作る（0以外は1..127）。
    fn fill_activations(concat: &mut [u8; CONCAT], zero_permille: u64, state: &mut u64) {
        for v in concat.iter_mut() {
            let mut next = || {
                *state ^= *state << 13;
                *state ^= *state >> 7;
                *state ^= *state << 17;
                *state
            };
            *v = if next() % 1000 < zero_permille {
                0
            } else {
                (next() % 127 + 1) as u8
            };
        }
    }

    /// 第1層の出力が1つずつスカラーと一致すること（ADR-0151群L）。
    /// ゼロの位置は毎回変わるので、4列チャンクの飛ばし方が総当たりに近く出る。
    #[test]
    fn l1_matches_scalar_without_saturation() {
        let net = small_l1_net(23);
        let mut state = 0xDEAD_BEEF_1234_5678u64;
        let mut concat = [0u8; CONCAT];
        let mut out = vec![0u8; L1_OUT];
        for zero_permille in [0u64, 500, 731, 990] {
            for _ in 0..16 {
                fill_activations(&mut concat, zero_permille, &mut state);
                l1(&net, &concat, &mut out);
                for (o, &h) in out.iter().enumerate() {
                    let sum: i32 = (0..CONCAT)
                        .map(|i| i32::from(net.w2[o * CONCAT + i]) * i32::from(concat[i]))
                        .sum();
                    assert_eq!(h, clip((net.b2[o] + sum) >> 6), "o={o}");
                }
            }
        }
    }

    /// 派生表を持たないネットでも同じ値を返すこと（ADR-0151群L）。
    /// 列駆動は表が空なら密へ落ちる。落ちても結果は変わらない。
    #[test]
    fn l1_without_the_interleaved_table() {
        let full = small_l1_net(29);
        let mut bare = small_l1_net(29);
        bare.w2_sparse = Vec::new();
        let mut state = 0x0BAD_C0DE_5EED_1234u64;
        let mut concat = [0u8; CONCAT];
        let (mut a, mut b) = (vec![0u8; L1_OUT], vec![0u8; L1_OUT]);
        for _ in 0..16 {
            fill_activations(&mut concat, 731, &mut state);
            l1(&full, &concat, &mut a);
            l1(&bare, &concat, &mut b);
            assert_eq!(a, b);
        }
    }
}
