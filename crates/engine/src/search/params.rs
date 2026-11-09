//! 探索の定数とLMRのリダクション表。

#[allow(clippy::wildcard_imports)]
use crate::tunables::*;
use crate::value::Value;

// ---- 探索定数（調整は1調整=1SPRT） ----
//
// SPSAの対象にした定数は `tunables.rs` にある。以下の
// コメントが式ごと説明し、値はそちらが持つ。ここに残るのは対象外の定数。

/// NMP（yaneuraou-search.cpp:3236-3301）。
/// 発動条件はcutNodeで、静的評価が `beta - 16*depth - 53*improving + 378`
/// 以上のとき。リダクションは `7 + depth/3`。深さが16以上なら、同じ深さの
/// 検証探索でzugzwangの誤りを確かめる。評価値は歩=90スケールで一致する
/// ため絶対値のまま使う
pub(super) const NMP_EVAL_DEPTH: Value = 16;
pub(super) const NMP_VERIFY_MIN_DEPTH: u32 = 16;
/// 子ノードのfutility（RFP。yaneuraou-search.cpp:3217-3227）。
/// 係数 `m = 76 - 21*(TT不ヒット)` を置くと、マージンは
/// `m*depth - (2686*improving + 362*opponentWorsening)*m/1024 + |correction値|/180600`
/// になる。これを評価から引いてもβを超えるなら刈る。返り値は `(2β+eval)/3`。
/// 評価値は歩=90スケールで一致するため絶対値のまま使う。
/// correction historyの分母131072はG1で参照実装へ揃えたので、除数180600も
/// 換算せずに使える
pub(super) const RFP_MAX_DEPTH: u32 = 15;
pub(super) const RFP_CORR_DIVISOR: i32 = 180600;
/// 親ノードfutilityの上限深さとマージン（
/// yaneuraou-search.cpp:3665-3676）。尺度はdepthではなく履歴で補正した
/// lmrDepthである。マージンは
/// `42 + 151*(最善手未発見) + 120*lmrDepth + 86*(staticEval > alpha)` で、
/// これを静的評価へ足してもalphaに届かない手を刈る。評価値は歩=90の
/// スケールで一致する
pub(super) const FUTILITY_MAX_DEPTH: i32 = 13;
/// 「今読んでいる手」をinfoで出し始める経過時間。
/// USIの慣例に合わせ、短い探索では出さない
pub(super) const CURRMOVE_MIN_MS: u64 = 3000;
/// IIR（yaneuraou-search.cpp:3319-3320）。
/// TTに手がないノードを1浅く読む。前回PVの上・allNode・1手前を大きく
/// 削って来たノードは対象外
pub(super) const IIR_MIN_DEPTH: u32 = 6;
pub(super) const IIR_MAX_PRIOR_REDUCTION: i32 = 3;
/// ProbCut（yaneuraou-search.cpp:3357-3424）。
/// 閾値は `beta + 224 - 61*improving`、確認探索の深さは `depth - 4`。
/// MovePickerへ渡すSEEの閾値は `閾値 - 静的評価` である。評価値は歩=90
/// スケールで一致するため絶対値のまま使う
pub(super) const PROBCUT_MARGIN: Value = 224;
pub(super) const PROBCUT_IMPROVING: Value = 61;
pub(super) const PROBCUT_MIN_DEPTH: u32 = 3;
pub(super) const PROBCUT_DEPTH_REDUCTION: i32 = 4;
/// singular extension（yaneuraou-search.cpp:3747-3758）。
/// TT手を除外した検証探索を `ttValue - (60 + 66*(ttPvかつ非PV)) * depth / 55`
/// の窓で行う。深さの下限は `6 + ttPv` である。参照実装はチェスの
/// `2 * depth` では「1手以外はすべてそれぐらい悪い」ため大半がsingularに
/// なると書き、1割ぐらいがsingularになる係数へ調整している
/// （yaneuraou-search.cpp:3728-3731）。評価値は歩=90スケールで一致する
/// ため絶対値のまま使う
pub(super) const SINGULAR_MIN_DEPTH: u32 = 6;
pub(super) const SINGULAR_MARGIN: Value = 60;
pub(super) const SINGULAR_MARGIN_TTPV: Value = 66;
pub(super) const SINGULAR_MARGIN_DIV: Value = 55;
/// correction historyの合成重み。分母は131072。
/// 出典はやねうら王の `correction_value()`（yaneuraou-search.cpp:737）。
/// 6要素（歩・小駒・先手非歩・後手非歩・2手前と4手前のcontinuation）を
/// この重みで合成する
pub(super) const CORR_W_PAWN: i32 = 12153;
pub(super) const CORR_W_MINOR: i32 = 8620;
pub(super) const CORR_W_NON_PAWN: i32 = 12355;
pub(super) const CORR_W_CONT: i32 = 7982;
pub(super) const CORR_DIVISOR: i32 = 131072;
/// 1手前の指し手がないときのcontinuation項の代替値
/// （yaneuraou-search.cpp:735）。
pub(super) const CORR_CONT_DEFAULT: i32 = 8;
/// 取る手のfutility（yaneuraou-search.cpp:3618-3619）。
/// `staticEval + 218 + 223*lmrDepth + 取った駒の価値 + 131*captHist/1024`
/// がalpha以下なら刈る。評価値は歩=90スケールで一致する
pub(super) const CAPT_FUTILITY_MAX_DEPTH: i32 = 7;
pub(super) const CAPT_FUTILITY_HIST: i32 = 131;
/// 静かな手のcontinuation history枝刈り（/// yaneuraou-search.cpp:3650）。1手前・2手前のcontinuation historyと
/// pawn historyの和が `-4097 * depth` を下回る手は読まない
pub(super) const CONT_HIST_PRUNE_COEF: i32 = 4097;
/// 置換表の下界を使った簡易ProbCut。探索を伴わず、
/// TTに `beta + このマージン` 以上の下界が depth-4 以上の深さで
/// 記録されていればカットする。出典はやねうら王の `beta + 416`。
/// 評価値は歩=90スケールで一致するため絶対値のまま用いる。
pub(super) const TT_PROBCUT_MARGIN: Value = 416;
pub(super) const TT_PROBCUT_DEPTH_SLACK: u32 = 4;

