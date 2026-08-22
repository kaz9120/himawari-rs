//! SPSAでチューニングする探索定数（ADR-0143）。
//!
//! `tune` featureなしでは各項目が定数を返す `#[inline(always)]` の関数で、
//! 呼び出しは定数へ畳み込まれる。featureありでは同じ関数がatomicを読み、
//! USIの `setoption` で走行中に書き換えられる。対象外の兄弟定数が
//! `search.rs` に残る群は、式の説明もそちらにある。群ごと移したものは
//! ここのコメントが説明を持つ。
//!
//! 可動域はSPSAの探索範囲であると同時に、USIオプションのmin/maxになる。
//! 除数に使う定数は最小値を1以上にして、ゼロ除算を型の外で防ぐ。

#[cfg(feature = "tune")]
use std::sync::atomic::{AtomicI64, Ordering};

/// チューニング項目の宣言。ここに並べた名前が、そのままUSIオプション名になる。
macro_rules! tunables {
    ($($name:ident : $ty:ty = $default:expr, [$min:expr, $max:expr];)+) => {
        #[cfg(feature = "tune")]
        mod store {
            use super::AtomicI64;
            $(
                #[allow(non_upper_case_globals)]
                pub static $name: AtomicI64 = AtomicI64::new($default as i64);
            )+
        }

        $(
            #[cfg(not(feature = "tune"))]
            #[allow(non_snake_case)]
            #[inline(always)]
            pub fn $name() -> $ty {
                $default
            }

            #[cfg(feature = "tune")]
            #[allow(non_snake_case)]
            #[inline(always)]
            pub fn $name() -> $ty {
                store::$name.load(Ordering::Relaxed) as $ty
            }
        )+

        /// USIオプションとして公開する項目の一覧。
        #[cfg(feature = "tune")]
        pub const ENTRIES: &[Entry] = &[
            $(
                Entry {
                    name: stringify!($name),
                    default: $default as i64,
                    min: $min,
                    max: $max,
                    cell: &store::$name,
                },
            )+
        ];
    };
}

/// チューニング項目のメタデータ。USI宣言とsetoptionの解決に使う。
#[cfg(feature = "tune")]
pub struct Entry {
    pub name: &'static str,
    pub default: i64,
    pub min: i64,
    pub max: i64,
    cell: &'static AtomicI64,
}

/// 名前で項目を探して値を入れる。可動域の外はclampする。
/// 見つかったらtrueを返す。呼び出し側は他のオプションを先に照合し、
/// どれにも当たらなかった名前だけをここへ回す。
#[cfg(feature = "tune")]
pub fn set(name: &str, value: i64) -> bool {
    for e in ENTRIES {
        if e.name == name {
            e.cell.store(value.clamp(e.min, e.max), Ordering::Relaxed);
            return true;
        }
    }
    false
}

