//! 探索v1（ADR-0024, 0026）。
//!
//! fail-softのalpha-beta＋反復深化＋aspiration window。
//! PVは三角配列（Vecの連結）。詰みスコアはTT境界でply補正する。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};

use himawari_core::{
    Color, Move, MoveList, Piece, PieceType, Position, Repetition, Square, generate_legal,
};

use crate::eval::Evaluator;
use crate::movepick::{
    ContinuationCorrectionHistory, ContinuationHistory, Histories, LOW_PLY_HISTORY_SIZE, MoveBuf,
    MovePicker, SharedHistories,
};
use crate::timeman::{IterationStats, Limits, TimeManager};
use crate::tt::{Bound, EvalHash, Tt};
use crate::value::{
    MAX_PLY, PAWN_VALUE, VALUE_DRAW, VALUE_INFINITE, VALUE_MATE_IN_MAX_PLY, VALUE_MATED_IN_MAX_PLY,
    VALUE_NONE, VALUE_SUPERIOR, VALUE_ZERO, Value, mate_in, mated_in, value_from_tt, value_to_tt,
};

// ---- 探索定数（ADR-0028。調整は1調整=1SPRT） ----

/// NMP（ADR-0028, 0109のG4。yaneuraou-search.cpp:3236-3301）。
/// 発動条件はcutNodeで、静的評価が `beta - 16*depth - 53*improving + 378`
/// 以上のとき。リダクションは `7 + depth/3`。深さが16以上なら、同じ深さの
/// 検証探索でzugzwangの誤りを確かめる。評価値は歩=90スケールで一致する
/// ため絶対値のまま使う（ADR-0074）
const NMP_EVAL_DEPTH: Value = 16;
const NMP_EVAL_IMPROVING: Value = 53;
const NMP_EVAL_BASE: Value = 378;
const NMP_BASE_REDUCTION: u32 = 7;
const NMP_DEPTH_DIVISOR: u32 = 3;
const NMP_VERIFY_MIN_DEPTH: u32 = 16;
/// 子ノードのfutility（RFP。ADR-0109のG4。yaneuraou-search.cpp:3217-3227）。
/// 係数 `m = 76 - 21*(TT不ヒット)` を置くと、マージンは
/// `m*depth - (2686*improving + 362*opponentWorsening)*m/1024 + |correction値|/180600`
/// になる。これを評価から引いてもβを超えるなら刈る。返り値は `(2β+eval)/3`。
/// 評価値は歩=90スケールで一致するため絶対値のまま使う（ADR-0074）。
/// correction historyの分母131072はG1で参照実装へ揃えたので、除数180600も
/// 換算せずに使える（ADR-0109の「定数の扱い」）
const RFP_MAX_DEPTH: u32 = 15;
const RFP_MULT: i32 = 76;
const RFP_NO_TT_HIT: i32 = 21;
const RFP_IMPROVING: i32 = 2686;
const RFP_OPP_WORSENING: i32 = 362;
const RFP_CORR_DIVISOR: i32 = 180600;
/// 親ノードfutilityの上限深さとマージン（ADR-0028, 0109のG3。
/// yaneuraou-search.cpp:3665-3676）。尺度はdepthではなく履歴で補正した
/// lmrDepthである。マージンは
/// `42 + 151*(最善手未発見) + 120*lmrDepth + 86*(staticEval > alpha)` で、
/// これを静的評価へ足してもalphaに届かない手を刈る。評価値は歩=90の
/// スケールで一致する（ADR-0074）
const FUTILITY_MAX_DEPTH: i32 = 13;
const FUTILITY_BASE: Value = 42;
const FUTILITY_NO_BEST: Value = 151;
const FUTILITY_MARGIN: Value = 120;
const FUTILITY_OVER_ALPHA: Value = 86;
/// 「今読んでいる手」をinfoで出し始める経過時間（ADR-0086）。
/// USIの慣例に合わせ、短い探索では出さない
const CURRMOVE_MIN_MS: u64 = 3000;
/// IIR（ADR-0028, 0109のG4。yaneuraou-search.cpp:3319-3320）。
/// TTに手がないノードを1浅く読む。前回PVの上・allNode・1手前を大きく
/// 削って来たノードは対象外
const IIR_MIN_DEPTH: u32 = 6;
const IIR_MAX_PRIOR_REDUCTION: i32 = 3;
/// ProbCut（ADR-0051, 0109のG4。yaneuraou-search.cpp:3357-3424）。
/// 閾値は `beta + 224 - 61*improving`、確認探索の深さは `depth - 4`。
/// MovePickerへ渡すSEEの閾値は `閾値 - 静的評価` である。評価値は歩=90
/// スケールで一致するため絶対値のまま使う（ADR-0074）
const PROBCUT_MARGIN: Value = 224;
const PROBCUT_IMPROVING: Value = 61;
const PROBCUT_MIN_DEPTH: u32 = 3;
const PROBCUT_DEPTH_REDUCTION: i32 = 4;
/// singular extension（ADR-0050, 0109のG5。yaneuraou-search.cpp:3747-3758）。
/// TT手を除外した検証探索を `ttValue - (60 + 66*(ttPvかつ非PV)) * depth / 55`
/// の窓で行う。深さの下限は `6 + ttPv` である。参照実装はチェスの
/// `2 * depth` では「1手以外はすべてそれぐらい悪い」ため大半がsingularに
/// なると書き、1割ぐらいがsingularになる係数へ調整している
/// （yaneuraou-search.cpp:3728-3731）。評価値は歩=90スケールで一致する
/// ため絶対値のまま使う（ADR-0074）
const SINGULAR_MIN_DEPTH: u32 = 6;
const SINGULAR_MARGIN: Value = 60;
const SINGULAR_MARGIN_TTPV: Value = 66;
const SINGULAR_MARGIN_DIV: Value = 55;
/// correction historyの合成重み（ADR-0085, 0109）。分母は131072。
/// 出典はやねうら王の `correction_value()`（yaneuraou-search.cpp:737）。
/// 6要素（歩・小駒・先手非歩・後手非歩・2手前と4手前のcontinuation）を
/// この重みで合成する
const CORR_W_PAWN: i32 = 12153;
const CORR_W_MINOR: i32 = 8620;
const CORR_W_NON_PAWN: i32 = 12355;
const CORR_W_CONT: i32 = 7982;
const CORR_DIVISOR: i32 = 131072;
/// 1手前の指し手がないときのcontinuation項の代替値
/// （yaneuraou-search.cpp:735）。
const CORR_CONT_DEFAULT: i32 = 8;
/// SEEベースの枝刈り（ADR-0090, 0109）。移動先での駒の取り合いを静的に
/// 解き、この額より損をする手を捨てる。出典はやねうら王の
/// `-25*lmrDepth^2`（静かな手。yaneuraou-search.cpp:3697）と
/// `-max(167*depth + captHist*34/1024, 0)`（取る手・王手する手。
/// yaneuraou-search.cpp:3631）。SEEの駒価値は歩=90でやねうら王と
/// 同系列のため絶対値のまま使える（ADR-0074）。閾値が負なので
/// 「多少の駒損は許し、大きな損だけ刈る」
const SEE_QUIET_COEF: i32 = 25;
const SEE_CAPTURE_COEF: i32 = 167;
const SEE_CAPT_HIST: i32 = 34;
/// 取る手のfutility（ADR-0109のG3。yaneuraou-search.cpp:3618-3619）。
/// `staticEval + 218 + 223*lmrDepth + 取った駒の価値 + 131*captHist/1024`
/// がalpha以下なら刈る。評価値は歩=90スケールで一致する（ADR-0074）
const CAPT_FUTILITY_MAX_DEPTH: i32 = 7;
const CAPT_FUTILITY_BASE: Value = 218;
const CAPT_FUTILITY_DEPTH: Value = 223;
const CAPT_FUTILITY_HIST: i32 = 131;
/// 静かな手のcontinuation history枝刈り（ADR-0109のG3。
/// yaneuraou-search.cpp:3650）。1手前・2手前のcontinuation historyと
/// pawn historyの和が `-4097 * depth` を下回る手は読まない
const CONT_HIST_PRUNE_COEF: i32 = 4097;
/// historyによるlmrDepth補正の除数（ADR-0109のG3。
/// yaneuraou-search.cpp:3661）。上の和にmain historyの `71/32`
/// （yaneuraou-search.cpp:3656）を足した値をこれで割り、lmrDepthへ
/// 加える。この補正の後で親futilityと静かな手のSEE枝刈りが同じ
/// lmrDepthを読むので、順序を入れ替えてはならない
const LMR_DEPTH_HIST_DIVISOR: i32 = 3220;
/// razoring（ADR-0057, 0109のG4。yaneuraou-search.cpp:3191-3192）。
/// 評価が `alpha - 502 - 306*depth^2` を下回るなら通常探索をやめて
/// qsearchの値を返す。深さの上限はなく、マージンがdepthの2乗で伸びる。
/// 評価値は歩=90スケールで一致するため絶対値のまま使う（ADR-0074）
const RAZOR_BASE: Value = 502;
const RAZOR_DEPTH_COEF: Value = 306;
/// 静止探索のfutility（ADR-0077）。stand patにこの値を足した額を上限とし、
/// 取る駒の価値を足してもalphaに届かない手を捨てる。movecount制限は
/// 「3手目以降は駒価値を見ずに捨てる」。出典はやねうら王の
/// `futilityBase = staticEval + 328` と `moveCount > 2`。評価値は歩=90
/// スケールで一致するため絶対値のまま用いる（ADR-0074）。
/// 置換表の下界を使った簡易ProbCut（ADR-0078）。探索を伴わず、
/// TTに `beta + このマージン` 以上の下界が depth-4 以上の深さで
/// 記録されていればカットする。出典はやねうら王の `beta + 416`。
/// 評価値は歩=90スケールで一致するため絶対値のまま用いる（ADR-0074）。
const TT_PROBCUT_MARGIN: Value = 416;
const TT_PROBCUT_DEPTH_SLACK: u32 = 4;

const QS_FUTILITY_MARGIN: Value = 328;
const QS_MOVECOUNT_LIMIT: u32 = 2;
/// 静止探索で探索する取る手のSEE下限（yaneuraou-search.cpp:4989）。
/// 出典のPawnValueは90で、本エンジンの歩の価値と一致するため絶対値のまま
/// 用いる（ADR-0074）。歩損（-90）は下回るので、歩損は許す下限である
const QS_SEE_MARGIN: Value = -73;

/// 置換表のdepth欄のゲタ（tt.cpp:45-66, 103-164）。参照実装も内部で
/// DEPTH_NONE（-3）分を下駄履きして符号なしで持つ。同じ表現を採る。
/// 深さの比較はすべて差なので、ゲタを履かせても置換方針は変わらない
const TT_DEPTH_OFFSET: i32 = 3;
/// 静止探索が書き出すdepth（types.h:405のDEPTH_QS = 0）。
const TT_DEPTH_QS: u8 = 3;
/// 探索を伴わない値を書き出すdepth（types.h:418のDEPTH_UNSEARCHED = -2）。
/// DEPTH_QSより小さいので、この値ではTTカットが起きない
const TT_DEPTH_UNSEARCHED: u8 = 1;

/// aspirationの初期窓（ADR-0109のG9。yaneuraou-search.cpp:1670-1673）。
/// 幅は `5 + threadIdx%8 + |二乗平均スコア|/9000` で、評価値が大きいほど
/// 広がる。中心は前深さの生スコアではなくスコアの移動平均に置く。
/// 外したら幅を4/3倍にして読み直す（yaneuraou-search.cpp:1795）。
/// 評価値は歩=90スケールで参照実装と一致するため、除数9000は換算せずに
/// 使える（ADR-0074）
const ASPIRATION_BASE: Value = 5;
const ASPIRATION_MSS_DIV: Value = 9000;
const ASPIRATION_GROWTH_DIV: Value = 3;
/// スレッドごとの窓幅のずれ幅（yaneuraou-search.cpp:1670）。
/// 参照実装が持つ唯一の明示的なLazy SMPの多様化である（ADR-0031）
const ASPIRATION_THREAD_SPREAD: usize = 8;

/// historyのbonus・malusを配る対象として覚えておく手数の上限
/// （yaneuraou-search.cpp:702のSEARCHEDLIST_CAPACITY）。
const SEARCHED_LIST_CAPACITY: u32 = 32;

/// この手数に達したら静かな手の生成をやめる（ADR-0109のG1）。
/// 出典はやねうら王の `(3 + depth * depth) / (2 - improving)`
/// （yaneuraou-search.cpp:3593）。
/// 除数が実行時に決まると `udiv` が出るので、2で割る側だけシフトへ置き換える。
/// `u32` の右シフトは切り捨て除算と完全に同値である（符号付きなら負の側で
/// 丸めの向きが変わるため、この置き換えはできない）
fn lmp_limit(depth: u32, improving: bool) -> u32 {
    let base = 3 + depth * depth;
    if improving { base } else { base >> 1 }
}

/// LMRのリダクション表の要素数。深さと手数の両方でこの表を引くので、
/// 生成できる手数の上限（`MoveList` の608）に合わせる。
/// 参照実装も `std::array<int, MAX_MOVES>` である
/// （yaneuraou-search.h:582）。
const REDUCTIONS_LEN: usize = 608;

/// LMRのリダクション表（G2。yaneuraou-search.cpp:2168-2169）。
/// `2763 / 128 × ln(i)` を整数化した1次元表で、深さと手数の積を取る。
/// 積が1024倍の固定小数になるスケールはADR-0076で確認済み。
///
/// 要素はi16で持つ（ADR-0151群H）。値域は0〜138（`i = 607` が最大）で
/// i16に収まり、表が2.4KBから1.2KBへ半分になる。読み出し側は `i32::from`
/// で広げてから掛けるので、値も計算結果も従来とビット一致する。
static REDUCTIONS: std::sync::OnceLock<[i16; REDUCTIONS_LEN]> = std::sync::OnceLock::new();

/// リダクション表への参照を得る。`f64::ln` を含むのでconstにできず、
/// 実行時に1回だけ作る。`Worker::new` が参照を受け取って持ち回るので、
/// 指し手ごとに `OnceLock` の初期化済み判定を通ることはない
fn reductions_table() -> &'static [i16; REDUCTIONS_LEN] {
    REDUCTIONS.get_or_init(|| {
        let mut t = [0i16; REDUCTIONS_LEN];
        for (i, r) in t.iter_mut().enumerate().skip(1) {
            let v = (2763.0 / 128.0 * (i as f64).ln()) as i32;
            debug_assert!(
                i32::from(i16::MIN) <= v && v <= i32::from(i16::MAX),
                "リダクション表の値がi16の範囲を超えた: i={i}, v={v}"
            );
            *r = v as i16;
        }
        t
    })
}

/// スレッド間の共有状態（ADR-0020）。
pub struct Shared {
    pub stop: AtomicBool,
    /// 探索を破棄した（反復の途中で止めた）ことを示す（thread.h:296-301）。
    /// 中断した反復のスコアは信用できないので、前の反復へ戻す判断に使う
    pub aborted_search: AtomicBool,
    /// go ponder中か（yaneuraou-search.h:203-214の `SearchManager::ponder`）。
    /// 真の間は時間で探索を止めない。"ponderhit" で偽になる。"stop" では
    /// 変えない（参照実装も同じ。stopは `stop` フラグ側で止まる）
    pub ponder: AtomicBool,
    /// "ponderhit" した時刻の、go受領時刻からの経過[ms]（timeman.h:120の
    /// `ponderhitTime - startTime`）。"ponderhit" を受けるまでは0
    pub ponderhit_offset: AtomicI64,
    pub nodes: AtomicU64,
    /// 反復深化で深さが増えているか（G9。yaneuraou-search.h:232）。
    /// メインが時間の余りから決め、全スレッドが次の反復の頭で読む。
    /// 偽なら同じ深さを掘り直したとみなし `search_again_counter` が増える
    pub increase_depth: AtomicBool,
    /// rootの最善手が変わった回数（G9。yaneuraou-search.h:539）。
    /// 参照実装はスレッドごとの原子変数を持ち、メインが毎反復で合算して
    /// 0へ戻す。本エンジンは合算先を1つにまとめ、`swap(0)` で汲み出す
    pub best_move_changes: AtomicU64,
    pub tt: Tt,
    /// 評価値キャッシュ（ADR-0049）。全スレッド共有、new_gameでクリア。
    pub eval_hash: EvalHash,
    /// 全スレッドで共有するhistory（ADR-0162）。pawn historyと
    /// correction historyの2面を持ち、表はスレッド数に比例して伸びる
    pub hists: Arc<SharedHistories>,
}

impl Shared {
    pub fn new(hash_mb: usize) -> Shared {
        Shared::with_threads(hash_mb, 1)
    }

    pub fn with_threads(hash_mb: usize, threads: usize) -> Shared {
        Shared {
            hists: Arc::new(SharedHistories::new(threads)),
            stop: AtomicBool::new(false),
            aborted_search: AtomicBool::new(false),
            ponder: AtomicBool::new(false),
            ponderhit_offset: AtomicI64::new(0),
            nodes: AtomicU64::new(0),
            increase_depth: AtomicBool::new(true),
            best_move_changes: AtomicU64::new(0),
            tt: Tt::new(hash_mb),
            eval_hash: EvalHash::new(),
        }
    }
}

/// 探索中の報告（ADR-0086）。反復深化1周分の結果と、rootで今読んで
/// いる手の2種類がある。呼び出し側はどちらもUSIのinfo行へ落とす。
pub enum SearchInfo {
    /// 反復深化1周分（MultiPVでは1ラインごとに1回）。
    Iteration(IterInfo),
    /// aspiration窓を外れた途中経過（ADR-0091）。確定値ではないため、
    /// 消費側はUSIの `lowerbound` / `upperbound` を付けて出す。
    Bound(IterInfo, ScoreBound),
    /// rootで今読んでいる手（ADR-0086）。長考中の可視化に使う。
    CurrMove { depth: u32, mv: Move },
}

/// aspiration窓を外れたときのスコアの確からしさ（ADR-0091）。
/// USIの `lowerbound` / `upperbound` に対応する。
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ScoreBound {
    /// fail high。実際の評価はこの値以上。
    Lower,
    /// fail low。実際の評価はこの値以下。
    Upper,
}