/// 静止探索のmovecount制限（「3手目以降は駒価値を見ずに
/// 捨てる」）。出典はやねうら王の `moveCount > 2`
pub(super) const QS_MOVECOUNT_LIMIT: u32 = 2;

/// 置換表のdepth欄のゲタ（tt.cpp:45-66, 103-164）。参照実装も内部で
/// DEPTH_NONE（-3）分を下駄履きして符号なしで持つ。同じ表現を採る。
/// 深さの比較はすべて差なので、ゲタを履かせても置換方針は変わらない
pub(super) const TT_DEPTH_OFFSET: i32 = 3;
/// 静止探索が書き出すdepth（types.h:405のDEPTH_QS = 0）。
pub(super) const TT_DEPTH_QS: u8 = 3;
/// 探索を伴わない値を書き出すdepth（types.h:418のDEPTH_UNSEARCHED = -2）。
/// DEPTH_QSより小さいので、この値ではTTカットが起きない
pub(super) const TT_DEPTH_UNSEARCHED: u8 = 1;

/// スレッドごとの窓幅のずれ幅（yaneuraou-search.cpp:1670）。
/// 参照実装が持つ唯一の明示的なLazy SMPの多様化である
pub(super) const ASPIRATION_THREAD_SPREAD: usize = 8;

/// historyのbonus・malusを配る対象として覚えておく手数の上限
/// （yaneuraou-search.cpp:702のSEARCHEDLIST_CAPACITY）。
pub(super) const SEARCHED_LIST_CAPACITY: u32 = 32;

/// この手数に達したら静かな手の生成をやめる。
/// 出典はやねうら王の `(3 + depth * depth) / (2 - improving)`
/// （yaneuraou-search.cpp:3593）。
/// 除数が実行時に決まると `udiv` が出るので、2で割る側だけシフトへ置き換える。
/// `u32` の右シフトは切り捨て除算と完全に同値である（符号付きなら負の側で
/// 丸めの向きが変わるため、この置き換えはできない）
pub(super) fn lmp_limit(depth: u32, improving: bool) -> u32 {
    let base = 3 + depth * depth;
    if improving { base } else { base >> 1 }
}

/// LMRのリダクション表の要素数。深さと手数の両方でこの表を引くので、
/// 生成できる手数の上限（`MoveList` の608）に合わせる。
/// 参照実装も `std::array<int, MAX_MOVES>` である
/// （yaneuraou-search.h:582）。
pub(super) const REDUCTIONS_LEN: usize = 608;

/// LMRのリダクション表（G2。yaneuraou-search.cpp:2168-2169）。
/// `LMR_COEF / 128 × ln(i)`（既定2763）を整数化した1次元表で、深さと
/// 手数の積を取る。
/// 積が1024倍の固定小数になることは確かめてある。
///
/// 要素はi16で持つ。値域は0〜138（`i = 607` が最大）で
/// i16に収まり、表が2.4KBから1.2KBへ半分になる。読み出し側は `i32::from`
/// で広げてから掛けるので、値も計算結果も従来とビット一致する。
#[cfg(not(feature = "tune"))]
static REDUCTIONS: std::sync::OnceLock<[i16; REDUCTIONS_LEN]> = std::sync::OnceLock::new();

fn build_reductions() -> [i16; REDUCTIONS_LEN] {
    let mut t = [0i16; REDUCTIONS_LEN];
    for (i, r) in t.iter_mut().enumerate().skip(1) {
        let v = (f64::from(LMR_COEF()) / 128.0 * (i as f64).ln()) as i32;
        debug_assert!(
            i32::from(i16::MIN) <= v && v <= i32::from(i16::MAX),
            "リダクション表の値がi16の範囲を超えた: i={i}, v={v}"
        );
        *r = v as i16;
    }
    t
}

/// リダクション表への参照を得る。`f64::ln` を含むのでconstにできず、
/// 実行時に1回だけ作る。`Worker::new` が参照を受け取って持ち回るので、
/// 指し手ごとに `OnceLock` の初期化済み判定を通ることはない
#[cfg(not(feature = "tune"))]
pub(super) fn reductions_table() -> &'static [i16; REDUCTIONS_LEN] {
    REDUCTIONS.get_or_init(build_reductions)
}

/// tuneビルド用。`LMR_COEF` がsetoptionで変わりうるので、`Worker::new` の
/// たびに係数を見て、変わっていたら表を作り直す。前の表は解放せずに漏らすが、
/// 漏れるのは係数を変えた回数ぶんの1.2KBで、チューニング走行に限る
#[cfg(feature = "tune")]
pub(super) fn reductions_table() -> &'static [i16; REDUCTIONS_LEN] {
    static CURRENT: std::sync::Mutex<Option<(i32, &'static [i16; REDUCTIONS_LEN])>> =
        std::sync::Mutex::new(None);
    let coef = LMR_COEF();
    let mut cur = CURRENT.lock().unwrap();
    match *cur {
        Some((c, table)) if c == coef => table,
        _ => {
            let table: &'static [i16; REDUCTIONS_LEN] = Box::leak(Box::new(build_reductions()));
            *cur = Some((coef, table));
            table
        }
    }
}