tunables! {
    // NMP（式と出典はsearch.rsのNMP_EVAL_DEPTH群のコメント）
    NMP_EVAL_IMPROVING: crate::value::Value = 50, [0, 200];
    NMP_EVAL_BASE: crate::value::Value = 373, [50, 1000];
    NMP_BASE_REDUCTION: u32 = 7, [3, 12];
    NMP_DEPTH_DIVISOR: u32 = 4, [1, 8];
    // 子ノードのfutility（RFP。式と出典はsearch.rsのRFP_MAX_DEPTH群のコメント）
    RFP_MULT: i32 = 104, [20, 200];
    RFP_NO_TT_HIT: i32 = 14, [0, 80];
    RFP_IMPROVING: i32 = 1309, [500, 8000];
    RFP_OPP_WORSENING: i32 = 269, [0, 2000];
    // 親ノードのfutility（式と出典はsearch.rsのFUTILITY_MAX_DEPTHのコメント）
    FUTILITY_BASE: crate::value::Value = 42, [0, 300];
    FUTILITY_NO_BEST: crate::value::Value = 149, [0, 500];
    FUTILITY_MARGIN: crate::value::Value = 124, [30, 400];
    FUTILITY_OVER_ALPHA: crate::value::Value = 93, [0, 300];
    // razoring（ADR-0057, 0109のG4。yaneuraou-search.cpp:3191-3192）。
    // 評価が `alpha - RAZOR_BASE - RAZOR_DEPTH_COEF*depth^2` を下回るなら
    // 通常探索をやめてqsearchの値を返す。深さの上限はなく、マージンが
    // depthの2乗で伸びる
    RAZOR_BASE: crate::value::Value = 483, [100, 1500];
    RAZOR_DEPTH_COEF: crate::value::Value = 299, [50, 1000];
    // SEEベースの枝刈り（ADR-0090, 0109）。移動先での駒の取り合いを静的に
    // 解き、この額より損をする手を捨てる。出典はやねうら王の
    // `-25*lmrDepth^2`（静かな手。yaneuraou-search.cpp:3697）と
    // `-max(167*depth + captHist*34/1024, 0)`（取る手・王手する手。
    // yaneuraou-search.cpp:3631）。閾値が負なので「多少の駒損は許し、
    // 大きな損だけ刈る」
    SEE_QUIET_COEF: i32 = 23, [5, 100];
    SEE_CAPTURE_COEF: i32 = 166, [40, 500];
    SEE_CAPT_HIST: i32 = 37, [0, 150];
    // 取る手のfutility（式と出典はsearch.rsのCAPT_FUTILITY_MAX_DEPTH群のコメント）
    CAPT_FUTILITY_BASE: crate::value::Value = 201, [0, 800];
    CAPT_FUTILITY_DEPTH: crate::value::Value = 199, [50, 800];
    // 静止探索のfutility（ADR-0077）。stand patにQS_FUTILITY_MARGINを
    // 足した額を上限とし、取る駒の価値を足してもalphaに届かない手を捨てる。
    // 出典はやねうら王の `futilityBase = staticEval + 328`。
    // QS_SEE_MARGINは静止探索で読む取る手のSEE下限
    // （yaneuraou-search.cpp:4989）。歩損（-90）は下回るので、歩損は許す
    QS_FUTILITY_MARGIN: crate::value::Value = 302, [80, 1000];
    QS_SEE_MARGIN: crate::value::Value = -60, [-300, 50];
    // aspirationの初期窓（ADR-0109のG9。yaneuraou-search.cpp:1670-1673）。
    // 幅は `ASPIRATION_BASE + threadIdx%8 + |二乗平均スコア|/ASPIRATION_MSS_DIV`
    // で、評価値が大きいほど広がる。外したら幅へ `幅/ASPIRATION_GROWTH_DIV`
    // を足して読み直す（yaneuraou-search.cpp:1795）
    ASPIRATION_BASE: crate::value::Value = 6, [2, 30];
    ASPIRATION_MSS_DIV: crate::value::Value = 8927, [2000, 30000];
    ASPIRATION_GROWTH_DIV: crate::value::Value = 2, [2, 8];
    // LMR。LMR_COEFはリダクション表の係数（1024倍固定小数の分子。
    // ADR-0076。search.rsのbuild_reductionsが使う）。
    // LMR_DEPTH_HIST_DIVISORはhistoryによるlmrDepth補正の除数
    // （ADR-0109のG3。yaneuraou-search.cpp:3661。continuation+pawn historyの
    // 和にmain historyの `71/32` を足した値をこれで割り、lmrDepthへ加える）
    LMR_COEF: i32 = 3182, [1000, 6000];
    LMR_DEPTH_HIST_DIVISOR: i32 = 3595, [800, 12000];
}

#[cfg(all(test, feature = "tune"))]
mod tests {
    use super::*;

    #[test]
    fn set_updates_and_clamps() {
        // 既定値へ依存させない。SPSAの採択で既定値は書き換わる（ADR-0143）
        let e = ENTRIES.iter().find(|e| e.name == "RFP_MULT").unwrap();
        assert_eq!(i64::from(RFP_MULT()), e.default);
        assert!(set("RFP_MULT", e.default + 1));
        assert_eq!(i64::from(RFP_MULT()), e.default + 1);
        // 可動域の外はclampされる
        assert!(set("RFP_MULT", e.max + 10_000));
        assert_eq!(i64::from(RFP_MULT()), e.max);
        assert!(set("RFP_MULT", e.default));
        assert_eq!(i64::from(RFP_MULT()), e.default);
    }

    #[test]
    fn unknown_name_is_rejected() {
        assert!(!set("NO_SUCH_TUNABLE", 1));
    }

    #[test]
    fn entry_defaults_are_within_range() {
        for e in ENTRIES {
            assert!(e.min <= e.default && e.default <= e.max, "{}", e.name);
        }
    }
}
