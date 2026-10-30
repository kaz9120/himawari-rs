//! キャッシュの先読み（ADR-0250）。
//!
//! どれもCPUへのヒントで、メモリを読み書きしない。CPUが無視しても結果は
//! 変わらないので、探索木にも評価値にも影響しない。

/// `p` を含むキャッシュラインの読み込みを始めさせる。
#[inline(always)]
pub fn line<T>(p: *const T) {
    #[cfg(target_arch = "aarch64")]
    // SAFETY: prfmはメモリを読み書きせず、無効なアドレスでも例外を起こさない
    unsafe {
        std::arch::asm!(
            "prfm pldl1keep, [{p}]",
            p = in(reg) p,
            options(nostack, preserves_flags, readonly)
        );
    }
    #[cfg(target_arch = "x86_64")]
    // SAFETY: prefetchはメモリを読み書きせず、無効なアドレスでも例外を起こさない
    unsafe {
        std::arch::x86_64::_mm_prefetch::<{ std::arch::x86_64::_MM_HINT_T0 }>(p.cast());
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    let _ = p;
}