/// 反復深化1周分の報告（MultiPVでは1ラインごとに1回）。
pub struct IterInfo {
    pub depth: u32,
    /// 静止探索を含めて到達した最大ply（ADR-0086）。
    pub seldepth: u32,
    /// 1始まりのライン番号。MultiPV=1のときは0（info出力で省略）。
    pub multipv: usize,
    pub score: Value,
    pub pv: Vec<Move>,
    pub nodes: u64,
    pub elapsed_ms: u64,
    pub hashfull: usize,
}

/// root手1つの探索状態（ADR-0032）。
pub struct RootMove {
    pub mv: Move,
    pub score: Value,
    pub prev_score: Value,
    pub pv: Vec<Move>,
    /// この手の探索に費やしたノード数（ADR-0062、G9。search.h:129）。
    /// メインワーカーのローカル値のみ。**go全体の累計で、イテレーション
    /// ごとには戻さない。** highBestMoveEffortが読む比の分母も同じく
    /// go全体の累計ノード数である（S:1969-1970）
    pub effort: u64,
    /// スコアの移動平均（G9。search.h:139、yaneuraou-search.cpp:4105-4106）。
    /// aspirationの窓の中心に使う。前深さの生スコアより揺れが小さい。
    /// 初期値の `-VALUE_INFINITE` は「まだ値がない」ことを表す番兵
    pub average_score: Value,
    /// スコアの二乗平均（G9。search.h:142、yaneuraou-search.cpp:4108-4110）。
    /// 符号を保つため `value * |value|` を平均する。aspirationの初期窓幅が
    /// この絶対値に比例して広がる。初期値の `-VALUE_INFINITE^2` は番兵で、
    /// **この値のまま窓幅を計算すると窓が全開になる。** 深さ1で
    /// aspirationを事実上無効にする仕掛けである
    pub mean_squared_score: Value,
}

/// `RootMove::mean_squared_score` の番兵（search.h:142）。
const MEAN_SQUARED_NONE: Value = -(VALUE_INFINITE * VALUE_INFINITE);

/// goをまたいでメインスレッドが持ち越す記憶（G9。yaneuraou-search.h:207-247）。
/// 参照実装はSearchManagerのメンバーで、`isready` でクリアされる。
/// 本エンジンはスレッドループが持ち、`usinewgame` でクリアする
pub struct MainMemory {
    /// 前回のgoの最終スコア（yaneuraou-search.h:215）。`iter_value` の
    /// 初期値に使う。`VALUE_INFINITE` は「前回がない」ことを表す番兵
    pub best_previous_score: Value,
    /// 前回のgoの最善手の `average_score`（yaneuraou-search.h:216）。
    /// 思考時間のfallingEvalが読む。番兵は同じく `VALUE_INFINITE` で、
    /// **番兵のままだとfallingEvalが上限に張り付き、初手に時間を多く使う。**
    /// 参照実装のコメントが明示する意図的な挙動である（S:281-285）
    pub best_previous_average_score: Value,
    /// 前回のgoのtimeReduction（yaneuraou-search.h:211）
    pub previous_time_reduction: f64,
    /// 前回のgoのgame_ply（yaneuraou-search.h:247）。手番が入れ替わって
    /// いないかの検出に使う
    pub last_game_ply: u16,
}

impl Default for MainMemory {
    /// 参照実装の `YaneuraOuEngine::clear()`（S:281-292）と同じ初期値。
    fn default() -> Self {
        MainMemory {
            best_previous_score: VALUE_INFINITE,
            best_previous_average_score: VALUE_INFINITE,
            previous_time_reduction: 0.85,
            last_game_ply: 0,
        }
    }
}

pub struct SearchResult {
    pub best: Move,
    pub score: Value,
    /// 相手の予測応手（PVの2手目。なければNONE。ADR-0033）。
    pub ponder: Move,
    /// `root_moves[0]` の生スコア（G10。S:610-611の投票が読む）。
    /// USIへ出す `score` と違い、頭打ちや窓外れの整形をしていない
    pub root_score: Value,
    /// `root_moves[0].average_score`（G10。S:1250）。
    /// best threadのものを次のgoへ持ち越す
    pub root_average_score: Value,
    /// `root_moves[0].pv`（G10。S:625-626の投票が読む）。
    pub pv: Vec<Move>,
    /// 確定した最後のイテレーションの深さ（G10。S:618の投票が読む）。
    pub completed_depth: u32,
}

/// plyごとの探索状態（ADR-0109のG0, G1）。
///
/// 参照実装のStackに対応する。statScore・moveCount・ttPvなどは、
/// 読む側を入れる群で足す。
#[derive(Clone, Copy)]
struct StackEntry {
    /// このplyで指した手。null moveはMove::NULL、rootの手前はMove::NONE
    current_move: Move,
    /// singular検証探索中の除外手（ADR-0050）
    excluded_move: Move,
    /// 静的評価。王手中はVALUE_NONE
    static_eval: Value,
    /// このplyで手番側に王手がかかっていたか（G1）。continuation historyの
    /// 面の選択と、更新する段数の打ち切りに使う
    in_check: bool,
    /// このplyで指した手が条件手になるcontinuation historyの面（G1）。
    /// 参照実装の `Stack::continuationHistory` に対応する
    cont_base: usize,
    /// 同じくcontinuation correction historyの面（G1）。
    cont_corr_base: usize,
    /// このplyで今何手目を調べているか（G1。yaneuraou-search.cpp:3520）。
    /// 1手前の値をhistoryの更新条件が読む
    move_count: u32,
    /// このplyで置換表にヒットしたか（G1。yaneuraou-search.cpp:2623）。
    /// 同じく1手前の値を読む
    tt_hit: bool,
    /// 置換表にPVノードとして記録された値か（G2。yaneuraou-search.cpp:2657）。
    /// LMRのリダクション2項が読み、置換表へ書き戻す
    tt_pv: bool,
    /// このplyでβカットした回数（G2。yaneuraou-search.cpp:4214）。
    /// 親が次plyの値を読む。多いほどリダクションを増やす
    cutoff_cnt: i32,
    /// このplyのLMRが削った量（G2。yaneuraou-search.cpp:3980）。
    /// 子が `priorReduction` として読み、深さの事後補正に使う（G4）
    reduction: i32,
    /// このplyで今調べている手の履歴の強さ（G2。yaneuraou-search.cpp:3924-3932）。
    /// リダクションの減算と、子でのhistory更新量の2方向へ効く
    stat_score: i32,
    /// 前回の反復深化のPV上にいるか（G3。yaneuraou-search.cpp:2370-2372）。
    /// PVノードでここが真の間は静かな手の浅い枝刈りを抑え、前回のPVを壊さない
    follow_pv: bool,
}

/// Stackの前方余白。ss-6まで境界検査なしで参照するために置く。
const STACK_OFFSET: usize = 7;

/// Stackの要素数。前方の余白と、`ply + 2` まで書く後方の余裕を含む。
/// 固定長にすることで、`stack[ply + STACK_OFFSET]` の境界検査が
/// コンパイル時定数との比較になる
const STACK_LEN: usize = MAX_PLY + 10;

/// Stackの初期値。全plyをこれで埋める。
const STACK_INIT: StackEntry = StackEntry {
    current_move: Move::NONE,
    excluded_move: Move::NONE,
    static_eval: VALUE_NONE,
    in_check: false,
    // 指し手のないplyは番兵の面を指す
    cont_base: ContinuationHistory::SENTINEL,
    cont_corr_base: 0,
    move_count: 0,
    tt_hit: false,
    tt_pv: false,
    cutoff_cnt: 0,
    reduction: 0,
    stat_score: 0,
    follow_pv: false,
};

/// 置換表から読んだ値（ADR-0022, 0024, 0125）。`probe_tt` が組み立て、
/// TTカット・枝刈り・延長・リダクションが繰り返し読む。
#[derive(Clone, Copy)]
struct TtInfo {
    /// probeの生データ。eval欄の再利用とヒットの有無の判定が読む
    hit: Option<crate::tt::TtData>,
    /// この局面で指せる形に直したTT手。指せないときは `Move::NONE`
    mv: Move,
    value: Value,
    /// ゲタを外した深さ。ヒットしないときは参照実装のDEPTH_NONE
    /// （tt.cpp:445）に合わせて -3 とする
    depth: i32,
    bound: Bound,
}

/// ムーブループの手前で決まるノードの前提（ADR-0125）。
///
/// 切り出した枝刈り・延長・リダクション・終端処理が共通して読む値を束ねる。
/// `search` の引数と局所変数をそのまま写したもので、値は組み立てたあと
/// 変わらない。ムーブループで動く `depth`・`alpha`・`improving` はここへ
/// 入れず、関数の引数で渡す。
///
/// ノードがPVかどうかはここへ入れない。`search` と同じくconst genericの
/// `PV` で受け取る（ADR-0151の群J）。
#[derive(Clone, Copy)]
struct NodeInfo {
    ply: usize,
    cut_node: bool,
    /// PVでもcutでもないノード（yaneuraou-search.cpp:2251）。
    /// 全手を調べる見込みなのでリダクションを強める。IIRの条件も読む
    all_node: bool,
    in_check: bool,
    /// 前回の反復深化のPVを辿っているか（yaneuraou-search.cpp:2370-2372）
    follow_pv: bool,
    /// 除外手（singular extension用。ADR-0050）。検証探索中はTT手が入る
    excluded: Move,
    key: u64,
    /// correction history補正前の静的評価。TT storeのeval欄へそのまま入る
    raw_eval: Value,
    /// correction history補正後の静的評価（ADR-0046）
    static_eval: Value,
    /// 枝刈り用の評価値（yaneuraou-search.cpp:3084-3087）。置換表の値の
    /// ほうが見積りとして適切なら差し替わる。razoringとRFPだけが読む
    eval: Value,
    /// correction historyの合成値（yaneuraou-search.cpp:3010）
    corr_value: i32,
    /// 1手前のLMRが削った量（yaneuraou-search.cpp:2552-2553）
    prior_reduction: i32,
    /// 1手前が取った駒（yaneuraou-search.cpp:2355）
    prior_capture: Piece,
    /// 1手前の移動先（yaneuraou-search.cpp:2550）。特殊手ならNone
    prev_sq: Option<Square>,
    tt: TtInfo,
    /// 置換表の手が駒を取る手か（yaneuraou-search.cpp:2671）。
    /// RFPの条件とリダクションの1項が読む
    tt_capture: bool,
}

/// ノードごとに使い回すバッファ（ADR-0151の群B）。
///
/// 素直に書くと毎ノードで `Vec` を確保し直すので、mallocがプロファイルの
/// 2.1%を占めていた。`Worker` がply別に持ち、`std::mem::take` で貸し出して
/// 使い終わったら戻す。takeは空との交換なので確保が起きず、戻したときに
/// 容量が残る。同じplyで次に借りるときは確保も再確保も起きない。
///
/// 添字はそのバッファを所有するノードのplyである。ノードは自分のplyの
/// スロットだけを借り、子は `ply + 1` のスロットを借りるので衝突しない。
/// 同一plyへ再帰する経路（singular検証・NMP検証）は、いずれも借りる前に
/// 呼んで戻ってくるので重ならない。
///
/// 返却漏れが起きても正しさは壊れない。次に借りるときが空の `Vec` に
/// なり、確保が1回復活するだけである。
struct NodeBuffers {
    /// 最善にならなかった静かな手（`search` のquiets_searched）
    quiets: Vec<Move>,
    /// 同じく取る手（`search` のcaptures_searched）
    captures: Vec<Move>,
    /// 子ノードのPV（`search` のchild_pv）
    child_pv: Vec<Move>,
    /// `MovePicker` へ貸す採点済みの手
    moves: MoveBuf,
}

impl NodeBuffers {
    fn with_capacity() -> NodeBuffers {
        NodeBuffers {
            // 覚える手数の上限は決まっている（yaneuraou-search.cpp:4246-4256）
            quiets: Vec::with_capacity(SEARCHED_LIST_CAPACITY as usize),
            captures: Vec::with_capacity(SEARCHED_LIST_CAPACITY as usize),
            // PVが伸びるのはPVノードだけなので、伸びたぶんを残せば足りる
            child_pv: Vec::new(),
            moves: MoveBuf::with_node_capacity(),
        }
    }
}

pub struct Worker {
    pub pos: Position,
    pub evaluator: Evaluator,
    /// historyの一式（ADR-0109のG1）。対局を通じてスレッドが持ち回る。
    pub hist: Histories,
    /// plyごとの探索状態（ADR-0109）。添字は `ply + STACK_OFFSET` で引く。
    /// 前方の余白により、ply 0でも1手前・2手前を境界検査なしで読める。
    /// 長さをコンパイル時定数にするため配列で持つ（ADR-0124）。`Vec` だと
    /// 長さがヒープ上にあり、`&mut self` を通る呼び出しのたびに読み直す
    stack: Box<[StackEntry; STACK_LEN]>,
    /// ply別の再利用バッファ（ADR-0151の群B）。`search` と `qsearch` は
    /// `ply >= MAX_PLY` で何も借りずに返すので、長さはMAX_PLYで足りる
    node_bufs: Box<[NodeBuffers; MAX_PLY]>,
    /// このイテレーションで到達した最大ply（seldepth。ADR-0086）。
    sel_depth: u32,
    /// このイテレーションのaspiration窓の幅（G2。yaneuraou-search.cpp:1708）。
    /// リダクションが「今の窓幅がroot窓幅の何割か」で削る量を決める
    root_delta: Value,
    /// 今の反復深化の深さ（G5。yaneuraou-search.h:557）。
    /// singularの多段化のマージンが `ply > rootDepth` で分岐に使う。
    /// rootから遠いノードでは延長を積みにくくする項である
    root_depth: u32,
    /// このplyに達するまでNMPを止める（G4。yaneuraou-search.h:543）。
    /// NMPの検証探索の中で立て、探索を抜けたら0へ戻す。再帰的な検証を
    /// 認めないための状態で、スレッドごとに持つ
    nmp_min_ply: usize,
    /// 前回の反復深化で得たPV（G3。yaneuraou-search.h:562）。
    /// 各ノードの `follow_pv` 判定が読む。goのたびに捨てる
    last_iteration_pv: Vec<Move>,
    /// 深さ1のイテレーションを終えたか。終えるまでstopを無視する。
    /// root手は生成順のままなので、深さ1の途中で打ち切ると探索して
    /// いない手を返してしまう
    depth1_done: bool,
    /// go ponder中に予算を使い切った（yaneuraou-search.h:220）。
    /// ponder中はGUIの指示があるまで止められないので、その場では止めず
    /// 予約だけしておく。ponderhit後の最初の判定で終了時刻が確定する。
    /// aspirationのfail lowで解除する（S:1783-1784）
    stop_on_ponderhit: bool,
    nodes: u64,
    shared: Arc<Shared>,
    tm: TimeManager,
    limits: Limits,
    max_moves_to_draw: u16,
    /// 検討モードのライン数（ADR-0032）。対局時は1。
    multi_pv: usize,
    root_moves: Vec<RootMove>,
    /// このワーカーの通し番号（G9。thread.h:198のthreadIdx）。
    /// aspirationの初期窓幅を `thread_idx % 8` だけずらす。参照実装が持つ
    /// 唯一の明示的なLazy SMPの多様化である
    thread_idx: usize,
    /// ワーカーの総数（G9）。bestMoveInstabilityの分母になる
    thread_count: usize,
    /// goをまたぐ記憶（G9）。スレッドループが持ち回る
    pub memory: MainMemory,
    /// rootの手番（G10）。引き分けの評価値の符号を決める
    root_color: Color,
    /// rootの手番から見た引き分けの評価値（G10。S:1002-1009）。
    /// `DrawValueBlack` / `DrawValueWhite` を歩の価値で換算した値で、
    /// `ThreadPool` が `set_draw_value` で入れる。既定は0（従来と同じ）
    draw_value_us: Value,
    /// LMRのリダクション表への参照。`Worker::new` で1回だけ受け取る。
    /// 表引きは指し手ごとに起きるので、そのたびに `OnceLock` を叩かない
    reductions: &'static [i16; REDUCTIONS_LEN],
}

impl Worker {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pos: Position,
        shared: Arc<Shared>,
        limits: Limits,
        tm: TimeManager,
        max_moves_to_draw: u16,
        multi_pv: usize,
        evaluator: Evaluator,
        hist: Histories,
    ) -> Worker {
        let root_color = pos.side_to_move();
        Worker {
            pos,
            evaluator,
            hist,
            // ヒープ上で作ってから配列へ移す。要素数はコンパイル時に
            // 決まっているので、この変換が失敗することはない
            stack: vec![STACK_INIT; STACK_LEN]
                .into_boxed_slice()
                .try_into()
                .unwrap_or_else(|_| unreachable!("要素数はSTACK_LENで固定")),
            node_bufs: std::iter::repeat_with(NodeBuffers::with_capacity)
                .take(MAX_PLY)
                .collect::<Vec<_>>()
                .into_boxed_slice()
                .try_into()
                .unwrap_or_else(|_| unreachable!("要素数はMAX_PLYで固定")),
            sel_depth: 0,
            // 0除算を避ける番兵。search_rootが毎回入れ直す
            root_delta: 1,
            root_depth: 0,
            nmp_min_ply: 0,
            last_iteration_pv: Vec::new(),
            depth1_done: false,
            stop_on_ponderhit: false,
            nodes: 0,
            shared,
            tm,
            limits,
            max_moves_to_draw,
            multi_pv: multi_pv.max(1),
            root_moves: Vec::new(),
            thread_idx: 0,
            thread_count: 1,
            memory: MainMemory::default(),
            root_color,
            draw_value_us: VALUE_ZERO,
            reductions: reductions_table(),
        }
    }

    /// 引き分けの評価値を設定する（G10。S:1002-1009）。手番別の設定値を
    /// 歩の価値で換算し、rootの手番から見た値として持つ。
    /// 相手番のleafでは符号を反転させる。非対称な探索を避けるためである
    pub fn set_draw_value(&mut self, black: i32, white: i32) {
        let v = if self.root_color == Color::Black {
            black
        } else {
            white
        };
        self.draw_value_us = v * PAWN_VALUE / 100;
    }

    /// このワーカーの通し番号と総数を渡す（G9）。aspirationの多様化と
    /// bestMoveInstabilityの分母に使う。既定は単スレッド相当の (0, 1)。
    pub fn set_thread(&mut self, idx: usize, count: usize) {
        self.thread_idx = idx;
        self.thread_count = count.max(1);
    }

    /// 深さ1を終えるまではstopを無視する。`iterate` は打ち切り時に
    /// `root_moves[0]` を返すが、root手は生成順に並んでいるため、深さ1の
    /// 途中で止まると探索していない手が出てしまう。深さ1は数msで終わる
    /// ので、待つ代償は小さい
    #[inline]
    fn stopped(&self) -> bool {
        self.depth1_done && self.shared.stop.load(Ordering::Relaxed)
    }

    /// 定期的な時間・ノード制限の検査（S:5480-5560）。時間制限を持つのは
    /// メインワーカーだけ（ヘルパーはtmが無制限。ADR-0020, 0031）。
    /// あわせてローカルのノード数を共有カウンタへ流し込む。
    ///
    /// 即座に止めるのは3つだけである。movetime超過、nodes超過、予約した
    /// 終了時刻の到来。`maximum()` の超過はその場では止めず、秒単位で
    /// 切り上げた終了時刻を予約する。秒ぎりぎりまで思考したほうが得だから
    #[inline]
    fn check_limits(&mut self) {
        if !self.nodes.is_multiple_of(2048) {
            return;
        }
        self.shared.nodes.fetch_add(2048, Ordering::Relaxed);
        // go ponder中はGUIの指示があるまで止めない（S:5502-5507）。
        // 時間管理そのものは働いていて、経過時間はgo受領時刻から数える
        if self.shared.ponder.load(Ordering::SeqCst) {
            return;
        }
        // 深さ1を終えるまでは止めない（S:5542の completedDepth >= 1）
        if !self.depth1_done {
            return;
        }
        let elapsed = self.tm.elapsed_ms();
        let ponderhit_offset = self.shared.ponderhit_offset.load(Ordering::SeqCst);
        if (self.limits.movetime > 0 && elapsed >= self.limits.movetime as i64)
            || (self.limits.nodes > 0 && self.nodes >= self.limits.nodes)
            // search_endは0が「未確定」を表す。負の値も予約済みとして扱う。
            // MinimumThinkingTimeを小さくするとround_upがNetworkDelayを
            // 引いて負になり、0比較では止まらなくなる
            || (self.tm.search_end != 0 && self.tm.search_end <= elapsed)
        {
            self.shared.aborted_search.store(true, Ordering::Relaxed);
            self.shared.stop.store(true, Ordering::Relaxed);
        } else if self.tm.search_end == 0
            && self.tm.use_time_management()
            // ponder中に予算を使い切っていたなら、ponderhit後の最初の
            // 判定でここへ来る（S:5551-5558の条件1と2）
            && (elapsed > self.tm.maximum() || self.stop_on_ponderhit)
        {
            self.tm.set_search_end(elapsed, ponderhit_offset);
        }
    }

    #[inline]
    fn draw_value(&self) -> Value {
        // 手番別の引き分けスコア（G10。S:1008-1009、S:2468）。
        // rootと同じ手番なら+、相手番なら-にする
        let base = if self.pos.side_to_move() == self.root_color {
            self.draw_value_us
        } else {
            -self.draw_value_us
        };
        // 千日手PVへの固着を防ぐ±1の揺らぎ（ADR-0026、S:785のvalue_draw）
        base + VALUE_DRAW + 1 - (self.nodes & 2) as Value
    }

    /// 生の静的評価をeval hash経由で得る（ADR-0049）。ヒットなら
    /// evaluate()の全計算を省き、ミスなら計算して格納する。詰み圏の値は
    /// 入らない（evaluateの出力域のみ）。correction history補正は呼び側。
    #[inline]
    fn eval_cached(&mut self, key: u64) -> Value {
        if let Some(v) = self.shared.eval_hash.probe(key) {
            return v;
        }
        let v = self.evaluator.evaluate(&self.pos);
        self.shared.eval_hash.store(key, v);
        v
    }

    /// 指し手をStackへ記録する（ADR-0109。yaneuraou-search.cpp:2102-2120）。
    /// continuation historyの面は、このplyの王手の有無と手の種別で決まる。
    #[inline]
    fn set_current_move(&mut self, ply: usize, m: Move, capture: bool) {
        let in_check = self.stack[ply + STACK_OFFSET].in_check;
        let e = &mut self.stack[ply + STACK_OFFSET];
        e.current_move = m;
        e.cont_base = ContinuationHistory::base(in_check, capture, m.piece_after(), m.to());
        e.cont_corr_base = ContinuationCorrectionHistory::base(m);
    }

    /// 1手前から6手前までのcontinuation historyの面（ADR-0109）。
    /// 余白があるのでply 0でも境界検査は要らない。
    #[inline]
    fn cont_bases(&self, ply: usize) -> [usize; 6] {
        std::array::from_fn(|i| self.stack[ply + STACK_OFFSET - 1 - i].cont_base)
    }

    /// LMRのリダクション量（G2。yaneuraou-search.cpp:5148-5151）。
    /// 返る値は1024倍の固定小数である。
    ///
    /// 基礎値は深さと手数の表の積。そこから窓幅の比で引き、改善して
    /// いなければ基礎値の206/512を足し、定数1133を足す。窓が広い
    /// （root窓に近い）ほど削らない。
    #[inline]
    fn reduction(&self, improving: bool, depth: u32, move_count: u32, delta: Value) -> i32 {
        let t = self.reductions;
        let scale = i32::from(t[(depth as usize).min(REDUCTIONS_LEN - 1)])
            * i32::from(t[(move_count as usize).min(REDUCTIONS_LEN - 1)]);
        scale - delta * 585 / self.root_delta + i32::from(!improving) * scale * 206 / 512 + 1133
    }

    /// その手の履歴の強さ（G2。yaneuraou-search.cpp:3924-3932）。
    /// 取る手は取った駒の価値とcapture history、静かな手はmain historyの
    /// 2倍と1手前・2手前のcontinuation historyで測る。do_moveの前に呼ぶ
    #[inline]
    fn stat_score(&self, m: Move, cont: &[usize; 6]) -> i32 {
        let to = m.to();
        let pc = m.piece_after();
        let captured = if m.is_drop() {
            Piece::EMPTY
        } else {
            self.pos.piece_on(to)
        };
        if !captured.is_empty() {
            863 * himawari_core::piece_value(captured.piece_type()) / 128
                + self.hist.capture.get(pc, to, captured.piece_type())
        } else {
            2 * self.hist.main.get(self.pos.side_to_move(), m)
                + self.hist.cont.get(cont[0], pc, to)
                + self.hist.cont.get(cont[1], pc, to)
        }
    }

    /// 往復手（shuffling）か（G5。yaneuraou-search.cpp:813-838）。
    /// 2手前・4手前と行き先を交換しているだけの手は、singularの検証探索に
    /// かけない。参照実装の `#if STOCKFISH` の外側が将棋版で、50手ルールの
    /// 条件を落とし、代わりに駒打ちを除いている。取る手も往復手ではない
    fn is_shuffling(&self, m: Move, ply: usize) -> bool {
        // capture_stage(m) は将棋版では単なるcapture(m)（position.h:1345）
        if m.is_drop() || !self.pos.piece_on(m.to()).is_empty() {
            return false;
        }
        // null moveをまたぐと2手前・4手前の手が繋がらない。局面が動いて
        // いない手数が浅いうちも対象にしない
        if self.pos.state().plies_from_null <= 6 || ply < 18 {
            return false;
        }
        let move2 = self.stack[ply + STACK_OFFSET - 2].current_move;
        let move4 = self.stack[ply + STACK_OFFSET - 4].current_move;
        // is_ok()は「fromとtoが違う」ことなので、特殊手の除外に対応する
        if move2.is_special() || move4.is_special() || move2.is_drop() || move4.is_drop() {
            return false;
        }
        m.from_sq() == move2.to() && move2.from_sq() == move4.to()
    }

    /// correction historyの6要素を重み付きで合成する（ADR-0046, 0109）。
    /// 出典はやねうら王の `correction_value()`（yaneuraou-search.cpp:724-737）。
    /// 131072で割る前の値を返す。LMRのリダクションもこの値を読む。
    #[inline]
    fn correction_value(&self, ply: usize) -> i32 {
        let (pcv, micv, bnpcv, wnpcv) = self.shared.hists.corr.probe(&self.pos);
        // 余白があるのでply 0でも境界検査は要らない。前方は初期値のMove::NONE
        let prev1 = self.stack[ply + STACK_OFFSET - 1].current_move;
        let cntcv = if prev1.is_special() {
            CORR_CONT_DEFAULT
        } else {
            let to = prev1.to();
            let pc = self.pos.piece_on(to);
            self.hist
                .corr_cont
                .get(self.stack[ply + STACK_OFFSET - 2].cont_corr_base, pc, to)
                + self
                    .hist
                    .corr_cont
                    .get(self.stack[ply + STACK_OFFSET - 4].cont_corr_base, pc, to)
        };
        CORR_W_PAWN * pcv
            + CORR_W_MINOR * micv
            + CORR_W_NON_PAWN * (wnpcv + bnpcv)
            + CORR_W_CONT * cntcv
    }

    /// 生の静的評価にcorrection historyの補正を加える（ADR-0046, 0109）。
    /// 出典はやねうら王の `to_corrected_static_eval()`
    /// （yaneuraou-search.cpp:744-746）。詰み圏に入らないようクランプする。
    #[inline]
    fn to_corrected_with(&self, raw: Value, cv: i32) -> Value {
        (raw + cv / CORR_DIVISOR).clamp(VALUE_MATED_IN_MAX_PLY + 1, VALUE_MATE_IN_MAX_PLY - 1)
    }

    /// correction historyの補正込みの静的評価。
    #[inline]
    fn to_corrected(&self, raw: Value, ply: usize) -> Value {
        self.to_corrected_with(raw, self.correction_value(ply))
    }

    /// 探索を打ち切ってよい終端か（ADR-0026, 0030）。千日手・優等局面・
    /// 最大手数の到達・入玉宣言勝ちの4つを、この順に調べる。成立したら
    /// その評価値を返す。
    ///
    /// 通常探索と静止探索で判定も順序も同じなので1つにまとめる（ADR-0125）。
    /// 最大手数は静止探索でも通常探索と同じ扱いにする
    /// （yaneuraou-search.cpp:4620）。入玉宣言勝ちはADR-0109で唯一の例外と
    /// して位置を残した判定で、玉が敵陣外なら即falseで安い
    #[inline]
    fn terminal_value(&self, ply: usize) -> Option<Value> {
        match self.pos.repetition_state(ply) {
            Repetition::Draw => return Some(self.draw_value()),
            Repetition::Win => return Some(mate_in(ply)),
            Repetition::Lose => return Some(mated_in(ply)),
            Repetition::Superior => return Some(VALUE_SUPERIOR),
            Repetition::Inferior => return Some(-VALUE_SUPERIOR),
            Repetition::None => {}
        }
        if self.max_moves_to_draw > 0 && self.pos.game_ply() >= self.max_moves_to_draw {
            return Some(self.draw_value());
        }
        if self.pos.can_declare_win() {
            return Some(mate_in(ply));
        }
        None
    }

    /// 置換表を引き、Stackへヒットの有無とttPvを記録する（ADR-0022, 0024。
    /// yaneuraou-search.cpp:2623, 2657）。カットの判定は `tt_cutoff` が行う。
    fn probe_tt(&mut self, key: u64, ply: usize, is_pv: bool, excluded: Move) -> TtInfo {
        let hit = self.shared.tt.probe(key);
        // 1手先のノードがhistoryの更新条件で読む（yaneuraou-search.cpp:2623）
        self.stack[ply + STACK_OFFSET].tt_hit = hit.is_some();
        // 置換表にPVとして記録された値か（yaneuraou-search.cpp:2657）。
        // 除外手つき探索は同じplyでsearchを呼び直すので、上書きしない
        if excluded == Move::NONE {
            self.stack[ply + STACK_OFFSET].tt_pv = is_pv || hit.as_ref().is_some_and(|d| d.pv);
        }
        let mut tt = TtInfo {
            hit,
            mv: Move::NONE,
            value: VALUE_NONE,
            // 置換表のdepth欄はゲタを外して扱う。ヒットしないときは参照実装の
            // DEPTH_NONE（tt.cpp:445）に合わせて -3 とする
            depth: -TT_DEPTH_OFFSET,
            bound: Bound::None,
        };
        if let Some(data) = &hit {
            if let Some(m) = self.pos.to_move(data.mv)
                && self.pos.pseudo_legal(m)
            {
                tt.mv = m;
            }
            tt.value = value_from_tt(data.value, ply);
            tt.depth = i32::from(data.depth) - TT_DEPTH_OFFSET;
            tt.bound = data.bound;
        }
        tt
    }

    /// TTカットの判定（ADR-0024。yaneuraou-search.cpp:2700-2748）。
    /// カットするならその値を返す。除外手つき探索中はカットしない
    /// （probeは行い、eval再利用は可）。
    #[allow(clippy::too_many_arguments)]
    fn tt_cutoff(
        &mut self,
        tt: &TtInfo,
        ply: usize,
        depth: u32,
        alpha: Value,
        beta: Value,
        is_pv: bool,
        excluded: Move,
        prior_capture: Piece,
        prev_sq: Option<Square>,
    ) -> Option<Value> {
        if tt.hit.is_none() || !(excluded == Move::NONE && !is_pv && tt.depth >= depth as i32) {
            return None;
        }
        let usable = match tt.bound {
            Bound::Exact => true,
            Bound::Lower => tt.value >= beta,
            Bound::Upper => tt.value <= alpha,
            Bound::None => false,
        };
        if !usable {
            return None;
        }
        // TTカットでもオーダリングを更新する
        // （yaneuraou-search.cpp:2735-2748）
        if tt.mv != Move::NONE && tt.value >= beta {
            let tt_capture = !tt.mv.is_drop() && !self.pos.piece_on(tt.mv.to()).is_empty();
            if !tt_capture {
                self.update_quiet_histories(ply, tt.mv, (130 * depth as i32 - 71).min(1043));
            }
            // 1手前の早い静かな手への追加ペナルティ
            if let Some(prev_sq) = prev_sq
                && self.stack[ply + STACK_OFFSET - 1].move_count <= 4
                && prior_capture.is_empty()
            {
                let pc = self.pos.piece_on(prev_sq);
                self.update_continuation_histories(ply - 1, pc, prev_sq, -2142);
            }
        }
        Some(tt.value)
    }

    /// MultiPV>1のときだけライン番号を出す（現行互換）。
    #[inline]
    fn multipv_label(&self, pv_idx: usize) -> usize {
        if self.multi_pv > 1 { pv_idx + 1 } else { 0 }
    }

    /// 反復深化1周分の報告を組み立てる（ADR-0125）。3か所で
    /// seldepth・nodes・elapsed_ms・hashfullの採り方が同じなのでまとめる。
    /// nodesは全ワーカー合算（単スレッドではローカル値と一致）。
    fn iter_info(&self, depth: u32, multipv: usize, score: Value, pv: Vec<Move>) -> IterInfo {
        IterInfo {
            depth,
            seldepth: self.sel_depth.max(depth),
            multipv,
            score,
            pv,
            nodes: self.shared.nodes.load(Ordering::Relaxed).max(self.nodes),
            elapsed_ms: self.tm.elapsed().as_millis() as u64,
            hashfull: self.shared.tt.hashfull(),
        }
    }

    /// aspirationのfail時に途中経過を報告する（ADR-0091）。
    /// PVは並べ替え済みの `root_moves[pv_idx]` から採る。
    fn report_bound(
        &self,
        on_info: &mut dyn FnMut(SearchInfo),
        depth: u32,
        pv_idx: usize,
        score: Value,
        bound: ScoreBound,
    ) {
        let line: Vec<Move> = self.root_moves[pv_idx].pv.clone();
        if line.is_empty() {
            return;
        }
        on_info(SearchInfo::Bound(
            self.iter_info(depth, self.multipv_label(pv_idx), score, line),
            bound,
        ));
    }

    /// root手をスコアの降順に安定ソートする（search.h:168-171）。
    /// 同点なら前の反復のスコアの降順で、それも同点なら元の並びを保つ。
    fn sort_root_moves(moves: &mut [RootMove]) {
        moves.sort_by(|a, b| b.score.cmp(&a.score).then(b.prev_score.cmp(&a.prev_score)));
    }

    /// 反復深化。各イテレーション完了時にon_iterを呼ぶ。
    /// 局面を差し替え、plyごとの状態を初期化する（ADR-0136）。
    ///
    /// 前処理ツールがWorkerを使い回すために要る。局面ごとに `new` を呼ぶと
    /// history一式（数MB）を毎回確保することになり、実測で679局面/秒まで
    /// 落ちた。**スタックを戻すのは結果を処理順に依存させないためである。**
    /// historyは持ち越すが、これは同じ対局を続けて読むときと同じ状況で、
    /// 手順の再現性を壊さない。
    pub fn set_position(&mut self, pos: Position) {
        self.root_color = pos.side_to_move();
        self.pos = pos;
        // accumulatorのスタックを捨てる。前の局面の計算済みフラグが
        // 残っていると、全く別の局面の評価値をそのまま読んでしまう
        self.evaluator.new_search(&self.pos);
        self.stack.fill(STACK_INIT);
        self.sel_depth = 0;
        self.root_delta = 1;
        self.root_depth = 0;
        self.nmp_min_ply = 0;
        self.last_iteration_pv.clear();
    }

    /// 静止探索を1回走らせ、到達した静止局面まで `self.pos` を進める
    /// （ADR-0136）。戻り値は進めた手数である。
    ///
    /// 教師局面をqsearchのPV葉へ置き換える前処理が使う。**探索の内側は
    /// 変えない。** qsearchは最善手を置換表へ書くので、置換表を辿れば
    /// PVを再構成できる。qsearchにPV収集を足すと、全ての葉で費用を払う
    /// ことになるため採らない。
    ///
    /// 打ち切りは `max_plies` で行う。置換表の衝突で手が繋がり続ける場合と、
    /// 千日手のような循環に備える。
    pub fn walk_to_quiet(&mut self, max_plies: usize) -> usize {
        let mut plies = 0;
        while plies < max_plies {
            self.stack[STACK_OFFSET].in_check = self.pos.in_check();
            self.qsearch::<true>(-VALUE_INFINITE, VALUE_INFINITE, 0);
            let key = self.pos.key();
            let Some(data) = self.shared.tt.probe(key) else {
                break;
            };
            let Some(m) = self.pos.to_move(data.mv) else {
                break;
            };
            if !self.pos.pseudo_legal(m) || !self.pos.is_legal(m) {
                break;
            }
            self.pos.do_move(m);
            // do_moveとpushは対で行う（探索本体と同じ作法）。
            // 欠かすとaccumulatorが1手前のまま計算済みとして読まれる
            self.evaluator.push(&self.pos);
            plies += 1;
        }
        plies
    }

    pub fn iterate(&mut self, on_info: &mut dyn FnMut(SearchInfo)) -> SearchResult {
        // 入玉宣言勝ち（ADR-0030）: 成立していれば探索せず宣言する
        if self.pos.can_declare_win() {
            return SearchResult {
                best: Move::WIN,
                score: mate_in(0),
                ponder: Move::NONE,
                root_score: -VALUE_INFINITE,
                root_average_score: -VALUE_INFINITE,
                pv: Vec::new(),
                completed_depth: 0,
            };
        }
        self.shared.tt.new_search();
        self.evaluator.new_search(&self.pos);
        // lowPly historyはgoのたびに埋め直す（ADR-0109。S:1539-1540）
        self.hist.new_search();
        // 前回のgoで得たPVは今回の探索では使えないので捨てる
        // （yaneuraou-search.cpp:943）
        self.last_iteration_pv.clear();
        let mut list = MoveList::default();
        generate_legal(&self.pos, false, &mut list);
        self.root_moves = list
            .as_slice()
            .iter()
            .map(|&mv| RootMove {
                mv,
                score: -VALUE_INFINITE,
                prev_score: VALUE_ZERO,
                pv: Vec::new(),
                effort: 0,
                average_score: -VALUE_INFINITE,
                mean_squared_score: MEAN_SQUARED_NONE,
            })
            .collect();
        if self.root_moves.is_empty() {
            return SearchResult {
                best: Move::RESIGN,
                score: mated_in(0),
                ponder: Move::NONE,
                root_score: -VALUE_INFINITE,
                root_average_score: -VALUE_INFINITE,
                pv: Vec::new(),
                completed_depth: 0,
            };
        }
        // 前回のgoと手番が入れ替わっているなら、持ち越したスコアの符号を
        // 反転させる（G9。S:1483-1492）。番兵はそのまま残す
        let root_game_ply = self.pos.game_ply();
        if (i32::from(self.memory.last_game_ply) - i32::from(root_game_ply)) & 1 != 0 {
            if self.memory.best_previous_score != VALUE_INFINITE {
                self.memory.best_previous_score = -self.memory.best_previous_score;
            }
            if self.memory.best_previous_average_score != VALUE_INFINITE {
                self.memory.best_previous_average_score = -self.memory.best_previous_average_score;
            }
        }
        let mut last_score = VALUE_ZERO;
        // 最後に確定した最善手のPVとスコア（S:1426-1428）。中断した反復の
        // 詰み負けスコアはここへ戻すために覚える
        let mut last_best_pv: Vec<Move> = Vec::new();
        let mut last_best_score = -VALUE_INFINITE;
        // 最後に出したinfoが未確定の窓外れ（lowerbound / upperbound）か。
        // 打ち切りでこのまま終わると、GUIやCSAブリッジは確定していない値を
        // その手のスコアとして記録する。実際にfloodgateで `4723++` という
        // 値が残り、直後に評価が8300も反転した
        let mut unresolved_bound = false;
        // 確定した最後のイテレーションの深さ。
        let mut completed_depth = 0u32;
        // メインワーカーか（S:1917）。時間管理と統計はメインだけが行う
        let is_main = self.thread_idx == 0;
        // 直近のroot探索の返り値（S:1438）。窓を外した回も含む。
        // 思考時間のfallingEvalが読む
        let mut best_value = -VALUE_INFINITE;
        // 最善手が最後に変わった深さ（S:1426）。timeReductionの中心になる
        let mut last_best_move_depth = 0u32;
        // 直近4反復のスコアの環状バッファ（S:1440, 2055-2056）。
        // fallingEvalが4回前の反復のスコアと比べる
        let mut iter_value = [VALUE_ZERO; 4];
        let mut iter_idx = 0usize;
        // 最善手が入れ替わった回数の統計（S:1439）。反復ごとに半減させ、
        // 全スレッドの計数を足し込む
        let mut tot_best_move_changes = 0.0f64;
        // 今回goのtimeReduction（S:1439, 2062）。次のgoへ持ち越す
        let mut time_reduction = 1.0f64;
        // 同じ深さを掘り直した回数（S:1531-1534）。実効深さを削る量に効く
        let mut search_again_counter = 0u32;
        // 前回goのスコアで環状バッファを埋める（S:1495-1498）。
        // 前回がなければ0で埋める
        iter_value.fill(if self.memory.best_previous_score == VALUE_INFINITE {
            VALUE_ZERO
        } else {
            self.memory.best_previous_score
        });
        let max_depth = if self.limits.depth > 0 {
            self.limits.depth
        } else {
            (MAX_PLY - 1) as u32
        };

        'deepening: for depth in 1..=max_depth {
            // 反復の世代が進んだので、最善手の入れ替わりの重みを半分にする
            // （S:1577-1581）。メインだけが集計する
            if is_main {
                tot_best_move_changes /= 2.0;
            }
            for rm in &mut self.root_moves {
                rm.prev_score = rm.score;
            }
            // 深さが増えていないなら同じ深さを掘り直したことになる
            // （S:1600-1606）。メインが余り時間から立てた旗を全スレッドが読む
            if !self.shared.increase_depth.load(Ordering::Relaxed) {
                search_again_counter += 1;
            }
            // seldepthはイテレーションごとに測り直す（ADR-0086）
            self.sel_depth = 0;
            // singularの多段化のマージンが読む（S:1550, 3779）。実効深さ
            // （fail highで削った値）ではなく反復深化の深さを入れる
            self.root_depth = depth;
            let lines = self.multi_pv.min(self.root_moves.len());
            // 直前ラインの出力スコア。頭打ちの基準に使う（ADR-0032）
            let mut prev_line_score = VALUE_INFINITE;
            for pv_idx in 0..lines {
                // ラインごとのaspiration（G9。S:1669-1673）。窓幅は評価値の
                // 二乗平均に比例して広がり、中心はスコアの移動平均に置く。
                // 深さ1では二乗平均が番兵のままなので窓が全開になる
                let mut delta = ASPIRATION_BASE
                    + (self.thread_idx % ASPIRATION_THREAD_SPREAD) as Value
                    + self.root_moves[pv_idx].mean_squared_score.abs() / ASPIRATION_MSS_DIV;
                let avg = self.root_moves[pv_idx].average_score;
                let mut alpha = (avg - delta).max(-VALUE_INFINITE);
                let mut beta = (avg + delta).min(VALUE_INFINITE);
                // fail highした回数。1回ごとに実効深さを1段削る（S:1705-1706）
                let mut failed_high_cnt = 0u32;
                loop {
                    // fail highと掘り直しの分だけ実効深さを削る（S:1699-1707）。
                    // searchAgain 4回につき1回は深さが進むようにしてある
                    let adjusted_depth = depth
                        .saturating_sub(failed_high_cnt)
                        .saturating_sub(3 * (search_again_counter + 1) / 4)
                        .max(1);
                    let score = self.search_root(adjusted_depth, alpha, beta, pv_idx, on_info);
                    // 窓を外した回も含めて毎回並べ替える（S:1717）。
                    // 実の値を持つのは1手目とalphaを更新した手だけなので、
                    // 安定ソートでなければ残りの並びが崩れる
                    Self::sort_root_moves(&mut self.root_moves[pv_idx..]);
                    best_value = score;
                    if self.stopped() {
                        break 'deepening;
                    }
                    if score <= alpha {
                        // fail low: 実際の評価はこの値以下（ADR-0091）。
                        // 窓を広げて読み直す前に、途中経過として報告する
                        self.report_bound(on_info, depth, pv_idx, score, ScoreBound::Upper);
                        if pv_idx == 0 {
                            unresolved_bound = true;
                        }
                        // 窓は下へずらす。上端は元のalphaに畳む（S:1777-1784）
                        beta = alpha;
                        alpha = (score - delta).max(-VALUE_INFINITE);
                        failed_high_cnt = 0;
                        // 評価が下がったので、予約した停止を解除する
                        // （S:1783-1784）。読み直す価値のある局面である
                        self.stop_on_ponderhit = false;
                    } else if score >= beta {
                        // fail high: 実際の評価はこの値以上
                        self.report_bound(on_info, depth, pv_idx, score, ScoreBound::Lower);
                        if pv_idx == 0 {
                            unresolved_bound = true;
                        }
                        // 下端は元のbetaから幅の分だけ戻した位置まで上げる
                        // （S:1786-1790）
                        alpha = (beta - delta).max(alpha);
                        beta = (score + delta).min(VALUE_INFINITE);
                        failed_high_cnt += 1;
                    } else {
                        // 成功。最善手は並べ替えでこのラインの先頭に来ている。
                        // 出力スコアを前のラインで頭打ちにする。fail-softでは
                        // ラインごとにaspiration窓が違い、返り値が窓に依存する
                        // ため、探索順のままだと後のラインが前を上回ることが
                        // ある。root_movesへは生の値を残し、出力だけ整える。
                        // 並べ替えで整えると、確定済みラインと手が重複する。
                        // 基準は前ラインの生スコアではなく出力スコアにする。
                        // 生スコアだと、前ラインが頭打ちされた分を後ろのラインが
                        // 飛び越えられる（s1=100・s2=150・s3=120で3本目が
                        // 120となり2本目の100を上回る）
                        let line_score = score.min(prev_line_score);
                        prev_line_score = line_score;
                        let line_pv = self.root_moves[pv_idx].pv.clone();
                        if pv_idx == 0 {
                            last_score = score;
                            unresolved_bound = false;
                        }
                        on_info(SearchInfo::Iteration(self.iter_info(
                            depth,
                            self.multipv_label(pv_idx),
                            line_score,
                            line_pv,
                        )));
                        break;
                    }
                    // 外したので次は幅を4/3倍にする（S:1795）
                    delta += delta / ASPIRATION_GROWTH_DIV;
                }
            }
            // 今回の反復のPVを覚える（yaneuraou-search.cpp:1846-1853）。
            // 次の反復のfollow_pv判定が読む。打ち切られた反復のPVは
            // 途中までしか探索していないので採らない
            if !self.shared.stop.load(Ordering::Relaxed) {
                completed_depth = depth;
                // 最善手が前の反復から変わった深さを覚える（S:1848-1851）。
                // timeReductionのロジスティックの中心になる
                if self.last_iteration_pv.is_empty()
                    || self.last_iteration_pv.first() != self.root_moves[0].pv.first()
                {
                    last_best_move_depth = depth;
                }
                self.last_iteration_pv.clone_from(&self.root_moves[0].pv);
            }
            // 最善手が変わったら、確定したPVとスコアを覚え直す（S:1888-1893）
            if self.root_moves[0].pv.first() != last_best_pv.first() {
                last_best_pv.clone_from(&self.root_moves[0].pv);
                last_best_score = self.root_moves[0].score;
                last_best_move_depth = depth;
            }
            // ここまで来れば深さ1は完走している。以降はstopに従う
            self.depth1_done = true;
            // 詰みが確定したら打ち切る（ADR-0088）。反復深化なので、より短い
            // 詰みがあれば浅い周で見つかっている。これ以上読んでも結論は
            // 変わらない。詰まされる側も同じで、より長く粘る手があれば
            // alpha-betaが既にそちらを選んでいる
            if self.multi_pv == 1 && last_score.abs() >= VALUE_MATE_IN_MAX_PLY {
                break;
            }
            if self.stopped() {
                break;
            }
            if self.limits.nodes > 0 && self.nodes >= self.limits.nodes {
                break;
            }
            // ここから先はメインだけが行う（S:1917）
            if !is_main {
                continue;
            }
            // 全スレッドの最善手の入れ替わり回数を汲み出す（S:1952-1956）
            tot_best_move_changes +=
                self.shared.best_move_changes.swap(0, Ordering::Relaxed) as f64;
            // 次の反復を回す時間があるか（S:1961-2053）。停止条件を満たして
            // もその場では止めず、秒単位で切り上げた終了時刻を予約する。
            // 予約済みなら終了は確定しているので測り直さない
            if self.tm.use_time_management()
                && !self.shared.stop.load(Ordering::Relaxed)
                && !self.stop_on_ponderhit
                && self.tm.search_end == 0
            {
                let stats = IterationStats {
                    // 最善手にノードがどれだけ集中しているか（10万分率）。
                    // 分子も分母もgo全体の累計である（S:1969-1970）
                    nodes_effort: self.root_moves[0].effort * 100_000 / self.nodes.max(1),
                    best_value,
                    prev_go_average: self.memory.best_previous_average_score,
                    iter_value: iter_value[iter_idx],
                    last_best_move_depth,
                    completed_depth,
                    prev_time_reduction: self.memory.previous_time_reduction,
                    tot_best_move_changes,
                    thread_count: self.thread_count,
                    single_move: self.root_moves.len() == 1,
                };
                let plan = self.tm.plan_next_iteration(&stats);
                time_reduction = plan.time_reduction;
                if plan.budget_spent {
                    if self.shared.ponder.load(Ordering::SeqCst) {
                        // go ponder中はGUIの指示があるまで止められない。
                        // 停止を予約だけしておく（S:2043-2044）
                        self.stop_on_ponderhit = true;
                    } else {
                        let ponderhit_offset = self.shared.ponderhit_offset.load(Ordering::SeqCst);
                        self.tm.set_search_end(plan.elapsed, ponderhit_offset);
                    }
                } else {
                    // 深さを1段上げる余裕がないなら、次の反復では実効深さを
                    // 削って掘り直す（S:2049-2051）。ponder中は掘り直さない
                    self.shared.increase_depth.store(
                        self.shared.ponder.load(Ordering::SeqCst) || plan.has_depth_slack,
                        Ordering::Relaxed,
                    );
                }
            }
            // 4反復前のスコアと比べるための環状バッファ（S:2055-2056）
            iter_value[iter_idx] = best_value;
            iter_idx = (iter_idx + 1) & 3;
        }
        // 今回goのtimeReductionを次のgoへ持ち越す（S:2062）
        self.memory.previous_time_reduction = time_reduction;
        // 中断した探索で得た詰み負けのスコアは信用できない（S:1864-1887）。
        // 残りのroot手を読めば、負けが延びたり反証されたりしうる。前の
        // 反復で確定したPVとスコアへ戻す。
        //
        // 参照実装はこの判定を反復の末尾で毎回行う。本エンジンは中断時に
        // 反復の途中で抜けるので、ループを出たあとに1回だけ行う。
        // abortedSearchが立つとstopも立ち、その反復で必ずループを出るので
        // 判定の回数は変わらない
        if self.shared.aborted_search.load(Ordering::Relaxed)
            && self.root_moves[0].score != -VALUE_INFINITE
            && self.root_moves[0].score <= VALUE_MATED_IN_MAX_PLY
            && !last_best_pv.is_empty()
        {
            // 確定した最善手を先頭へ移す
            if let Some(i) = self
                .root_moves
                .iter()
                .position(|rm| rm.mv == last_best_pv[0])
            {
                let rm = self.root_moves.remove(i);
                self.root_moves.insert(0, rm);
            }
            self.root_moves[0].pv.clone_from(&last_best_pv);
            self.root_moves[0].score = last_best_score;
            last_score = last_best_score;
        }
        // 未確定の窓外れで終わるなら、確定した最後の結果を出し直す。
        // これを出さないと、消費側の最後の1行が lowerbound / upperbound の
        // ままになり、指し手と食い違うスコアがその手の評価として残る
        if unresolved_bound && completed_depth > 0 && !self.root_moves[0].pv.is_empty() {
            on_info(SearchInfo::Iteration(self.iter_info(
                completed_depth,
                self.multipv_label(0),
                last_score,
                self.root_moves[0].pv.clone(),
            )));
        }
        // 次のgoで使う値を持ち越す（G9。S:1249-1253）。参照実装はbest thread
        // のrootMoves[0]から採る。投票で他スレッドが選ばれたときは、
        // 呼び出し側（ThreadPool）がそちらの値で上書きする（G10）
        self.memory.best_previous_score = self.root_moves[0].score;
        self.memory.best_previous_average_score = self.root_moves[0].average_score;
        self.memory.last_game_ply = root_game_ply;
        // check_limitsで2048刻みに流し込んだ分を除いた端数を合算する
        self.shared
            .nodes
            .fetch_add(self.nodes % 2048, Ordering::Relaxed);
        SearchResult {
            best: self.root_moves[0].mv,
            score: last_score,
            ponder: self.root_moves[0].pv.get(1).copied().unwrap_or(Move::NONE),
            root_score: self.root_moves[0].score,
            root_average_score: self.root_moves[0].average_score,
            pv: self.root_moves[0].pv.clone(),
            completed_depth,
        }
    }

    /// root_moves[pv_idx..]を探索する（上位の確定済みラインは除外）。
    /// 戻り値はfail-softのスコア。各root手のスコアとPVは `root_moves` へ
    /// 直接書く（S:4113-4165）。呼び出し側は探索のあとで並べ替えて先頭を読む
    fn search_root(
        &mut self,
        depth: u32,
        mut alpha: Value,
        beta: Value,
        pv_idx: usize,
        on_info: &mut dyn FnMut(SearchInfo),
    ) -> Value {
        // リダクションの窓幅項の基準（yaneuraou-search.cpp:1708）
        self.root_delta = beta - alpha;
        let mut best = -VALUE_INFINITE;
        let moves: Vec<Move> = self.root_moves[pv_idx..].iter().map(|rm| rm.mv).collect();
        // 子が読むcontinuation historyの面の選択に要る（ADR-0109のG1）
        let in_check = self.pos.in_check();
        self.stack[STACK_OFFSET].in_check = in_check;
        // 1手目のノードがhistoryの更新条件でrootのStackを読む
        // （yaneuraou-search.cpp:2623, 3126-3133）。参照実装はrootも同じ
        // search()なので、置換表ヒットと静的評価がrootでも埋まっている
        let key = self.pos.key();
        self.stack[STACK_OFFSET].tt_hit = self.shared.tt.probe(key).is_some();
        // rootは常にPVノードなのでttPvはtrue（yaneuraou-search.cpp:2657）
        self.stack[STACK_OFFSET].tt_pv = true;
        self.stack[STACK_OFFSET + 2].cutoff_cnt = 0;
        self.stack[STACK_OFFSET].stat_score = 0;
        // rootは常に前回PVの上にいる（yaneuraou-search.cpp:2370）
        self.stack[STACK_OFFSET].follow_pv = true;
        // continuation historyの面（1手前から6手前まで）。rootでも
        // statScoreの計算に要る
        let cont = self.cont_bases(0);
        self.stack[STACK_OFFSET].static_eval = if in_check {
            VALUE_NONE
        } else {
            let raw = self.eval_cached(key);
            self.to_corrected(raw, 0)
        };
        for (j, &m) in moves.iter().enumerate() {
            let i = pv_idx + j;
            self.stack[STACK_OFFSET].move_count = (j + 1) as u32;
            // 長考中だけ「今読んでいる手」を出す（ADR-0086）。短い探索で
            // 出すとinfo行が溢れる
            if self.tm.elapsed().as_millis() as u64 >= CURRMOVE_MIN_MS {
                on_info(SearchInfo::CurrMove { depth, mv: m });
            }
            let capture = !m.is_drop() && !self.pos.piece_on(m.to()).is_empty();
            // 1手目のノードがhistoryの更新量で読む
            // （yaneuraou-search.cpp:3924-3932）
            self.stack[STACK_OFFSET].stat_score = self.stat_score(m, &cont);
            self.set_current_move(0, m, capture);
            let nodes_before = self.nodes;
            self.pos.do_move(m);
            self.evaluator.push(&self.pos);
            let mut child_pv = Vec::new();
            // rootはcut_node = false。PVの第1手と再探索はfalse、
            // 第2手以降のnull windowは反転してtrueになる（ADR-0109のG0）
            let value = if j == 0 {
                -self.search::<true>(-beta, -alpha, depth - 1, 1, m, &mut child_pv, false)
            } else {
                let v =
                    -self.search::<false>(-alpha - 1, -alpha, depth - 1, 1, m, &mut child_pv, true);
                if v > alpha && !self.stopped() {
                    -self.search::<true>(-beta, -alpha, depth - 1, 1, m, &mut child_pv, false)
                } else {
                    v
                }
            };
            self.evaluator.pop();
            self.pos.undo_move(m);
            // 打ち切られた分もこの手の探索コストなので先に計上（ADR-0062）
            self.root_moves[i].effort += self.nodes - nodes_before;
            if self.stopped() {
                return best;
            }
            // スコアの移動平均と二乗平均（G9。yaneuraou-search.cpp:4105-4110）。
            // 次の反復のaspiration窓の中心と幅になる。番兵のときは初回として
            // 値をそのまま入れる。二乗平均は符号を保つため |value| を掛ける
            {
                let rm = &mut self.root_moves[i];
                rm.average_score = if rm.average_score != -VALUE_INFINITE {
                    (value + rm.average_score) / 2
                } else {
                    value
                };
                rm.mean_squared_score = if rm.mean_squared_score != MEAN_SQUARED_NONE {
                    (value * value.abs() + rm.mean_squared_score) / 2
                } else {
                    value * value.abs()
                };
            }
            // root手のスコアとPVを書く（S:4113-4165）。実の値を持つのは
            // 1手目とalphaを更新した手だけで、残りは -VALUE_INFINITE にする。
            // この形なら途中で打ち切っても並べ替えが前の深さの結論を壊さない
            if j == 0 || value > alpha {
                self.root_moves[i].score = value;
                let line = &mut self.root_moves[i].pv;
                line.clear();
                line.push(m);
                line.extend_from_slice(&child_pv);
                // 最善手が入れ替わった回数を数える（G9。
                // yaneuraou-search.cpp:4157-4160）。1手目での確定は
                // 入れ替わりではない。MultiPVでは1本目だけを数える
                if j > 0 && pv_idx == 0 {
                    self.shared
                        .best_move_changes
                        .fetch_add(1, Ordering::Relaxed);
                }
            } else {
                self.root_moves[i].score = -VALUE_INFINITE;
            }
            if value > best {
                best = value;
                if value > alpha {
                    if value >= beta {
                        // 参照実装はrootも同じ経路を通る
                        // （yaneuraou-search.cpp:4214）
                        self.stack[STACK_OFFSET].cutoff_cnt += 1;
                        break;
                    }
                    alpha = value;
                }
            }
        }
        best
    }

    /// 通常探索。参照実装が `search<PV>` と `search<NonPV>` をテンプレートで
    /// 分けるのに合わせ、ノード種別をconst genericの `PV` で受ける
    /// （ADR-0151の群J）。呼び出し側は定数で呼び分ける。
    #[allow(clippy::too_many_arguments)]
    fn search<const PV: bool>(
        &mut self,
        mut alpha: Value,
        beta: Value,
        mut depth: u32,
        ply: usize,
        prev: Move,
        pv: &mut Vec<Move>,
        cut_node: bool,
    ) -> Value {
        // 参照実装の不変条件（ADR-0109のG0）。PVノードはcut_nodeにならない
        debug_assert!(!(PV && cut_node));
        pv.clear();
        if self.stopped() {
            return VALUE_ZERO;
        }
        self.sel_depth = self.sel_depth.max(ply as u32);
        self.nodes += 1;
        self.check_limits();
        if ply >= MAX_PLY {
            return self.evaluator.evaluate(&self.pos);
        }

        if let Some(v) = self.terminal_value(ply) {
            return v;
        }

        // mate distance pruning
        alpha = alpha.max(mated_in(ply));
        let beta = beta.min(mate_in(ply + 1));
        if alpha >= beta {
            return alpha;
        }

        // ノードの初期化（yaneuraou-search.cpp:2353-2357）。前の兄弟ノードの
        // 値が残らないよう、手数をここで0へ戻す。王手の有無もここで入れる。
        // TTカット時のhistory更新がcontinuation historyの段数の打ち切りに使う
        let in_check = self.pos.in_check();
        self.stack[ply + STACK_OFFSET].in_check = in_check;
        self.stack[ply + STACK_OFFSET].move_count = 0;
        // 2手先のβカット回数を戻す（yaneuraou-search.cpp:2555）。
        // 自分の次plyは1手前のノードが戻しているので、兄弟をまたいで貯まる
        self.stack[ply + STACK_OFFSET + 2].cutoff_cnt = 0;
        self.stack[ply + STACK_OFFSET].stat_score = 0;
        // 1手前のLMRが削った量（yaneuraou-search.cpp:2552-2553）。
        // 読んだ側が消す。深さの事後補正がこの値を見る
        let prior_reduction = self.stack[ply + STACK_OFFSET - 1].reduction;
        self.stack[ply + STACK_OFFSET - 1].reduction = 0;
        // 前回の反復深化のPVを辿っているか（yaneuraou-search.cpp:2370-2372）。
        // 1手前がPV上にいて、1手前の手が前回PVの同じplyの手と一致するときに
        // 限って真になる。search()はply >= 1でしか呼ばれない
        let follow_pv = self.stack[ply + STACK_OFFSET - 1].follow_pv
            && ply - 1 < self.last_iteration_pv.len()
            && self.stack[ply + STACK_OFFSET - 1].current_move == self.last_iteration_pv[ply - 1];
        self.stack[ply + STACK_OFFSET].follow_pv = follow_pv;
        // 1手前が取った駒と、1手前の移動先（yaneuraou-search.cpp:2355, 2550）。
        // historyの更新条件が繰り返し読む
        let prior_capture = self.pos.state().captured;
        let prev_move = self.stack[ply + STACK_OFFSET - 1].current_move;
        let prev_sq = if prev_move.is_special() {
            None
        } else {
            Some(prev_move.to())
        };

        // 除外手（singular extension用。ADR-0050）。検証探索中はTT手が入る
        let excluded = self.stack[ply + STACK_OFFSET].excluded_move;

        // 置換表（ADR-0022, 0024）
        let key = self.pos.key();
        let tt = self.probe_tt(key, ply, PV, excluded);
        if let Some(v) = self.tt_cutoff(
            &tt,
            ply,
            depth,
            alpha,
            beta,
            PV,
            excluded,
            prior_capture,
            prev_sq,
        ) {
            return v;
        }

        // 置換表の手が駒を取る手か（yaneuraou-search.cpp:2671）。
        // RFPの条件とリダクションの1項が読む
        let tt_capture =
            tt.mv != Move::NONE && !tt.mv.is_drop() && !self.pos.piece_on(tt.mv.to()).is_empty();
        // PVでもcutでもないノード（yaneuraou-search.cpp:2251）。
        // 全手を調べる見込みなのでリダクションを強める。IIRの条件も読む
        let all_node = !(PV || cut_node);

        if depth == 0 {
            // 参照実装はノード種別を引き継ぐ（yaneuraou-search.cpp:2256）
            return self.qsearch::<PV>(alpha, beta, ply);
        }

        // 静的評価（ADR-0028, 0109のG4）。TTのevalを再利用する。
        // rawは補正前（TT保存用）、static_evalはcorrection history補正後（ADR-0046）。
        let raw_eval = if in_check {
            VALUE_NONE
        } else {
            match &tt.hit {
                Some(d) if Value::from(d.eval) != VALUE_NONE => Value::from(d.eval),
                // TTにevalがなければeval hash経由で計算する（ADR-0049）
                _ => self.eval_cached(key),
            }
        };
        // correction historyの合成値（yaneuraou-search.cpp:3010）。
        // 静的評価の補正と、LMRのリダクションの減算が同じ値を読む
        let corr_value = self.correction_value(ply);
        // 王手中は評価関数を呼ばず、2手前の静的評価をそのまま写す
        // （yaneuraou-search.cpp:3017）。将棋は王手が続くので、ここを
        // VALUE_NONEにするとimprovingの連鎖が切れて枝刈りが一律に甘くなる
        let static_eval = if in_check {
            self.stack[ply + STACK_OFFSET - 2].static_eval
        } else {
            self.to_corrected_with(raw_eval, corr_value)
        };

        self.stack[ply + STACK_OFFSET].static_eval = static_eval;
        // 置換表の値がこの局面の見積りとしてより適切なら、枝刈り用の評価値へ
        // 採用する（yaneuraou-search.cpp:3084-3087）。下界なら真の値はこれ
        // 以上、上界ならこれ以下と分かっているため。razoringとRFPがこの値を
        // 読み、NMP以降とムーブループはstatic_evalを読む。除外手つき探索と
        // TT不ヒットのときは採用しない（参照実装のStep 6の分岐に対応する）
        let mut eval = static_eval;
        if !in_check && excluded == Move::NONE && tt.hit.is_some() && tt.value != VALUE_NONE {
            let usable = if tt.value > eval {
                matches!(tt.bound, Bound::Lower | Bound::Exact)
            } else {
                matches!(tt.bound, Bound::Upper | Bound::Exact)
            };
            if usable {
                eval = tt.value;
            }
        }
        // 静的評価の差で静かな手のオーダリングを補正する
        // （yaneuraou-search.cpp:3126-3133）。1手前で評価が下がっていれば
        // 1手前の手を良い手とみなして加点する。参照実装は王手中ここへ来ない
        let prev1 = self.stack[ply + STACK_OFFSET - 1];
        if !in_check
            && let Some(prev_sq) = prev_sq
            && !prev1.in_check
            && prior_capture.is_empty()
        {
            let eval_diff = (-(prev1.static_eval + static_eval)).clamp(-214, 171) + 60;
            let them = self.pos.side_to_move().flip();
            self.hist
                .main
                .update(them, prev1.current_move, eval_diff * 10);
            let pc = self.pos.piece_on(prev_sq);
            if tt.hit.is_none()
                && pc.piece_type() != PieceType::PAWN
                && !prev1.current_move.is_promote()
            {
                let slot = self.shared.hists.pawn.slot(self.pos.pawn_key());
                self.shared
                    .hists
                    .pawn
                    .update(slot, pc, prev_sq, eval_diff * 12);
            }
        }

        // ここまでで決まった前提を束ねる（ADR-0125）。以降の枝刈り・延長・
        // リダクション・終端処理が共通して読む
        let node = NodeInfo {
            ply,
            cut_node,
            all_node,
            in_check,
            follow_pv,
            excluded,
            key,
            raw_eval,
            static_eval,
            eval,
            corr_value,
            prior_reduction,
            prior_capture,
            prev_sq,
            tt,
            tt_capture,
        };

        // 2手前より静的評価が改善しているか（枝刈りの強弱に使う。
        // yaneuraou-search.cpp:3159）。王手中はfalse固定である。
        // 王手中のstatic_evalが2手前の写しなので、連続王手でも連鎖は切れない。
        // 余白の初期値VALUE_NONE（32602）を上回るstatic_evalは存在しないため、
        // ply < 2でも比較だけでfalseになる（参照実装も同じ性質に依存する）
        let mut improving = false;
        if let Some(v) =
            self.prune_before_moves::<PV>(&node, alpha, beta, &mut depth, &mut improving, prev)
        {
            return v;
        }

        // 置換表の下界による簡易ProbCut（ADR-0078）。探索を伴わない。
        // 除外手つき探索中はスキップする（ADR-0050）
        let tt_probcut_beta = beta + TT_PROBCUT_MARGIN;
        if excluded == Move::NONE
            && matches!(tt.bound, Bound::Lower | Bound::Exact)
            && tt.depth >= depth.saturating_sub(TT_PROBCUT_DEPTH_SLACK) as i32
            && tt.value >= tt_probcut_beta
            && beta.abs() < VALUE_MATE_IN_MAX_PLY
            && tt.value.abs() < VALUE_MATE_IN_MAX_PLY
        {
            return tt_probcut_beta;
        }

        // 延長を積む前の深さ。MovePickerのオーダリングの尺度
        // （yaneuraou-search.cpp:3453）と、TT手のnewDepthの基準
        // （yaneuraou-search.cpp:3556）がこの値を読む。参照実装はどちらも
        // depthを増やす前に決まるので、singularでdepthが増えても動かない
        let depth_pre_singular = depth as i32;
        // TT手に与える延長。Noneは判定に入らなかったことを表す
        let singular_ext = match self.singular_extension::<PV>(&node, beta, &mut depth, prev) {
            Ok(e) => e,
            // multi-cutと打ち切りはこのノードごと抜ける
            Err(v) => return v,
        };

        // ply別の再利用バッファを借りる（ADR-0151の群B）。同一plyへ再帰する
        // singular検証・NMP検証はここへ来る前に終わっているので重ならない
        let buf = std::mem::take(&mut self.node_bufs[ply].moves);
        let mut picker = MovePicker::new(&self.pos, tt.mv, depth_pre_singular, ply, buf);
        // continuation historyの面（1手前から6手前まで。ADR-0109のG1）
        let cont = self.cont_bases(ply);
        // 最善にならなかった手を良い順に覚える（yaneuraou-search.cpp:2343-2344）
        let mut quiets_searched = std::mem::take(&mut self.node_bufs[ply].quiets);
        let mut captures_searched = std::mem::take(&mut self.node_bufs[ply].captures);
        // 子のPVの置き場。子の `search` が冒頭でclearするので、
        // 前のノードの中身が残っていても読まれない
        let mut child_pv = std::mem::take(&mut self.node_bufs[ply].child_pv);
        quiets_searched.clear();
        captures_searched.clear();

        // バッファの返却を一本化するため、ここから先の復帰点をこのブロックに
        // まとめる。途中で `return` すると返却が漏れる
        let node_value = 'node: {
            let mut best = -VALUE_INFINITE;
            let mut best_move = Move::NONE;
            let mut best_move_is_capture = false;
            let mut count = 0u32;

            while let Some(m) = picker.next(&self.pos, &self.hist, &cont) {
                // 除外手はスキップ（singular検証探索。ADR-0050）。通常はexcluded==NONE
                if m == excluded {
                    continue;
                }
                if !self.pos.is_legal(m) {
                    continue;
                }
                count += 1;
                // 1手先のノードが更新条件で読む（yaneuraou-search.cpp:3520）
                self.stack[ply + STACK_OFFSET].move_count = count;
                let is_capture = !m.is_drop() && !self.pos.piece_on(m.to()).is_empty();
                let gives_check = self.pos.gives_check(m);

                // 延長（yaneuraou-search.cpp:3543, 3872）。singularの判定に入った
                // TT手はその結果を使う。王手延長（ADR-0024）は参照実装が持たない
                // 本エンジンの機能なので残すが、singularの判定が付いた手では
                // singular側を採る（多段化した延長量を王手延長で潰さない）。
                // 参照実装は延長をムーブループの枝刈りの後で加えるので、枝刈りの
                // 尺度（lmr_depth）はこの値でなく `depth - 1` を基準にする
                let (extension, base_depth) = match singular_ext {
                    // singularの判定が付いたTT手。参照実装はムーブループの中で
                    // `newDepth = depth - 1` を先に決め、そのあとdepthを増やすので、
                    // 増えた1手はTT手自身には乗らない
                    // （yaneuraou-search.cpp:3556, 3788）
                    Some(e) if m == tt.mv => (e, depth_pre_singular),
                    _ => (i32::from(gives_check), depth as i32),
                };
                let mut new_depth = base_depth - 1 + extension;

                // LMRのリダクション量（1024倍の固定小数。G2）。枝刈りの尺度
                // （lmr_depth）と実際の浅い探索で同じ値を使う
                let delta = beta - alpha;
                let mut r = self.reduction(improving, depth, count, delta);
                // 項1: ttPvノードは削る（yaneuraou-search.cpp:3573-3574）。
                // 枝刈りの尺度に入るのはここまでで、残りはdo_moveの側で足す
                if self.stack[ply + STACK_OFFSET].tt_pv {
                    r += 1013;
                }
                // Step 14: 浅い深さでの枝刈り（yaneuraou-search.cpp:3586-3698）
                if self.prune_shallow::<PV>(
                    &node,
                    &mut picker,
                    &mut best,
                    m,
                    alpha,
                    depth,
                    count,
                    r,
                    improving,
                    is_capture,
                    gives_check,
                    best_move,
                    &cont,
                ) {
                    continue;
                }

                // リダクションの加減算（yaneuraou-search.cpp:3879-3941）
                let mut r = self.reduction_amount::<PV>(&node, m, &cont, r, alpha, depth, count);

                self.set_current_move(ply, m, is_capture);
                self.pos.do_move(m);
                self.evaluator.push(&self.pos);

                let mut value = -VALUE_INFINITE;
                if depth >= 2 && count > 1 {
                    // LMR（yaneuraou-search.cpp:3954-4010）。参照実装の発動条件は
                    // 深さと手数だけで、取る手も王手する手も対象にする。
                    // リダクションが負なら `new_depth + 2` まで深く読む
                    let d = (new_depth - r / 1024).min(new_depth + 2).max(1) + i32::from(PV);
                    self.stack[ply + STACK_OFFSET].reduction = new_depth - d;
                    value = -self.search::<false>(
                        -alpha - 1,
                        -alpha,
                        d as u32,
                        ply + 1,
                        m,
                        &mut child_pv,
                        true,
                    );
                    self.stack[ply + STACK_OFFSET].reduction = 0;
                    if value > alpha {
                        // 減深探索の結果で読み直す深さを調整する
                        // （yaneuraou-search.cpp:3997-4000）。十分良ければ深く、
                        // 十分悪ければ浅くする
                        let do_deeper = d < new_depth && value > best + 48;
                        let do_shallower = value < best + 9;
                        new_depth += i32::from(do_deeper) - i32::from(do_shallower);
                        if new_depth > d && !self.stopped() {
                            value = -self.search::<false>(
                                -alpha - 1,
                                -alpha,
                                new_depth.max(0) as u32,
                                ply + 1,
                                m,
                                &mut child_pv,
                                !cut_node,
                            );
                        }
                        // LMR後のcontinuation history更新
                        // （yaneuraou-search.cpp:4008）
                        self.update_continuation_histories(ply, m.piece_after(), m.to(), 1426);
                    }
                } else if !PV || count > 1 {
                    // LMRを省いたときの調整（yaneuraou-search.cpp:4017-4030）。
                    // 項12: TT手がなければ削る。削る量が大きければ深さを落とす
                    if tt.mv == Move::NONE {
                        r += 1057;
                    }
                    let d = new_depth - i32::from(r > 4628) - i32::from(r > 5772 && new_depth > 2);
                    value = -self.search::<false>(
                        -alpha - 1,
                        -alpha,
                        d.max(0) as u32,
                        ply + 1,
                        m,
                        &mut child_pv,
                        !cut_node,
                    );
                }
                // PVノードは第1手とfail highの後だけ全窓で読み直す
                // （yaneuraou-search.cpp:4043-4061）
                if PV && (count == 1 || value > alpha) && !self.stopped() {
                    // 静止探索へ直行する手前で、TT手だけは1手残す
                    // （yaneuraou-search.cpp:4053-4057）。負の延長でnew_depthが
                    // 0以下になったTT手をqsearchへ落とすと、詰みの発見が鈍る
                    if m == tt.mv
                        && ((tt.value != VALUE_NONE
                            && tt.value.abs() >= VALUE_MATE_IN_MAX_PLY
                            && tt.depth > 0)
                            || tt.depth > 1)
                    {
                        new_depth = new_depth.max(1);
                    }
                    value = -self.search::<true>(
                        -beta,
                        -alpha,
                        new_depth.max(0) as u32,
                        ply + 1,
                        m,
                        &mut child_pv,
                        false,
                    );
                }
                self.evaluator.pop();
                self.pos.undo_move(m);
                if self.stopped() {
                    break 'node VALUE_ZERO;
                }

                if value > best {
                    best = value;
                    if value > alpha {
                        best_move = m;
                        best_move_is_capture = is_capture;
                        if PV {
                            pv.clear();
                            pv.push(m);
                            pv.extend_from_slice(&child_pv);
                        }
                        if value >= beta {
                            // 次plyのfail highの多さをリダクションへ渡す
                            // （yaneuraou-search.cpp:4214）。2手以上延長した手の
                            // カットは数えない（G5で延長が最大+3になった）
                            if extension < 2 || PV {
                                self.stack[ply + STACK_OFFSET].cutoff_cnt += 1;
                            }
                            break;
                        }
                        // alphaを更新できたので、残りの手を浅く読む
                        // （yaneuraou-search.cpp:4228-4229）。決着スコアのときは
                        // 深さを保って読み切る
                        if depth > 2 && depth < 14 && value.abs() < VALUE_MATE_IN_MAX_PLY {
                            depth -= 2;
                        }
                        alpha = value;
                    }
                }

                // 最善でなかった手を、あとでmalusを配るために覚えておく
                // （yaneuraou-search.cpp:4246-4256）。上限は32手
                if m != best_move && count <= SEARCHED_LIST_CAPACITY {
                    if is_capture {
                        captures_searched.push(m);
                    } else {
                        quiets_searched.push(m);
                    }
                }
            }

            if count == 0 {
                // 除外手つき探索（singularの検証）では、TT手を除いたせいで手が
                // 尽きただけなのでfail lowのスコアを返す（yaneuraou-search.cpp:4295）。
                // 詰みの値を返すと、TT手が唯一の合法手であるノードの検証値が
                // 常に最小になり、多段延長が必ず最大まで積む
                if excluded != Move::NONE {
                    break 'node alpha;
                }
                // 合法手なし = 詰み（将棋はステイルメイトも負け）
                break 'node mated_in(ply);
            }

            self.finalize_node::<PV>(
                &node,
                depth,
                alpha,
                beta,
                best,
                best_move,
                best_move_is_capture,
                &quiets_searched,
                &captures_searched,
            );
            best
        };

        // 借りたバッファを戻す。容量が残るので次の同じplyでは確保が起きない
        self.node_bufs[ply].moves = picker.into_buf();
        self.node_bufs[ply].quiets = quiets_searched;
        self.node_bufs[ply].captures = captures_searched;
        self.node_bufs[ply].child_pv = child_pv;
        node_value
    }

    /// ノードの終端処理（yaneuraou-search.cpp:4299-4418）。指し手の統計、
    /// 1手前のttPvの引き継ぎ、TT store、correction historyの更新をこの順に行う。
    ///
    /// `quiets_searched` と `captures_searched` は、このノードで調べたが
    /// 最善にならなかった手を良い順に並べたもの。
    #[allow(clippy::too_many_arguments)]
    fn finalize_node<const PV: bool>(
        &mut self,
        node: &NodeInfo,
        depth: u32,
        alpha: Value,
        beta: Value,
        best: Value,
        best_move: Move,
        best_move_is_capture: bool,
        quiets_searched: &[Move],
        captures_searched: &[Move],
    ) {
        let &NodeInfo {
            ply,
            in_check,
            excluded,
            key,
            raw_eval,
            static_eval,
            prior_capture,
            prev_sq,
            tt,
            ..
        } = node;
        // 指し手の統計を更新する（yaneuraou-search.cpp:4299-4356）。
        // βカットしていなくても、alphaを更新した手があれば更新する
        if best_move != Move::NONE {
            self.update_all_stats(
                ply,
                depth,
                best_move,
                tt.mv,
                prev_sq,
                prior_capture,
                quiets_searched,
                captures_searched,
            );
            // 非PVノードのbestMove確定時の更新（yaneuraou-search.cpp:4308）。
            // もう1か所、multi-cutが減点する（yaneuraou-search.cpp:3819）
            if !PV {
                self.hist
                    .tt_move
                    .update(if best_move == tt.mv { 805 } else { -787 });
            }
        } else if let Some(prev_sq) = prev_sq
            && prior_capture.is_empty()
        {
            // fail lowを引き起こした1手前の静かな手への加点
            // （yaneuraou-search.cpp:4320-4341）
            let prev1 = self.stack[ply + STACK_OFFSET - 1];
            let mut bonus_scale = -232;
            bonus_scale -= prev1.stat_score / 108;
            bonus_scale += (59 * depth as i32).min(454);
            bonus_scale += 169 * i32::from(prev1.move_count > 8);
            bonus_scale += 145 * i32::from(!in_check && best <= static_eval - 110);
            bonus_scale += 154 * i32::from(!prev1.in_check && best <= -prev1.static_eval - 73);
            let bonus_scale = bonus_scale.max(0);
            let scaled = (135 * depth as i32 - 80).min(1400) * bonus_scale;

            let pc = self.pos.piece_on(prev_sq);
            self.update_continuation_histories(ply - 1, pc, prev_sq, scaled * 221 / 16384);
            let them = self.pos.side_to_move().flip();
            self.hist
                .main
                .update(them, prev1.current_move, scaled * 235 / 32768);
            if pc.piece_type() != PieceType::PAWN && !prev1.current_move.is_promote() {
                let slot = self.shared.hists.pawn.slot(self.pos.pawn_key());
                self.shared
                    .hists
                    .pawn
                    .update(slot, pc, prev_sq, scaled * 290 / 8192);
            }
        } else if let Some(prev_sq) = prev_sq {
            // fail lowを引き起こした1手前の取る手への加点
            // （yaneuraou-search.cpp:4346-4351）
            self.hist.capture.update(
                self.pos.piece_on(prev_sq),
                prev_sq,
                prior_capture.piece_type(),
                1018,
            );
        }

        // 良い手が見つからなかったなら1手前のttPvを引き継ぐ
        // （yaneuraou-search.cpp:4367-4368）。1手前が探索木に入れた変化なら、
        // この局面も探索木へ加える
        if best <= alpha {
            self.stack[ply + STACK_OFFSET].tt_pv =
                self.stack[ply + STACK_OFFSET].tt_pv || self.stack[ply + STACK_OFFSET - 1].tt_pv;
        }

        // 除外手つき探索中はTT storeをしない（ADR-0050）。
        // 検証専用の結果でこのキーの本体を汚さない
        if excluded == Move::NONE {
            let bound = if best >= beta {
                Bound::Lower
            } else if PV && best_move != Move::NONE {
                Bound::Exact
            } else {
                Bound::Upper
            };
            self.shared.tt.store(
                key,
                best_move.to_move16(),
                value_to_tt(best, ply),
                raw_eval as i16,
                (depth as i32 + TT_DEPTH_OFFSET).min(255) as u8,
                bound,
                // 参照実装はis_pvではなくttPvを書き戻す
                // （yaneuraou-search.cpp:4397）
                self.stack[ply + STACK_OFFSET].tt_pv,
            );
        }

        // correction history更新（ADR-0046, 0109）。最善手が取る手でなく、
        // 誤差の向きが最善手の有無と一致するときだけ蓄積する
        // （yaneuraou-search.cpp:4410-4418）。参照実装は除外手つき探索でも
        // 更新するので、TT storeと違いここは条件に入れない
        if !in_check
            && !(best_move != Move::NONE && best_move_is_capture)
            && (best > static_eval) == (best_move != Move::NONE)
        {
            let w = if best_move != Move::NONE { 12 } else { 17 };
            // クランプ幅は値域の1/4（history.h:30のCORRECTION_HISTORY_LIMIT=1024）
            let bonus = ((best - static_eval) * depth as i32 * w / 128).clamp(-256, 256);
            self.update_correction_history(ply, 1069 * bonus / 1024);
        }
    }

    /// LMRのリダクション量の加減算（G2。yaneuraou-search.cpp:3879-3941）。
    /// 参照実装はdo_moveの後に置くが、読む材料は進める前の局面で決まるので
    /// ここでまとめる。項1（ttPv）は枝刈りの尺度にも入るため呼び出し側にある。
    ///
    /// 項10の途中で、その手の履歴の強さをStackへ控える。子のhistory更新量も
    /// この値を読むので、do_moveの前に測る必要がある
    #[allow(clippy::too_many_arguments)]
    fn reduction_amount<const PV: bool>(
        &mut self,
        node: &NodeInfo,
        m: Move,
        cont: &[usize; 6],
        mut r: i32,
        alpha: Value,
        depth: u32,
        count: u32,
    ) -> i32 {
        let &NodeInfo {
            ply,
            cut_node,
            all_node,
            corr_value,
            tt,
            tt_capture,
            ..
        } = node;
        // 項2: ttPvノードは大きく戻す。TTの値がalphaを超える、TTの
        // 深さが足りている、といった手掛かりがあるほど戻す
        if self.stack[ply + STACK_OFFSET].tt_pv {
            r -= 2819
                + i32::from(PV) * 973
                + i32::from(tt.value > alpha) * 905
                + i32::from(tt.depth >= depth as i32) * (935 + i32::from(cut_node) * 959);
        }
        // 項3: 他の調整を補正する基準オフセット
        r += 691;
        // 項4: 手数が進むほど戻す
        r -= count as i32 * 65;
        // 項5: correction historyの補正が大きい局面は戻す
        r -= corr_value.abs() / 25600;
        // 項6: cutNodeは削る。TT手がなければさらに削る
        if cut_node {
            r += 3611 + 985 * i32::from(tt.mv == Move::NONE);
        }
        // 項7: TT手が駒を取る手なら削る
        if tt_capture {
            r += 1054;
        }
        // 項8: 次plyでfail highが多いなら削る
        let child_cutoffs = self.stack[ply + STACK_OFFSET + 1].cutoff_cnt;
        if child_cutoffs > 1 {
            r += 251 + 1124 * i32::from(child_cutoffs > 2) + 1042 * i32::from(all_node);
        }
        // 項9: TT手は戻す
        if m == tt.mv {
            r -= 2239;
        }

        // その手の履歴の強さを控える（yaneuraou-search.cpp:3924-3932）。
        // 子のhistory更新量にも効くのでdo_moveの前に測る
        let stat_score = self.stat_score(m, cont);
        self.stack[ply + STACK_OFFSET].stat_score = stat_score;
        // 項10: 履歴の良い手は戻し、悪い手は削る
        r -= stat_score * 428 / 4096;
        // 項11: allNodeでは全体を割り増す
        if all_node {
            r += r * 273 / (256 * depth as i32 + 260);
        }
        r
    }

    /// Step 14。浅い深さでの枝刈り（yaneuraou-search.cpp:3586-3698）。
    /// 真を返したら、その手は読まずに次の手へ進む。
    ///
    /// 前提条件は「rootでない」「bestValueが敗勢でない」の2つだけで、
    /// search()は常にrootでない。`best` は1手目を読み終えるまで
    /// -VALUE_INFINITEなので、第1手はここで刈られない。親futilityは
    /// 刈った手の見込み値で `best` を引き上げる
    #[allow(clippy::too_many_arguments)]
    fn prune_shallow<const PV: bool>(
        &mut self,
        node: &NodeInfo,
        picker: &mut MovePicker,
        best: &mut Value,
        m: Move,
        alpha: Value,
        depth: u32,
        count: u32,
        r: i32,
        improving: bool,
        is_capture: bool,
        gives_check: bool,
        best_move: Move,
        cont: &[usize; 6],
    ) -> bool {
        if *best <= VALUE_MATED_IN_MAX_PLY {
            return false;
        }
        let &NodeInfo {
            in_check,
            follow_pv,
            static_eval,
            ..
        } = node;
        // move count pruning（yaneuraou-search.cpp:3592-3593）: 手数を
        // 使い切ったら、MovePickerに静かな手の生成そのものをやめさせる
        if count >= lmp_limit(depth, improving) {
            picker.skip_quiet_moves();
        }

        // lmr_depth: LMRで削ったあとに実際に読む深さ
        // （yaneuraou-search.cpp:3599-3600）。生のdepthで枝刈りを
        // 判断すると、深いノードほど閾値が大きくなり刈りすぎる。
        // 参照実装は延長を加える前の `depth - 1` を基準にする
        let mut lmr_depth = depth as i32 - 1 - r / 1024;

        if is_capture || gives_check {
            // 取る駒（駒打ちと王手だけの手ではEMPTY）と、その
            // capture history（yaneuraou-search.cpp:3612-3614）
            let captured = self.pos.piece_on(m.to()).piece_type();
            let capt_hist = self.hist.capture.get(m.piece_after(), m.to(), captured);

            // 取る手のfutility（yaneuraou-search.cpp:3616-3623）。
            // 取った駒の価値を足してもalphaに届かない手を捨てる。
            // 王手する手は対象外
            if !gives_check && lmr_depth < CAPT_FUTILITY_MAX_DEPTH {
                let futility_value = static_eval
                    + CAPT_FUTILITY_BASE
                    + CAPT_FUTILITY_DEPTH * lmr_depth
                    + himawari_core::piece_value(captured)
                    + CAPT_FUTILITY_HIST * capt_hist / 1024;
                if futility_value <= alpha {
                    return true;
                }
            }

            // 取る手・王手する手のSEE枝刈り
            // （yaneuraou-search.cpp:3634-3641）。許す損の額が
            // capture historyで動く。alphaが負のときは刈らない
            let margin =
                (SEE_CAPTURE_COEF * depth as i32 + capt_hist * SEE_CAPT_HIST / 1024).max(0);
            if alpha >= VALUE_DRAW && !self.pos.see_ge(m, -margin) {
                return true;
            }
        } else if !follow_pv || !PV {
            // 前回の反復深化のPV上にいるPVノードでは、静かな手の
            // 枝刈りを一切かけない（yaneuraou-search.cpp:3644）。
            // 前回のPVを浅い枝刈りで壊さないための仕掛けである
            //
            // 静かな手の履歴（yaneuraou-search.cpp:3646-3648）。
            // 1手前・2手前のcontinuation historyとpawn historyの和
            let to = m.to();
            let pc = m.piece_after();
            let pawn_slot = self.shared.hists.pawn.slot(self.pos.pawn_key());
            let mut history = self.hist.cont.get(cont[0], pc, to)
                + self.hist.cont.get(cont[1], pc, to)
                + self.shared.hists.pawn.get(pawn_slot, pc, to);

            // continuation historyによる枝刈り
            // （yaneuraou-search.cpp:3650-3651）。履歴が極端に
            // 悪い手は読まない
            if history < -CONT_HIST_PRUNE_COEF * depth as i32 {
                return true;
            }

            // main historyを足してlmr_depthを補正する
            // （yaneuraou-search.cpp:3656-3661）。以降の枝刈りが
            // 使う尺度そのものが履歴で動く
            history += 71 * self.hist.main.get(self.pos.side_to_move(), m) / 32;
            lmr_depth += history / LMR_DEPTH_HIST_DIVISOR;

            // 親ノードのfutility（yaneuraou-search.cpp:3665-3682）。
            // 子を展開する前に、alphaへ届かないと見込める静かな手を
            // 捨てる。尺度は補正後のlmr_depthで、まだ最善手が
            // 見つかっていないときと静的評価がalphaを超えている
            // ときにマージンを積む
            let futility_value = static_eval
                + FUTILITY_BASE
                + FUTILITY_NO_BEST * Value::from(best_move == Move::NONE)
                + FUTILITY_MARGIN * lmr_depth
                + FUTILITY_OVER_ALPHA * Value::from(static_eval > alpha);
            if !in_check && lmr_depth < FUTILITY_MAX_DEPTH && futility_value <= alpha {
                // 刈った手の見込み値でbestValueを引き上げる。
                // 詰み圏の値は動かさない
                if *best <= futility_value
                    && best.abs() < VALUE_MATE_IN_MAX_PLY
                    && futility_value < VALUE_MATE_IN_MAX_PLY
                {
                    *best = futility_value;
                }
                return true;
            }

            // 負のSEEを持つ手の枝刈り（yaneuraou-search.cpp:3691-3698）。
            // 参照実装はここで0止めする
            let lmr_depth = lmr_depth.max(0);
            if !self.pos.see_ge(m, -SEE_QUIET_COEF * lmr_depth * lmr_depth) {
                return true;
            }
        }
        false
    }

    /// singular extension（ADR-0050, 0109のG5。yaneuraou-search.cpp:3745-3850）。
    /// TT手を除外した検証探索がsingular_betaを下回れば、TT手だけが傑出して
    /// いると見て延長する。参照実装はムーブループの中でTT手に当たったときに
    /// 判定するが、対象はTT手だけで、TT手はMovePickerが最初に返す。ループの
    /// 手前で1回求めても同じである（ムーブループの枝刈りは第1手には効かない）。
    ///
    /// 検証値の位置で3通りに分かれる。singular_beta未満なら延長して `depth`
    /// も1手増やし、beta以上ならmulti-cutでこのノードごと刈り、間なら
    /// negative extensionになる。multi-cutと打ち切りは `Err` で返し、
    /// 呼び出し側がそのままreturnする。
    fn singular_extension<const PV: bool>(
        &mut self,
        node: &NodeInfo,
        beta: Value,
        depth: &mut u32,
        prev: Move,
    ) -> Result<Option<i32>, Value> {
        let &NodeInfo {
            ply,
            cut_node,
            excluded,
            static_eval,
            tt,
            ..
        } = node;
        let mut singular_ext: Option<i32> = None;
        if excluded == Move::NONE
            && ply > 0
            && tt.mv != Move::NONE
            // ttPvノードでは1手深いところから判定する
            && *depth >= SINGULAR_MIN_DEPTH + u32::from(self.stack[ply + STACK_OFFSET].tt_pv)
            && tt.bound != Bound::Upper
            && tt.bound != Bound::None
            && tt.depth >= (*depth).saturating_sub(3) as i32
            && tt.value.abs() < VALUE_MATE_IN_MAX_PLY
            // 往復手は検証探索にかけない（G5。yaneuraou-search.cpp:3749）
            && !self.is_shuffling(tt.mv, ply)
            && self.pos.is_legal(tt.mv)
        {
            let singular_beta = tt.value
                - (SINGULAR_MARGIN
                    + SINGULAR_MARGIN_TTPV
                        * Value::from(self.stack[ply + STACK_OFFSET].tt_pv && !PV))
                    * *depth as Value
                    / SINGULAR_MARGIN_DIV;
            // 検証探索の深さは延長前のnewDepth（= *depth - 1）の半分
            // （yaneuraou-search.cpp:3758）
            let singular_depth = (*depth - 1) / 2;
            self.stack[ply + STACK_OFFSET].excluded_move = tt.mv;
            let mut verify_pv = Vec::new();
            let v = self.search::<false>(
                singular_beta - 1,
                singular_beta,
                singular_depth,
                ply,
                prev,
                &mut verify_pv,
                // 検証探索はcut_nodeを引き継ぐ（ADR-0109のG0）
                cut_node,
            );
            self.stack[ply + STACK_OFFSET].excluded_move = Move::NONE;
            // 検証探索の再帰でstatic_evalが同値で上書きされる。念のため戻す
            self.stack[ply + STACK_OFFSET].static_eval = static_eval;
            if self.stopped() {
                return Err(VALUE_ZERO);
            }
            // 検証値の位置で3通りに分かれる。singular_beta未満なら多段延長、
            // beta以上ならmulti-cut、間ならnegative extensionになる
            if v < singular_beta {
                // 多段延長（yaneuraou-search.cpp:3777-3788）。検証値が
                // singular_betaをマージン分下回るごとに1手積み、最大+3にする。
                // 組合せ爆発を抑えるため、PVノードではマージンを大きく取って
                // 積みにくくする。TT手が取る手でない・correction値が大きい・
                // ttMoveHistoryが良い・rootから遠い、では積みやすくなる
                // 多段延長は入れない（ADR-0114の二分割）。参照実装の
                // doubleMargin / tripleMarginをそのまま移すと、本エンジンでは
                // singular率が43.5%（参照実装の設計点は1割）あるため+3まで
                // 積む機会が4倍以上になる。群全体では562局で-99.0だった。
                // 到達深さが30秒で4段落ちたのが主因である。
                // 検証窓の係数は本エンジンで成立していないので、多段化は
                // singular率を設計点へ寄せてから再訪する
                singular_ext = Some(1);
                // このノード自体の深さも1手増やす（yaneuraou-search.cpp:3788）。
                // TT手のnewDepthは増やす前の値で決まっているので、増分は
                // 残りの手とTT storeに効く
                *depth += 1;
            } else if v >= beta && v.abs() < VALUE_MATE_IN_MAX_PLY {
                // multi-cut（yaneuraou-search.cpp:3817-3821）。TT手を除いた
                // 浅い探索でもβを超えたので、このノードは「1手だけ傑出」では
                // なく複数の手がfail highすると見て、部分木をまとめて刈る。
                // 返す値はsoftbound（真の値がこれ以上と分かっている値）である
                self.hist
                    .tt_move
                    .update((-424 - 107 * *depth as i32).max(-3375));
                return Err(v);
            } else if tt.value >= beta {
                // negative extension（yaneuraou-search.cpp:3841-3850）。
                // 検証値がsingular_betaとβの間なので、singularともmulti-cutとも
                // 言えない。TT手が今のβを超えてfail highすると見込めるなら
                // 大きく削り、他の手を先に読ませる
                singular_ext = Some(-3);
            } else if cut_node {
                // 同じくcutNodeだが、TT手がβを超えるとは見込めない場合
                singular_ext = Some(-2);
            } else {
                // どの分岐にも入らない場合は延長も短縮もしない。
                // 参照実装のextensionの初期値0に対応する
                singular_ext = Some(0);
            }
        }
        Ok(singular_ext)
    }

    /// ムーブループの手前の枝刈り（ADR-0028, 0057, 0109のG4。
    /// yaneuraou-search.cpp:3159-3424）。razoring・RFP・NMP・IIR・ProbCutを
    /// この順に試す。刈れたらその値を返し、呼び出し側はそのままreturnする。
    ///
    /// 深さの事後補正とIIRが `depth` を、NMPの前後の再計算が `improving` を
    /// 書き換える。刈らずに抜けたときだけ呼び出し側へ反映される。
    fn prune_before_moves<const PV: bool>(
        &mut self,
        node: &NodeInfo,
        alpha: Value,
        beta: Value,
        depth: &mut u32,
        improving: &mut bool,
        prev: Move,
    ) -> Option<Value> {
        // 王手中はevalベースの枝刈りを一切行わない
        // （yaneuraou-search.cpp:3013-3020のgoto moves_loop）。
        // 静的評価が2手前の写しでしかないため、判断材料にできない
        if node.in_check {
            return None;
        }
        let &NodeInfo {
            ply,
            cut_node,
            all_node,
            follow_pv,
            excluded,
            key,
            raw_eval,
            static_eval,
            eval,
            corr_value,
            prior_reduction,
            tt,
            tt_capture,
            ..
        } = node;
        *improving = static_eval > self.stack[ply + STACK_OFFSET - 2].static_eval;
        // 相手の状況が悪化しているか（yaneuraou-search.cpp:3169）。
        // 普通は `static_eval == -(1手前のstatic_eval)` なので、これを
        // 上回るなら相手にとって評価が悪くなっている
        let opponent_worsening = static_eval > -self.stack[ply + STACK_OFFSET - 1].static_eval;

        // 1手前のリダクションに応じた残り深さの事後補正
        // （yaneuraou-search.cpp:3176-3179）。深く削って戻ってきた手が
        // 相手を悪くできていないなら1手足し、静的評価の和が閾値を超えて
        // いるなら1手引く
        if prior_reduction >= 3 && !opponent_worsening {
            *depth += 1;
        }
        if prior_reduction >= 2
            && *depth >= 2
            && static_eval + self.stack[ply + STACK_OFFSET - 1].static_eval > 173
        {
            *depth -= 1;
        }

        // razoring（ADR-0057, 0109のG4。yaneuraou-search.cpp:3191-3192）。
        // 評価がalphaを大きく下回るなら通常探索をやめ、qsearchの値を返す。
        // PVノードでないことが唯一の前提で、深さの上限はない
        if !PV && eval < alpha - RAZOR_BASE - RAZOR_DEPTH_COEF * (*depth * *depth) as Value {
            // razoringは非PVノード限定なので常にNonPV（yaneuraou-search.cpp:3192）
            return Some(self.qsearch::<false>(alpha, beta, ply));
        }

        // 子ノードのfutility（RFP。yaneuraou-search.cpp:3217-3227）。
        // 残り深さで評価が動きうる幅を見積り、それを引いてもβを超えるなら
        // 刈る。TTにヒットしていないノードは見積りを狭める
        let futility_mult =
            RFP_MULT - RFP_NO_TT_HIT * i32::from(!self.stack[ply + STACK_OFFSET].tt_hit);
        let futility_margin = futility_mult * *depth as i32
            - (RFP_IMPROVING * i32::from(*improving)
                + RFP_OPP_WORSENING * i32::from(opponent_worsening))
                * futility_mult
                / 1024
            + corr_value.abs() / RFP_CORR_DIVISOR;
        if !self.stack[ply + STACK_OFFSET].tt_pv
            && *depth < RFP_MAX_DEPTH
            && eval >= beta
            && eval - futility_margin >= beta
            && (tt.mv == Move::NONE || tt_capture)
            && beta > VALUE_MATED_IN_MAX_PLY
            && eval < VALUE_MATE_IN_MAX_PLY
        {
            // 静的評価そのものではなく、βへ寄せた値を返す
            return Some((2 * beta + eval) / 3);
        }

        // NMP（ADR-0028, 0109のG4。yaneuraou-search.cpp:3236-3301）。
        // 手番を渡して浅く探索し、それでもβ以上なら刈る。cutNode限定で、
        // 深さの下限はない。評価の閾値はβから残り深さとimprovingで割り引く。
        // 除外手つき探索中はスキップ（ADR-0050）
        if cut_node
            && static_eval
                >= beta
                    - NMP_EVAL_DEPTH * *depth as Value
                    - NMP_EVAL_IMPROVING * Value::from(*improving)
                    + NMP_EVAL_BASE
            && excluded == Move::NONE
            && ply >= self.nmp_min_ply
            && beta > VALUE_MATED_IN_MAX_PLY
        {
            // 連続してnull moveは指さない（yaneuraou-search.cpp:3247）。
            // null moveの子はcut_node = falseなのでここへ来ない
            debug_assert!(prev != Move::NULL);
            let r = NMP_BASE_REDUCTION + *depth / NMP_DEPTH_DIVISOR;
            let mut null_pv = Vec::new();
            // null moveは王手でも駒取りでもないので番兵の面を指す
            // （yaneuraou-search.cpp:3254-3256）
            let e = &mut self.stack[ply + STACK_OFFSET];
            e.current_move = Move::NULL;
            e.cont_base = ContinuationHistory::SENTINEL;
            e.cont_corr_base = 0;
            self.pos.do_null_move();
            self.evaluator.push(&self.pos);
            let v = -self.search::<false>(
                -beta,
                -beta + 1,
                (*depth).saturating_sub(r),
                ply + 1,
                Move::NULL,
                &mut null_pv,
                // NMPの子はcut_node = false（ADR-0109のG0）
                false,
            );
            self.evaluator.pop();
            self.pos.undo_null_move();
            if self.stopped() {
                return Some(VALUE_ZERO);
            }
            // パス由来の詰みスコアは信用しない。刈らずに読み進める
            if v >= beta && v < VALUE_MATE_IN_MAX_PLY {
                // 深いところでは同じ深さの検証探索で裏を取る
                // （yaneuraou-search.cpp:3277-3301）。zugzwangでの誤りを
                // 減らす。検証探索の中ではnmpMinPlyまでNMPを止める
                if self.nmp_min_ply != 0 || *depth < NMP_VERIFY_MIN_DEPTH {
                    return Some(v);
                }
                self.nmp_min_ply = ply + 3 * (*depth - r) as usize / 4;
                let mut verify_pv = Vec::new();
                let vv = self.search::<false>(
                    beta - 1,
                    beta,
                    *depth - r,
                    ply,
                    prev,
                    &mut verify_pv,
                    false,
                );
                self.nmp_min_ply = 0;
                if self.stopped() {
                    return Some(VALUE_ZERO);
                }
                if vv >= beta {
                    return Some(v);
                }
            }
        }

        // NMPの後にβで再計算する（yaneuraou-search.cpp:3306）。
        // 静的評価がβ以上なら、2手前と比べていなくても改善扱いにする
        *improving |= static_eval >= beta;

        // IIR（ADR-0028, 0109のG4。yaneuraou-search.cpp:3319-3320）。
        // TTに手がないノードは良い順序を作れないので1浅く読み、再訪時に
        // TT手付きで読み直す。前回PVの上と、全手を読むallNodeでは行わない。
        // 1手前を深く削って来たノードも対象から外す
        if !follow_pv
            && !all_node
            && *depth >= IIR_MIN_DEPTH
            && tt.mv == Move::NONE
            && prior_reduction <= IIR_MAX_PRIOR_REDUCTION
        {
            *depth -= 1;
        }

        // ProbCut（ADR-0051, 0109のG4。yaneuraou-search.cpp:3357-3424）。
        // betaを大きく超えそうなノードでは、浅い確認探索で「十分良い取る手が
        // 1つある」ことを示せれば高深度の全探索を省いてカットする。
        // 閾値はimprovingで動き、MovePickerがSEEでこの閾値を満たす取る手
        // だけをcapture history込みの順序で返す
        let probcut_beta = beta + PROBCUT_MARGIN - PROBCUT_IMPROVING * Value::from(*improving);
        if *depth >= PROBCUT_MIN_DEPTH
            && beta.abs() < VALUE_MATE_IN_MAX_PLY
            // 置換表の値がprobcut_beta未満と分かっているなら試さない
            && !(tt.value != VALUE_NONE && tt.value < probcut_beta)
        {
            let probcut_depth = *depth as i32 - PROBCUT_DEPTH_REDUCTION;
            // ply別の再利用バッファを借りる（ADR-0151の群B）。このplyの
            // MovePickerは同時に1つしか生きないので、同じスロットを使える
            let buf = std::mem::take(&mut self.node_bufs[ply].moves);
            let mut picker =
                MovePicker::new_probcut(&self.pos, tt.mv, probcut_beta - static_eval, buf);
            let cont = self.cont_bases(ply);
            // バッファの返却を一本化するため、復帰点をこのブロックにまとめる
            let probcut_value = 'probcut: {
                while let Some(m) = picker.next(&self.pos, &self.hist, &cont) {
                    // 除外手はsingular検証探索中のTT手（ADR-0050）
                    if m == excluded || !self.pos.is_legal(m) {
                        continue;
                    }
                    self.set_current_move(ply, m, !self.pos.piece_on(m.to()).is_empty());
                    self.pos.do_move(m);
                    self.evaluator.push(&self.pos);
                    // まずqsearchで確認（窓は (-probcut_beta, -probcut_beta+1)）
                    let mut v = -self.qsearch::<false>(-probcut_beta, -probcut_beta + 1, ply + 1);
                    // 通ったら同じ窓で通常探索 *depth-4 を確認する
                    if v >= probcut_beta && probcut_depth > 0 {
                        let mut child_pv = Vec::new();
                        v = -self.search::<false>(
                            -probcut_beta,
                            -probcut_beta + 1,
                            probcut_depth as u32,
                            ply + 1,
                            m,
                            &mut child_pv,
                            // ProbCutの子はcut_nodeを反転する（ADR-0109のG0）
                            !cut_node,
                        );
                    }
                    self.evaluator.pop();
                    self.pos.undo_move(m);
                    if self.stopped() {
                        break 'probcut Some(VALUE_ZERO);
                    }
                    if v >= probcut_beta {
                        // fail-soft。TTにlower bound・*depth-3で保存する
                        self.shared.tt.store(
                            key,
                            m.to_move16(),
                            value_to_tt(v, ply),
                            raw_eval as i16,
                            (probcut_depth + 1 + TT_DEPTH_OFFSET).clamp(0, 255) as u8,
                            Bound::Lower,
                            // 参照実装はttPvを書き戻す（yaneuraou-search.cpp:3418）
                            self.stack[ply + STACK_OFFSET].tt_pv,
                        );
                        // 決着スコアでなければ、上乗せしたマージンを戻して
                        // カットする。決着スコアのときは次の手を試す
                        if v.abs() < VALUE_MATE_IN_MAX_PLY {
                            break 'probcut Some(v - (probcut_beta - beta));
                        }
                    }
                }
                None
            };
            // 借りたバッファを戻す
            self.node_bufs[ply].moves = picker.into_buf();
            if probcut_value.is_some() {
                return probcut_value;
            }
        }
        None
    }

    /// 静止探索（ADR-0024, 0109のG6）。出典はやねうら王の `qsearch()`
    /// （yaneuraou-search.cpp:4441-5145）。参照実装は `qsearch<PV>` と
    /// `qsearch<NonPV>` をテンプレートで分けるので、const genericの `PV` で
    /// 受ける（ADR-0151の群J）。
    fn qsearch<const PV: bool>(&mut self, mut alpha: Value, beta: Value, ply: usize) -> Value {
        if self.stopped() {
            return VALUE_ZERO;
        }
        self.sel_depth = self.sel_depth.max(ply as u32);
        self.nodes += 1;
        self.check_limits();
        if ply >= MAX_PLY {
            return self.evaluator.evaluate(&self.pos);
        }
        if let Some(v) = self.terminal_value(ply) {
            return v;
        }

        // Step 3. 置換表probe（ADR-0054。yaneuraou-search.cpp:4645-4685）
        let key = self.pos.key();
        let tt_hit = self.shared.tt.probe(key);
        // 参照実装はqsearchでもStackへ記録する（yaneuraou-search.cpp:4648）
        self.stack[ply + STACK_OFFSET].tt_hit = tt_hit.is_some();
        // 置換表にPVノードとして記録された値か（yaneuraou-search.cpp:4657）。
        // 末尾のstoreへそのまま書き戻す
        let pv_hit = tt_hit.as_ref().is_some_and(|d| d.pv);
        let mut tt_move = Move::NONE;
        let mut tt_eval = VALUE_NONE;
        let mut tt_value = VALUE_NONE;
        let mut tt_bound = Bound::None;
        if let Some(data) = &tt_hit {
            if let Some(m) = self.pos.to_move(data.mv)
                && self.pos.pseudo_legal(m)
            {
                tt_move = m;
            }
            tt_eval = Value::from(data.eval);
            tt_value = value_from_tt(data.value, ply);
            tt_bound = data.bound;
            // TTカットは非PVノードだけで行う（yaneuraou-search.cpp:4670-4679）。
            // PVノードでは前回evaluateした値が使えるのでカットしない。
            // DEPTH_UNSEARCHEDで書かれたstand patの記録はここを通らない
            if !PV
                && data.depth >= TT_DEPTH_QS
                && tt_value != VALUE_NONE
                // 原典の `bound & (value >= beta ? LOWER : UPPER)` をそのまま写す
                && if tt_value >= beta {
                    matches!(tt_bound, Bound::Lower | Bound::Exact)
                } else {
                    matches!(tt_bound, Bound::Upper | Bound::Exact)
                }
            {
                return tt_value;
            }
        }

        let in_check = self.pos.in_check();
        // continuation historyの面を決める材料（ADR-0109のG1）
        self.stack[ply + STACK_OFFSET].in_check = in_check;

        // Step 4. 静的評価（yaneuraou-search.cpp:4713-4837）。
        // rawは補正前でTT storeのeval欄へそのまま入る。王手中は定義しない
        let mut raw_eval = VALUE_NONE;
        let mut best = -VALUE_INFINITE;
        // 静止探索のfutilityの基準（ADR-0077）。王手中は定義しない
        let mut futility_base = -VALUE_INFINITE;
        if !in_check {
            let corr_value = self.correction_value(ply);
            // 1手詰め判定（ADR-0029。yaneuraou-search.cpp:4756-4784）。
            // 置換表にヒットしたときはすでに調べたはずなので、ミスのときだけ
            // 呼ぶ。王手中も呼ばない。評価関数の呼び出しより手前に置くので、
            // 詰みがあるときはevaluate()を省ける
            if tt_hit.is_none()
                && let Some(m) = crate::mate::mate_1ply(&self.pos)
            {
                // 次のノードで（指し手がなくなって）詰むという解釈
                let mate = mate_in(ply + 1);
                // 原典はmate_in()の値をvalue_to_tt()に通さず、そのまま書く
                // （yaneuraou-search.cpp:4777と、同じ処理の説明がある2911-2914）。
                // eval欄は評価関数を呼ぶ前なのでVALUE_NONEである
                self.shared.tt.store(
                    key,
                    m.to_move16(),
                    mate as i16,
                    VALUE_NONE as i16,
                    TT_DEPTH_QS,
                    Bound::Exact,
                    self.stack[ply + STACK_OFFSET].tt_pv,
                );
                return mate;
            }
            // TTのeval欄が空なら評価関数を呼ぶ（yaneuraou-search.cpp:4729-4731）
            raw_eval = if tt_eval != VALUE_NONE {
                tt_eval
            } else {
                self.eval_cached(key)
            };
            // 参照実装は静止探索でもstaticEvalをStackへ記録する
            // （yaneuraou-search.cpp:4729, 4790）。親のimprovingがこの値を読む
            let stand = self.to_corrected_with(raw_eval, corr_value);
            self.stack[ply + STACK_OFFSET].static_eval = stand;
            best = stand;
            // TTの値のほうがこの局面の見積りとして良ければ採用する
            // （yaneuraou-search.cpp:4743-4745）。詰みスコアは動かさない
            if tt_hit.is_some()
                && tt_value != VALUE_NONE
                && tt_value.abs() < VALUE_MATE_IN_MAX_PLY
                // 原典の `bound & (value > bestValue ? LOWER : UPPER)` をそのまま写す
                && if tt_value > best {
                    matches!(tt_bound, Bound::Lower | Bound::Exact)
                } else {
                    matches!(tt_bound, Bound::Upper | Bound::Exact)
                }
            {
                best = tt_value;
            }
            // stand pat（ADR-0024。yaneuraou-search.cpp:4813-4823）
            if best >= beta {
                // 決着スコアでなければ、返す値をβへ半分寄せる
                // （yaneuraou-search.cpp:4815-4817）。βを大きく超えた
                // stand patをそのまま返さず、見積りの甘さを削る
                if best.abs() < VALUE_MATE_IN_MAX_PLY {
                    best = (best + beta) / 2;
                }
                // TTにヒットしていなければ、探索を伴わない値として書き出す。
                // depthはDEPTH_UNSEARCHEDなのでTTカットには使われない
                if tt_hit.is_none() {
                    self.shared.tt.store(
                        key,
                        Move::NONE.to_move16(),
                        value_to_tt(best, ply),
                        raw_eval as i16,
                        TT_DEPTH_UNSEARCHED,
                        Bound::Lower,
                        false,
                    );
                }
                return best;
            }
            if best > alpha {
                alpha = best;
            }
            // 基準はTT値で上書きする前のstaticEvalである
            // （yaneuraou-search.cpp:4836）
            futility_base = stand + QS_FUTILITY_MARGIN;
        }
        // 1手前の移動先（取り返しをfutilityの対象から外す。ADR-0077）。
        // 余白の初期値はMove::NONEなので、ply 0でもNoneになる
        let prev_move = self.stack[ply + STACK_OFFSET - 1].current_move;
        let prev_sq = if prev_move != Move::NONE && !prev_move.is_special() {
            Some(prev_move.to())
        } else {
            None
        };

        // 参照実装は静止探索で取る手（歩成を含む）だけを生成する
        // （movepick.cpp:69）。静かな王手は生成しない
        // ply別の再利用バッファを借りる（ADR-0151の群B）
        let buf = std::mem::take(&mut self.node_bufs[ply].moves);
        let mut picker = MovePicker::new(&self.pos, tt_move, 0, ply, buf);
        let cont = self.cont_bases(ply);
        // バッファの返却を一本化するため、復帰点をこのブロックにまとめる
        let node_value = 'node: {
            let mut count = 0u32;
            let mut best_move = Move::NONE;
            while let Some(m) = picker.next(&self.pos, &self.hist, &cont) {
                if !self.pos.is_legal(m) {
                    continue;
                }
                count += 1;
                // capture_stage(m) は将棋版では単なるcapture(m)（position.h:1317-1320）
                let capture = !m.is_drop() && !self.pos.piece_on(m.to()).is_empty();

                // Step 6. 枝刈り（yaneuraou-search.cpp:4930-4991）。
                // bestValueが負け側の決着スコアの間は何も刈らない。詰みを逃れる
                // 手を探している最中だからである
                if best > VALUE_MATED_IN_MAX_PLY {
                    // futility（ADR-0077）: 王手をかけず取り返しでもない手を、
                    // 取る駒の価値を足してもalphaへ届かないなら捨てる
                    if futility_base > VALUE_MATED_IN_MAX_PLY
                        && Some(m.to()) != prev_sq
                        && !self.pos.gives_check(m)
                    {
                        if count > QS_MOVECOUNT_LIMIT {
                            continue;
                        }
                        let captured = self.pos.piece_on(m.to());
                        let gain = if captured.is_empty() {
                            VALUE_ZERO
                        } else {
                            himawari_core::piece_value(captured.piece_type())
                        };
                        // 捨てる前にbestを引き上げる（yaneuraou-search.cpp:4950-4954,
                        // 4965-4969）。fail-softの下限を正しく報告するためである。
                        // 検討モード（MultiPV>1）だけは抑える。ライン確定ごとに出力し、
                        // 確定後のソートを持たないので、窓に依存する値を入れると
                        // ライン間のスコア順序が崩れる（ADR-0077, 0109）
                        let raise = self.multi_pv == 1;
                        let futility_value = futility_base + gain;
                        if futility_value <= alpha {
                            if raise {
                                best = best.max(futility_value);
                            }
                            continue;
                        }
                        if !self.pos.see_ge(m, alpha - futility_base) {
                            if raise {
                                best = best.max(alpha.min(futility_base));
                            }
                            continue;
                        }
                    }

                    // 取る手でない手は一律に捨てる（yaneuraou-search.cpp:4975-4976）
                    if !capture {
                        continue;
                    }
                    // SEEが十分悪い手は探索しない（yaneuraou-search.cpp:4989-4990）。
                    // 無駄な王手ラッシュを抑える。歩損は許す下限である
                    if !self.pos.see_ge(m, QS_SEE_MARGIN) {
                        continue;
                    }
                }

                self.set_current_move(ply, m, capture);
                self.pos.do_move(m);
                self.evaluator.push(&self.pos);
                let value = -self.qsearch::<PV>(-beta, -alpha, ply + 1);
                self.evaluator.pop();
                self.pos.undo_move(m);
                if self.stopped() {
                    break 'node VALUE_ZERO;
                }
                if value > best {
                    best = value;
                    if value > alpha {
                        best_move = m;
                        alpha = value;
                        if value >= beta {
                            break;
                        }
                    }
                }
            }
            // Step 9. 詰みの確認（yaneuraou-search.cpp:5054-5070）。
            // 王手中に合法手が尽きたら詰み。参照実装は置換表へ書かずに返す。
            // 再訪問の確率が極めて低く、置換表を汚すだけだからである
            if in_check && count == 0 {
                break 'node mated_in(ply);
            }

            // 決着スコアでなければ、返す値をβへ半分寄せる
            // （yaneuraou-search.cpp:5093-5094）。stand patと同じ扱いである
            if best.abs() < VALUE_MATE_IN_MAX_PLY && best > beta {
                best = (best + beta) / 2;
            }

            // 置換表store（yaneuraou-search.cpp:5124-5126）。深さはDEPTH_QS固定。
            // 静止探索の結果は信用ならないのでBOUND_EXACTは書かない
            self.shared.tt.store(
                key,
                best_move.to_move16(),
                value_to_tt(best, ply),
                raw_eval as i16,
                TT_DEPTH_QS,
                if best >= beta {
                    Bound::Lower
                } else {
                    Bound::Upper
                },
                pv_hit,
            );
            best
        };

        // 借りたバッファを戻す
        self.node_bufs[ply].moves = picker.into_buf();
        node_value
    }

    /// correction historyを6要素まとめて更新する（ADR-0109のG1）。
    /// 出典はやねうら王の `update_correction_history()`
    /// （yaneuraou-search.cpp:748-771）。系統ごとに重みが違う。
    fn update_correction_history(&mut self, ply: usize, bonus: i32) {
        self.shared.hists.corr.update_all(&self.pos, bonus);
        let prev1 = self.stack[ply + STACK_OFFSET - 1].current_move;
        if !prev1.is_special() {
            let to = prev1.to();
            let pc = self.pos.piece_on(to);
            let base2 = self.stack[ply + STACK_OFFSET - 2].cont_corr_base;
            let base4 = self.stack[ply + STACK_OFFSET - 4].cont_corr_base;
            self.hist.corr_cont.update(base2, pc, to, bonus * 126 / 128);
            self.hist.corr_cont.update(base4, pc, to, bonus * 63 / 128);
        }
    }

    /// continuation historyへ段ごとの重みでbonusを配る（ADR-0109のG1）。
    /// 出典はやねうら王の `update_continuation_histories()`
    /// （yaneuraou-search.cpp:5384-5398）。王手中は2手前までで打ち切る。
    fn update_continuation_histories(&mut self, ply: usize, pc: Piece, to: Square, bonus: i32) {
        /// 何手前の面にどれだけ配るか（yaneuraou-search.cpp:5385-5386）。
        const CONTHIST_BONUSES: [(usize, i32); 6] =
            [(1, 1157), (2, 648), (3, 288), (4, 576), (5, 140), (6, 441)];
        let in_check = self.stack[ply + STACK_OFFSET].in_check;
        for (i, weight) in CONTHIST_BONUSES {
            if in_check && i > 2 {
                break;
            }
            let e = self.stack[ply + STACK_OFFSET - i];
            if !e.current_move.is_special() {
                let b = bonus * weight / 1024 + 88 * i32::from(i < 2);
                self.hist.cont.update(e.cont_base, pc, to, b);
            }
        }
    }

    /// 静かな手1つのhistoryを更新する（ADR-0109のG1）。
    /// 出典はやねうら王の `update_quiet_histories()`
    /// （yaneuraou-search.cpp:5408-5422）。
    fn update_quiet_histories(&mut self, ply: usize, m: Move, bonus: i32) {
        self.hist.main.update(self.pos.side_to_move(), m, bonus);
        if ply < LOW_PLY_HISTORY_SIZE {
            self.hist.low_ply.update(ply, m, bonus * 761 / 1024);
        }
        self.update_continuation_histories(ply, m.piece_after(), m.to(), bonus * 955 / 1024);
        let slot = self.shared.hists.pawn.slot(self.pos.pawn_key());
        let scaled = bonus * if bonus > 0 { 850 } else { 550 } / 1024;
        self.shared
            .hists
            .pawn
            .update(slot, m.piece_after(), m.to(), scaled);
    }

    /// 統計情報一式を更新する（ADR-0109のG1）。bestMoveが確定したノードの
    /// 終端で呼ぶ。出典はやねうら王の `update_all_stats()`
    /// （yaneuraou-search.cpp:5293-5367）。
    ///
    /// `quiets_searched` と `captures_searched` は、このノードで調べたが
    /// 最善にならなかった手を良い順に並べたもの。
    #[allow(clippy::too_many_arguments)]
    fn update_all_stats(
        &mut self,
        ply: usize,
        depth: u32,
        best_move: Move,
        tt_move: Move,
        prev_sq: Option<Square>,
        prior_capture: Piece,
        quiets_searched: &[Move],
        captures_searched: &[Move],
    ) {
        // bonus式（yaneuraou-search.cpp:5307-5309）。第3項は1手前の
        // statScore、つまりこのノードへ来た手の履歴の強さである
        let bonus = (128 * depth as i32 - 77).min(1529)
            + 353 * i32::from(best_move == tt_move)
            + self.stack[ply + STACK_OFFSET - 1].stat_score / 32;
        let malus = (882 * depth as i32 - 204).min(2122);

        let to = best_move.to();
        let captured = self.pos.piece_on(to);
        if best_move.is_drop() || captured.is_empty() {
            self.update_quiet_histories(ply, best_move, bonus * 806 / 1024);
            // 最善でなかった静かな手へmalusを配る。後ろの手ほど軽くする
            // （yaneuraou-search.cpp:5318-5326）
            let mut actual_malus = malus * 1113 / 1024;
            for &q in quiets_searched {
                actual_malus = actual_malus * 977 / 1024;
                self.update_quiet_histories(ply, q, -actual_malus);
            }
        } else {
            self.hist.capture.update(
                best_move.piece_after(),
                to,
                captured.piece_type(),
                bonus * 1286 / 1024,
            );
        }

        // 1手前が置換表の手でない早い静かな手で、それが反証されたときの
        // 追加ペナルティ（yaneuraou-search.cpp:5344-5345）
        if let Some(prev_sq) = prev_sq {
            let prev = self.stack[ply + STACK_OFFSET - 1];
            if prev.move_count == 1 + u32::from(prev.tt_hit) && prior_capture.is_empty() {
                let pc = self.pos.piece_on(prev_sq);
                self.update_continuation_histories(ply - 1, pc, prev_sq, -malus * 616 / 1024);
            }
        }

        // 最善でなかった取る手へmalusを配る（yaneuraou-search.cpp:5359-5364）
        for &c in captures_searched {
            let to = c.to();
            self.hist.capture.update(
                c.piece_after(),
                to,
                self.pos.piece_on(to).piece_type(),
                -malus * 1559 / 1024,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::Evaluator;
    use crate::timeman::TimeOptions;
    use himawari_core::Position;

    /// eval hashあり/なしで探索した (総ノード数, 最善手) を返す。
    fn search_nodes_best(sfen: &str, depth: u32, eval_hash: bool) -> (u64, Move) {
        let pos = Position::from_sfen(sfen).unwrap();
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            aborted_search: AtomicBool::new(false),
            ponder: AtomicBool::new(false),
            ponderhit_offset: AtomicI64::new(0),
            nodes: AtomicU64::new(0),
            increase_depth: AtomicBool::new(true),
            best_move_changes: AtomicU64::new(0),
            tt: Tt::new(16),
            eval_hash: if eval_hash {
                EvalHash::new()
            } else {
                EvalHash::disabled()
            },
            hists: Arc::new(SharedHistories::new(1)),
        });
        let limits = Limits {
            depth,
            ..Limits::default()
        };
        let tm = TimeManager::new(
            &limits,
            pos.side_to_move(),
            pos.game_ply(),
            &TimeOptions::default(),
        );
        let mut worker = Worker::new(
            pos,
            shared,
            limits,
            tm,
            0,
            1,
            Evaluator::material(),
            Histories::default(),
        );
        let result = worker.iterate(&mut |_| {});
        (worker.nodes, result.best)
    }

    /// 機能検証（ADR-0049）: eval hashは探索を変えないはずである。
    /// 偽ヒットを除けばノード数と最善手が有無で一致する。
    #[test]
    fn eval_hash_does_not_change_search() {
        // 中盤・序盤の複数局面で照合する
        for &(sfen, depth) in &[
            (himawari_core::SFEN_STARTPOS, 7),
            (
                "1n1gk2nl/1r4g2/1sppppspp/L5p2/1p5P1/2P6/1PSPPPPSP/7R1/1N1GKG1NL w BLPbp 24",
                6,
            ),
            ("4k4/9/9/5N3/9/9/9/9/4K4 b G 1", 5),
        ] {
            let (n_on, best_on) = search_nodes_best(sfen, depth, true);
            let (n_off, best_off) = search_nodes_best(sfen, depth, false);
            assert_eq!(
                n_on, n_off,
                "ノード数がeval hash有無で不一致: {sfen} (on={n_on}, off={n_off})"
            );
            assert_eq!(
                best_on.to_usi(),
                best_off.to_usi(),
                "最善手がeval hash有無で不一致: {sfen}"
            );
        }
    }
}
