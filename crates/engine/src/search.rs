//! 探索v1（ADR-0024, 0026）。
//!
//! fail-softのalpha-beta＋反復深化＋aspiration window。
//! PVは三角配列（Vecの連結）。詰みスコアはTT境界でply補正する。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use himawari_core::{
    Move, MoveList, Piece, PieceType, Position, Repetition, Square, generate_legal,
};

use crate::eval::Evaluator;
use crate::movepick::{
    ContinuationCorrectionHistory, ContinuationHistory, Histories, LOW_PLY_HISTORY_SIZE,
    MovePicker, PawnHistory,
};
use crate::timeman::{Limits, TimeManager};
use crate::tt::{Bound, EvalHash, Tt};
use crate::value::{
    MAX_PLY, VALUE_DRAW, VALUE_INFINITE, VALUE_MATE_IN_MAX_PLY, VALUE_MATED_IN_MAX_PLY, VALUE_NONE,
    VALUE_SUPERIOR, VALUE_ZERO, Value, mate_in, mated_in, value_from_tt, value_to_tt,
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

/// 置換表のdepth欄のゲタ（tt.cpp:45-66, 103-164）。参照実装も内部で
/// DEPTH_NONE（-3）分を下駄履きして符号なしで持つ。同じ表現を採る。
/// 深さの比較はすべて差なので、ゲタを履かせても置換方針は変わらない
const TT_DEPTH_OFFSET: i32 = 3;
/// 静止探索が書き出すdepth（types.h:405のDEPTH_QS = 0）。
const TT_DEPTH_QS: u8 = 3;
/// 探索を伴わない値を書き出すdepth（types.h:418のDEPTH_UNSEARCHED = -2）。
/// DEPTH_QSより小さいので、この値ではTTカットが起きない
const TT_DEPTH_UNSEARCHED: u8 = 1;

/// 思考時間の難易度スケール（ADR-0059）。optimumを3係数の積で伸縮させる。
/// 評価の下落は時間を伸ばし、最善手の安定とノード集中は縮める。
const FALLING_UNIT: f64 = 200.0;
const FALLING_MIN: f64 = 0.6;
const FALLING_MAX: f64 = 1.7;
const STABILITY_BASE: f64 = 1.5;
const STABILITY_STEP: f64 = 0.15;
const STABILITY_MIN: f64 = 0.75;
const EFFORT_LO: f64 = 0.75;
const EFFORT_HI: f64 = 1.0;
const EFFORT_SCALE_LO: f64 = 0.85;
const EFFORT_SCALE_HI: f64 = 0.70;
const SCALE_MIN_DEPTH: u32 = 8;

/// historyのbonus・malusを配る対象として覚えておく手数の上限
/// （yaneuraou-search.cpp:702のSEARCHEDLIST_CAPACITY）。
const SEARCHED_LIST_CAPACITY: u32 = 32;

/// この手数に達したら静かな手の生成をやめる（ADR-0109のG1）。
/// 出典はやねうら王の `(3 + depth * depth) / (2 - improving)`
/// （yaneuraou-search.cpp:3593）。
fn lmp_limit(depth: u32, improving: bool) -> u32 {
    (3 + depth * depth) / (2 - u32::from(improving))
}

/// LMRのリダクション表の要素数。深さと手数の両方でこの表を引くので、
/// 生成できる手数の上限（`MoveList` の608）に合わせる。
/// 参照実装も `std::array<int, MAX_MOVES>` である
/// （yaneuraou-search.h:582）。
const REDUCTIONS_LEN: usize = 608;

/// LMRのリダクション表（G2。yaneuraou-search.cpp:2168-2169）。
/// `2763 / 128 × ln(i)` を整数化した1次元表で、深さと手数の積を取る。
/// 積が1024倍の固定小数になるスケールはADR-0076で確認済み。
static REDUCTIONS: std::sync::OnceLock<[i32; REDUCTIONS_LEN]> = std::sync::OnceLock::new();

fn reductions(i: u32) -> i32 {
    let t = REDUCTIONS.get_or_init(|| {
        let mut t = [0i32; REDUCTIONS_LEN];
        for (i, r) in t.iter_mut().enumerate().skip(1) {
            *r = (2763.0 / 128.0 * (i as f64).ln()) as i32;
        }
        t
    });
    t[(i as usize).min(REDUCTIONS_LEN - 1)]
}

/// スレッド間の共有状態（ADR-0020）。
pub struct Shared {
    pub stop: AtomicBool,
    pub nodes: AtomicU64,
    pub tt: Tt,
    /// 評価値キャッシュ（ADR-0049）。全スレッド共有、new_gameでクリア。
    pub eval_hash: EvalHash,
}

impl Shared {
    pub fn new(hash_mb: usize) -> Shared {
        Shared {
            stop: AtomicBool::new(false),
            nodes: AtomicU64::new(0),
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
    /// このイテレーションでこの手の探索に費やしたノード数（ADR-0062）。
    /// メインワーカーのローカル値のみ。イテレーション開始時に0へ戻す
    pub nodes: u64,
}

pub struct SearchResult {
    pub best: Move,
    pub score: Value,
    /// 相手の予測応手（PVの2手目。なければNONE。ADR-0033）。
    pub ponder: Move,
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

pub struct Worker {
    pub pos: Position,
    pub evaluator: Evaluator,
    /// historyの一式（ADR-0109のG1）。対局を通じてスレッドが持ち回る。
    pub hist: Histories,
    /// plyごとの探索状態（ADR-0109）。添字は `ply + STACK_OFFSET` で引く。
    /// 前方の余白により、ply 0でも1手前・2手前を境界検査なしで読める。
    stack: Vec<StackEntry>,
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
    nodes: u64,
    shared: Arc<Shared>,
    tm: TimeManager,
    limits: Limits,
    max_moves_to_draw: u16,
    /// 検討モードのライン数（ADR-0032）。対局時は1。
    multi_pv: usize,
    root_moves: Vec<RootMove>,
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
        Worker {
            pos,
            evaluator,
            hist,
            stack: vec![
                StackEntry {
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
                MAX_PLY + 10
            ],
            sel_depth: 0,
            // 0除算を避ける番兵。search_rootが毎回入れ直す
            root_delta: 1,
            root_depth: 0,
            nmp_min_ply: 0,
            last_iteration_pv: Vec::new(),
            depth1_done: false,
            nodes: 0,
            shared,
            tm,
            limits,
            max_moves_to_draw,
            multi_pv: multi_pv.max(1),
            root_moves: Vec::new(),
        }
    }

    /// 深さ1を終えるまではstopを無視する。`iterate` は打ち切り時に
    /// `root_moves[0]` を返すが、root手は生成順に並んでいるため、深さ1の
    /// 途中で止まると探索していない手が出てしまう。深さ1は数msで終わる
    /// ので、待つ代償は小さい
    #[inline]
    fn stopped(&self) -> bool {
        self.depth1_done && self.shared.stop.load(Ordering::Relaxed)
    }

    /// 定期的な時間・ノード制限の検査。時間制限を持つのはメイン
    /// ワーカーだけ（ヘルパーはtmが無制限。ADR-0020, 0031）。
    /// あわせてローカルのノード数を共有カウンタへ流し込む。
    #[inline]
    fn check_limits(&self) {
        if self.nodes.is_multiple_of(2048) {
            self.shared.nodes.fetch_add(2048, Ordering::Relaxed);
            if self.tm.over_maximum() {
                self.shared.stop.store(true, Ordering::Relaxed);
            }
            if self.limits.nodes > 0 && self.nodes >= self.limits.nodes {
                self.shared.stop.store(true, Ordering::Relaxed);
            }
        }
    }

    #[inline]
    fn draw_value(&self) -> Value {
        // 千日手PVへの固着を防ぐ±1の揺らぎ（ADR-0026）
        VALUE_DRAW + 1 - (self.nodes & 2) as Value
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
        let scale = reductions(depth) * reductions(move_count);
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
        let (pcv, micv, bnpcv, wnpcv) = self.hist.corr.probe(&self.pos);
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

    /// aspirationのfail時に途中経過を報告する（ADR-0091）。
    /// fail lowではPVが空になりうるので、そのときは前の周のPVを使う。
    #[allow(clippy::too_many_arguments)]
    fn report_bound(
        &self,
        on_info: &mut dyn FnMut(SearchInfo),
        depth: u32,
        pv_idx: usize,
        score: Value,
        bound: ScoreBound,
        pv: &[Move],
    ) {
        let line: Vec<Move> = if pv.is_empty() {
            self.root_moves[pv_idx].pv.clone()
        } else {
            pv.to_vec()
        };
        if line.is_empty() {
            return;
        }
        on_info(SearchInfo::Bound(
            IterInfo {
                depth,
                seldepth: self.sel_depth.max(depth),
                multipv: if self.multi_pv > 1 { pv_idx + 1 } else { 0 },
                score,
                pv: line,
                nodes: self.shared.nodes.load(Ordering::Relaxed).max(self.nodes),
                elapsed_ms: self.tm.elapsed().as_millis() as u64,
                hashfull: self.shared.tt.hashfull(),
            },
            bound,
        ));
    }

    /// 反復深化。各イテレーション完了時にon_iterを呼ぶ。
    pub fn iterate(&mut self, on_info: &mut dyn FnMut(SearchInfo)) -> SearchResult {
        // 入玉宣言勝ち（ADR-0030）: 成立していれば探索せず宣言する
        if self.pos.can_declare_win() {
            return SearchResult {
                best: Move::WIN,
                score: mate_in(0),
                ponder: Move::NONE,
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
                nodes: 0,
            })
            .collect();
        if self.root_moves.is_empty() {
            return SearchResult {
                best: Move::RESIGN,
                score: mated_in(0),
                ponder: Move::NONE,
            };
        }
        let mut last_score = VALUE_ZERO;
        // 最後に出したinfoが未確定の窓外れ（lowerbound / upperbound）か。
        // 打ち切りでこのまま終わると、GUIやCSAブリッジは確定していない値を
        // その手のスコアとして記録する。実際にfloodgateで `4723++` という
        // 値が残り、直後に評価が8300も反転した
        let mut unresolved_bound = false;
        // 確定した最後のイテレーションの深さ。
        let mut completed_depth = 0u32;
        // 思考時間の難易度スケール用の状態（ADR-0059）
        let mut prev_best = Move::NONE;
        let mut prev_iter_score = VALUE_ZERO;
        let mut stable_iters: u32 = 0;
        let max_depth = if self.limits.depth > 0 {
            self.limits.depth
        } else {
            (MAX_PLY - 1) as u32
        };

        'deepening: for depth in 1..=max_depth {
            for rm in &mut self.root_moves {
                rm.prev_score = rm.score;
                // aspirationの再探索で同じ深さを複数回掘るため、
                // 深さの開始時に集計を戻す（ADR-0062）
                rm.nodes = 0;
            }
            // seldepthはイテレーションごとに測り直す（ADR-0086）
            self.sel_depth = 0;
            let lines = self.multi_pv.min(self.root_moves.len());
            // 直前ラインの出力スコア。頭打ちの基準に使う（ADR-0032）
            let mut prev_line_score = VALUE_INFINITE;
            for pv_idx in 0..lines {
                // ラインごとのaspiration。中心は前深さの自ラインのスコア
                let center = if pv_idx == 0 {
                    last_score
                } else {
                    self.root_moves[pv_idx].prev_score
                };
                let mut delta = 20;
                let (mut alpha, mut beta) = if depth >= 5 && center > -VALUE_INFINITE {
                    (center - delta, center + delta)
                } else {
                    (-VALUE_INFINITE, VALUE_INFINITE)
                };
                loop {
                    let (score, best_idx, pv) =
                        self.search_root(depth, alpha, beta, pv_idx, on_info);
                    if self.stopped() {
                        break 'deepening;
                    }
                    if score <= alpha {
                        // fail low: 実際の評価はこの値以下（ADR-0091）。
                        // 窓を広げて読み直す前に、途中経過として報告する
                        self.report_bound(on_info, depth, pv_idx, score, ScoreBound::Upper, &pv);
                        if pv_idx == 0 {
                            unresolved_bound = true;
                        }
                        beta = (alpha + beta) / 2;
                        alpha = (score - delta).max(-VALUE_INFINITE);
                        delta += delta / 2;
                    } else if score >= beta {
                        // fail high: 実際の評価はこの値以上
                        self.report_bound(on_info, depth, pv_idx, score, ScoreBound::Lower, &pv);
                        if pv_idx == 0 {
                            unresolved_bound = true;
                        }
                        beta = (score + delta).min(VALUE_INFINITE);
                        delta += delta / 2;
                    } else {
                        // 成功: 最善手をこのラインの先頭へ移して確定する
                        if best_idx != pv_idx {
                            let rm = self.root_moves.remove(best_idx);
                            self.root_moves.insert(pv_idx, rm);
                        }
                        self.root_moves[pv_idx].score = score;
                        self.root_moves[pv_idx].pv = pv.clone();
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
                        let line_pv = pv;
                        if pv_idx == 0 {
                            last_score = score;
                            unresolved_bound = false;
                            completed_depth = depth;
                        }
                        on_info(SearchInfo::Iteration(IterInfo {
                            depth,
                            seldepth: self.sel_depth.max(depth),
                            multipv: if self.multi_pv > 1 { pv_idx + 1 } else { 0 },
                            score: line_score,
                            pv: line_pv,
                            // 全ワーカー合算（単スレッドではローカル値と一致）
                            nodes: self.shared.nodes.load(Ordering::Relaxed).max(self.nodes),
                            elapsed_ms: self.tm.elapsed().as_millis() as u64,
                            hashfull: self.shared.tt.hashfull(),
                        }));
                        break;
                    }
                }
            }
            // 今回の反復のPVを覚える（yaneuraou-search.cpp:1846-1853）。
            // 次の反復のfollow_pv判定が読む。打ち切られた反復のPVは
            // 途中までしか探索していないので採らない
            if !self.shared.stop.load(Ordering::Relaxed) {
                self.last_iteration_pv.clone_from(&self.root_moves[0].pv);
            }
            // ここまで来れば深さ1は完走している。以降はstopに従う
            self.depth1_done = true;
            // 局面の難易度で思考時間をスケールする（ADR-0059）
            let cur_best = self.root_moves[0].mv;
            stable_iters = if cur_best == prev_best {
                stable_iters + 1
            } else {
                0
            };
            prev_best = cur_best;
            let scale = if self.multi_pv == 1 && depth >= SCALE_MIN_DEPTH {
                // 評価が下がっているほど伸ばす（読み抜けを警戒する）
                let drop = f64::from(prev_iter_score - last_score);
                let falling = (1.0 + drop / FALLING_UNIT).clamp(FALLING_MIN, FALLING_MAX);
                // 最善手が変わった直後は伸ばし、連続で不変なら縮める
                let stability = (STABILITY_BASE - STABILITY_STEP * f64::from(stable_iters))
                    .clamp(STABILITY_MIN, STABILITY_BASE);
                // 最善手にノードが集中しているほど縮める（ADR-0062）
                let total: u64 = self.root_moves.iter().map(|rm| rm.nodes).sum();
                let ratio = self.root_moves[0].nodes as f64 / total.max(1) as f64;
                let t = ((ratio - EFFORT_LO) / (EFFORT_HI - EFFORT_LO)).clamp(0.0, 1.0);
                let effort = EFFORT_SCALE_LO + (EFFORT_SCALE_HI - EFFORT_SCALE_LO) * t;
                falling * stability * effort
            } else {
                1.0
            };
            prev_iter_score = last_score;
            // 詰みが確定したら打ち切る（ADR-0088）。反復深化なので、より短い
            // 詰みがあれば浅い周で見つかっている。これ以上読んでも結論は
            // 変わらない。詰まされる側も同じで、より長く粘る手があれば
            // alpha-betaが既にそちらを選んでいる
            if self.multi_pv == 1 && last_score.abs() >= VALUE_MATE_IN_MAX_PLY {
                break;
            }
            if self.stopped() || self.tm.over_total(scale) {
                break;
            }
            if self.limits.nodes > 0 && self.nodes >= self.limits.nodes {
                break;
            }
        }
        // 未確定の窓外れで終わるなら、確定した最後の結果を出し直す。
        // これを出さないと、消費側の最後の1行が lowerbound / upperbound の
        // ままになり、指し手と食い違うスコアがその手の評価として残る
        if unresolved_bound && completed_depth > 0 && !self.root_moves[0].pv.is_empty() {
            on_info(SearchInfo::Iteration(IterInfo {
                depth: completed_depth,
                seldepth: self.sel_depth.max(completed_depth),
                multipv: if self.multi_pv > 1 { 1 } else { 0 },
                score: last_score,
                pv: self.root_moves[0].pv.clone(),
                nodes: self.shared.nodes.load(Ordering::Relaxed).max(self.nodes),
                elapsed_ms: self.tm.elapsed().as_millis() as u64,
                hashfull: self.shared.tt.hashfull(),
            }));
        }
        // check_limitsで2048刻みに流し込んだ分を除いた端数を合算する
        self.shared
            .nodes
            .fetch_add(self.nodes % 2048, Ordering::Relaxed);
        SearchResult {
            best: self.root_moves[0].mv,
            score: last_score,
            ponder: self.root_moves[0].pv.get(1).copied().unwrap_or(Move::NONE),
        }
    }

    /// root_moves[pv_idx..]を探索する（上位の確定済みラインは除外）。
    /// 戻り値は (スコア, 最善手のroot_moves添字, PV)。
    #[allow(clippy::too_many_arguments)]
    fn search_root(
        &mut self,
        depth: u32,
        mut alpha: Value,
        beta: Value,
        pv_idx: usize,
        on_info: &mut dyn FnMut(SearchInfo),
    ) -> (Value, usize, Vec<Move>) {
        // リダクションの窓幅項の基準（yaneuraou-search.cpp:1708）
        self.root_delta = beta - alpha;
        // singularの多段化のマージンが読む（yaneuraou-search.cpp:1550, 3779）。
        // aspirationの再探索でも同じ深さなので、ここで入れ直してよい
        self.root_depth = depth;
        let mut best = -VALUE_INFINITE;
        let mut best_idx = pv_idx;
        let mut best_pv: Vec<Move> = Vec::new();
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
                -self.search(-beta, -alpha, depth - 1, 1, m, &mut child_pv, true, false)
            } else {
                let v = -self.search(
                    -alpha - 1,
                    -alpha,
                    depth - 1,
                    1,
                    m,
                    &mut child_pv,
                    false,
                    true,
                );
                if v > alpha && !self.stopped() {
                    -self.search(-beta, -alpha, depth - 1, 1, m, &mut child_pv, true, false)
                } else {
                    v
                }
            };
            self.evaluator.pop();
            self.pos.undo_move(m);
            // 打ち切られた分もこの手の探索コストなので先に計上（ADR-0062）
            self.root_moves[i].nodes += self.nodes - nodes_before;
            if self.stopped() {
                return (best, best_idx, best_pv);
            }
            if value > best {
                best = value;
                if value > alpha {
                    alpha = value;
                    best_idx = i;
                    self.root_moves[i].score = value;
                    best_pv.clear();
                    best_pv.push(m);
                    best_pv.extend_from_slice(&child_pv);
                    if value >= beta {
                        // 参照実装はrootも同じ経路を通る
                        // （yaneuraou-search.cpp:4214）
                        self.stack[STACK_OFFSET].cutoff_cnt += 1;
                        break;
                    }
                }
            }
        }
        (best, best_idx, best_pv)
    }

    #[allow(clippy::too_many_arguments)]
    fn search(
        &mut self,
        mut alpha: Value,
        beta: Value,
        mut depth: u32,
        ply: usize,
        prev: Move,
        pv: &mut Vec<Move>,
        is_pv: bool,
        cut_node: bool,
    ) -> Value {
        // 参照実装の不変条件（ADR-0109のG0）。PVノードはcut_nodeにならない
        debug_assert!(!(is_pv && cut_node));
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

        // 千日手・優等局面（ADR-0026）
        match self.pos.repetition_state() {
            Repetition::Draw => return self.draw_value(),
            Repetition::Win => return mate_in(ply),
            Repetition::Lose => return mated_in(ply),
            Repetition::Superior => return VALUE_SUPERIOR,
            Repetition::Inferior => return -VALUE_SUPERIOR,
            Repetition::None => {}
        }
        if self.max_moves_to_draw > 0 && self.pos.game_ply() >= self.max_moves_to_draw {
            return self.draw_value();
        }
        // 入玉宣言勝ち（ADR-0030）。玉が敵陣外なら即falseで安い
        if self.pos.can_declare_win() {
            return mate_in(ply);
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
        let tt_hit = self.shared.tt.probe(key);
        // 1手先のノードがhistoryの更新条件で読む（yaneuraou-search.cpp:2623）
        self.stack[ply + STACK_OFFSET].tt_hit = tt_hit.is_some();
        // 置換表にPVとして記録された値か（yaneuraou-search.cpp:2657）。
        // 除外手つき探索は同じplyでsearchを呼び直すので、上書きしない
        if excluded == Move::NONE {
            self.stack[ply + STACK_OFFSET].tt_pv = is_pv || tt_hit.as_ref().is_some_and(|d| d.pv);
        }
        let mut tt_move = Move::NONE;
        let mut tt_value = VALUE_NONE;
        // 置換表のdepth欄はゲタを外して扱う。ヒットしないときは参照実装の
        // DEPTH_NONE（tt.cpp:445）に合わせて -3 とする
        let mut tt_depth = -TT_DEPTH_OFFSET;
        let mut tt_bound = Bound::None;
        if let Some(data) = &tt_hit {
            if let Some(m) = self.pos.to_move(data.mv)
                && self.pos.pseudo_legal(m)
            {
                tt_move = m;
            }
            tt_value = value_from_tt(data.value, ply);
            tt_depth = i32::from(data.depth) - TT_DEPTH_OFFSET;
            tt_bound = data.bound;
            // TTカット。除外手つき探索中はカットしない（probeは行い、eval再利用は可）
            if excluded == Move::NONE && !is_pv && tt_depth >= depth as i32 {
                let usable = match tt_bound {
                    Bound::Exact => true,
                    Bound::Lower => tt_value >= beta,
                    Bound::Upper => tt_value <= alpha,
                    Bound::None => false,
                };
                if usable {
                    // TTカットでもオーダリングを更新する
                    // （yaneuraou-search.cpp:2735-2748）
                    if tt_move != Move::NONE && tt_value >= beta {
                        let tt_capture =
                            !tt_move.is_drop() && !self.pos.piece_on(tt_move.to()).is_empty();
                        if !tt_capture {
                            self.update_quiet_histories(
                                ply,
                                tt_move,
                                (130 * depth as i32 - 71).min(1043),
                            );
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
                    return tt_value;
                }
            }
        }

        // 置換表の手が駒を取る手か（yaneuraou-search.cpp:2671）。
        // RFPの条件とリダクションの1項が読む
        let tt_capture = tt_move != Move::NONE
            && !tt_move.is_drop()
            && !self.pos.piece_on(tt_move.to()).is_empty();
        // PVでもcutでもないノード（yaneuraou-search.cpp:2251）。
        // 全手を調べる見込みなのでリダクションを強める。IIRの条件も読む
        let all_node = !(is_pv || cut_node);

        if depth == 0 {
            // 参照実装はノード種別を引き継ぐ（yaneuraou-search.cpp:2256）
            return self.qsearch(alpha, beta, ply, is_pv);
        }

        // 静的評価（ADR-0028, 0109のG4）。TTのevalを再利用する。
        // rawは補正前（TT保存用）、static_evalはcorrection history補正後（ADR-0046）。
        let raw_eval = if in_check {
            VALUE_NONE
        } else {
            match &tt_hit {
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
        if !in_check && excluded == Move::NONE && tt_hit.is_some() && tt_value != VALUE_NONE {
            let usable = if tt_value > eval {
                matches!(tt_bound, Bound::Lower | Bound::Exact)
            } else {
                matches!(tt_bound, Bound::Upper | Bound::Exact)
            };
            if usable {
                eval = tt_value;
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
            if tt_hit.is_none()
                && pc.piece_type() != PieceType::PAWN
                && !prev1.current_move.is_promote()
            {
                let slot = PawnHistory::slot(self.pos.pawn_key());
                self.hist.pawn.update(slot, pc, prev_sq, eval_diff * 12);
            }
        }

        // 2手前より静的評価が改善しているか（枝刈りの強弱に使う。
        // yaneuraou-search.cpp:3159）。王手中はfalse固定である。
        // 王手中のstatic_evalが2手前の写しなので、連続王手でも連鎖は切れない。
        // 余白の初期値VALUE_NONE（32602）を上回るstatic_evalは存在しないため、
        // ply < 2でも比較だけでfalseになる（参照実装も同じ性質に依存する）
        let mut improving = false;

        // 王手中はevalベースの枝刈りを一切行わない
        // （yaneuraou-search.cpp:3013-3020のgoto moves_loop）。
        // 静的評価が2手前の写しでしかないため、判断材料にできない
        if !in_check {
            improving = static_eval > self.stack[ply + STACK_OFFSET - 2].static_eval;
            // 相手の状況が悪化しているか（yaneuraou-search.cpp:3169）。
            // 普通は `static_eval == -(1手前のstatic_eval)` なので、これを
            // 上回るなら相手にとって評価が悪くなっている
            let opponent_worsening = static_eval > -self.stack[ply + STACK_OFFSET - 1].static_eval;

            // 1手前のリダクションに応じた残り深さの事後補正
            // （yaneuraou-search.cpp:3176-3179）。深く削って戻ってきた手が
            // 相手を悪くできていないなら1手足し、静的評価の和が閾値を超えて
            // いるなら1手引く
            if prior_reduction >= 3 && !opponent_worsening {
                depth += 1;
            }
            if prior_reduction >= 2
                && depth >= 2
                && static_eval + self.stack[ply + STACK_OFFSET - 1].static_eval > 173
            {
                depth -= 1;
            }

            // razoring（ADR-0057, 0109のG4。yaneuraou-search.cpp:3191-3192）。
            // 評価がalphaを大きく下回るなら通常探索をやめ、qsearchの値を返す。
            // PVノードでないことが唯一の前提で、深さの上限はない
            if !is_pv && eval < alpha - RAZOR_BASE - RAZOR_DEPTH_COEF * (depth * depth) as Value {
                // razoringは非PVノード限定なので常にNonPV（yaneuraou-search.cpp:3192）
                return self.qsearch(alpha, beta, ply, false);
            }

            // 子ノードのfutility（RFP。yaneuraou-search.cpp:3217-3227）。
            // 残り深さで評価が動きうる幅を見積り、それを引いてもβを超えるなら
            // 刈る。TTにヒットしていないノードは見積りを狭める
            let futility_mult =
                RFP_MULT - RFP_NO_TT_HIT * i32::from(!self.stack[ply + STACK_OFFSET].tt_hit);
            let futility_margin = futility_mult * depth as i32
                - (RFP_IMPROVING * i32::from(improving)
                    + RFP_OPP_WORSENING * i32::from(opponent_worsening))
                    * futility_mult
                    / 1024
                + corr_value.abs() / RFP_CORR_DIVISOR;
            if !self.stack[ply + STACK_OFFSET].tt_pv
                && depth < RFP_MAX_DEPTH
                && eval >= beta
                && eval - futility_margin >= beta
                && (tt_move == Move::NONE || tt_capture)
                && beta > VALUE_MATED_IN_MAX_PLY
                && eval < VALUE_MATE_IN_MAX_PLY
            {
                // 静的評価そのものではなく、βへ寄せた値を返す
                return (2 * beta + eval) / 3;
            }

            // NMP（ADR-0028, 0109のG4。yaneuraou-search.cpp:3236-3301）。
            // 手番を渡して浅く探索し、それでもβ以上なら刈る。cutNode限定で、
            // 深さの下限はない。評価の閾値はβから残り深さとimprovingで割り引く。
            // 除外手つき探索中はスキップ（ADR-0050）
            if cut_node
                && static_eval
                    >= beta
                        - NMP_EVAL_DEPTH * depth as Value
                        - NMP_EVAL_IMPROVING * Value::from(improving)
                        + NMP_EVAL_BASE
                && excluded == Move::NONE
                && ply >= self.nmp_min_ply
                && beta > VALUE_MATED_IN_MAX_PLY
            {
                // 連続してnull moveは指さない（yaneuraou-search.cpp:3247）。
                // null moveの子はcut_node = falseなのでここへ来ない
                debug_assert!(prev != Move::NULL);
                let r = NMP_BASE_REDUCTION + depth / NMP_DEPTH_DIVISOR;
                let mut null_pv = Vec::new();
                // null moveは王手でも駒取りでもないので番兵の面を指す
                // （yaneuraou-search.cpp:3254-3256）
                let e = &mut self.stack[ply + STACK_OFFSET];
                e.current_move = Move::NULL;
                e.cont_base = ContinuationHistory::SENTINEL;
                e.cont_corr_base = 0;
                self.pos.do_null_move();
                self.evaluator.push(&self.pos);
                let v = -self.search(
                    -beta,
                    -beta + 1,
                    depth.saturating_sub(r),
                    ply + 1,
                    Move::NULL,
                    &mut null_pv,
                    false,
                    // NMPの子はcut_node = false（ADR-0109のG0）
                    false,
                );
                self.evaluator.pop();
                self.pos.undo_null_move();
                if self.stopped() {
                    return VALUE_ZERO;
                }
                // パス由来の詰みスコアは信用しない。刈らずに読み進める
                if v >= beta && v < VALUE_MATE_IN_MAX_PLY {
                    // 深いところでは同じ深さの検証探索で裏を取る
                    // （yaneuraou-search.cpp:3277-3301）。zugzwangでの誤りを
                    // 減らす。検証探索の中ではnmpMinPlyまでNMPを止める
                    if self.nmp_min_ply != 0 || depth < NMP_VERIFY_MIN_DEPTH {
                        return v;
                    }
                    self.nmp_min_ply = ply + 3 * (depth - r) as usize / 4;
                    let mut verify_pv = Vec::new();
                    let vv = self.search(
                        beta - 1,
                        beta,
                        depth - r,
                        ply,
                        prev,
                        &mut verify_pv,
                        false,
                        false,
                    );
                    self.nmp_min_ply = 0;
                    if self.stopped() {
                        return VALUE_ZERO;
                    }
                    if vv >= beta {
                        return v;
                    }
                }
            }

            // NMPの後にβで再計算する（yaneuraou-search.cpp:3306）。
            // 静的評価がβ以上なら、2手前と比べていなくても改善扱いにする
            improving |= static_eval >= beta;

            // IIR（ADR-0028, 0109のG4。yaneuraou-search.cpp:3319-3320）。
            // TTに手がないノードは良い順序を作れないので1浅く読み、再訪時に
            // TT手付きで読み直す。前回PVの上と、全手を読むallNodeでは行わない。
            // 1手前を深く削って来たノードも対象から外す
            if !follow_pv
                && !all_node
                && depth >= IIR_MIN_DEPTH
                && tt_move == Move::NONE
                && prior_reduction <= IIR_MAX_PRIOR_REDUCTION
            {
                depth -= 1;
            }

            // ProbCut（ADR-0051, 0109のG4。yaneuraou-search.cpp:3357-3424）。
            // betaを大きく超えそうなノードでは、浅い確認探索で「十分良い取る手が
            // 1つある」ことを示せれば高深度の全探索を省いてカットする。
            // 閾値はimprovingで動き、MovePickerがSEEでこの閾値を満たす取る手
            // だけをcapture history込みの順序で返す
            let probcut_beta = beta + PROBCUT_MARGIN - PROBCUT_IMPROVING * Value::from(improving);
            if depth >= PROBCUT_MIN_DEPTH
                && beta.abs() < VALUE_MATE_IN_MAX_PLY
                // 置換表の値がprobcut_beta未満と分かっているなら試さない
                && !(tt_value != VALUE_NONE && tt_value < probcut_beta)
            {
                let probcut_depth = depth as i32 - PROBCUT_DEPTH_REDUCTION;
                let mut picker =
                    MovePicker::new_probcut(&self.pos, tt_move, probcut_beta - static_eval);
                let cont = self.cont_bases(ply);
                while let Some(m) = picker.next(&self.pos, &self.hist, &cont) {
                    // 除外手はsingular検証探索中のTT手（ADR-0050）
                    if m == excluded || !self.pos.is_legal(m) {
                        continue;
                    }
                    self.set_current_move(ply, m, !self.pos.piece_on(m.to()).is_empty());
                    self.pos.do_move(m);
                    self.evaluator.push(&self.pos);
                    // まずqsearchで確認（窓は (-probcut_beta, -probcut_beta+1)）
                    let mut v = -self.qsearch(-probcut_beta, -probcut_beta + 1, ply + 1, false);
                    // 通ったら同じ窓で通常探索 depth-4 を確認する
                    if v >= probcut_beta && probcut_depth > 0 {
                        let mut child_pv = Vec::new();
                        v = -self.search(
                            -probcut_beta,
                            -probcut_beta + 1,
                            probcut_depth as u32,
                            ply + 1,
                            m,
                            &mut child_pv,
                            false,
                            // ProbCutの子はcut_nodeを反転する（ADR-0109のG0）
                            !cut_node,
                        );
                    }
                    self.evaluator.pop();
                    self.pos.undo_move(m);
                    if self.stopped() {
                        return VALUE_ZERO;
                    }
                    if v >= probcut_beta {
                        // fail-soft。TTにlower bound・depth-3で保存する
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
                            return v - (probcut_beta - beta);
                        }
                    }
                }
            }
        }

        // 置換表の下界による簡易ProbCut（ADR-0078）。探索を伴わない。
        // 除外手つき探索中はスキップする（ADR-0050）
        let tt_probcut_beta = beta + TT_PROBCUT_MARGIN;
        if excluded == Move::NONE
            && matches!(tt_bound, Bound::Lower | Bound::Exact)
            && tt_depth >= depth.saturating_sub(TT_PROBCUT_DEPTH_SLACK) as i32
            && tt_value >= tt_probcut_beta
            && beta.abs() < VALUE_MATE_IN_MAX_PLY
            && tt_value.abs() < VALUE_MATE_IN_MAX_PLY
        {
            return tt_probcut_beta;
        }

        // singular extension（ADR-0050, 0109のG5。yaneuraou-search.cpp:3745-3782）。
        // TT手を除外した検証探索がsingular_betaを下回れば、TT手だけが傑出して
        // いると見て延長する。参照実装はムーブループの中でTT手に当たったときに
        // 判定するが、対象はTT手だけで、TT手はMovePickerが最初に返す。ループの
        // 手前で1回求めても同じである（ムーブループの枝刈りは第1手には効かない）
        //
        // 延長を積む前の深さ。MovePickerのオーダリングの尺度
        // （yaneuraou-search.cpp:3453）と、TT手のnewDepthの基準
        // （yaneuraou-search.cpp:3556）がこの値を読む。参照実装はどちらも
        // depthを増やす前に決まるので、singularでdepthが増えても動かない
        let depth_pre_singular = depth as i32;
        // TT手に与える延長。Noneは判定に入らなかったことを表す
        let mut singular_ext: Option<i32> = None;
        if excluded == Move::NONE
            && ply > 0
            && tt_move != Move::NONE
            // ttPvノードでは1手深いところから判定する
            && depth >= SINGULAR_MIN_DEPTH + u32::from(self.stack[ply + STACK_OFFSET].tt_pv)
            && tt_bound != Bound::Upper
            && tt_bound != Bound::None
            && tt_depth >= depth.saturating_sub(3) as i32
            && tt_value.abs() < VALUE_MATE_IN_MAX_PLY
            // 往復手は検証探索にかけない（G5。yaneuraou-search.cpp:3749）
            && !self.is_shuffling(tt_move, ply)
            && self.pos.is_legal(tt_move)
        {
            let singular_beta = tt_value
                - (SINGULAR_MARGIN
                    + SINGULAR_MARGIN_TTPV
                        * Value::from(self.stack[ply + STACK_OFFSET].tt_pv && !is_pv))
                    * depth as Value
                    / SINGULAR_MARGIN_DIV;
            // 検証探索の深さは延長前のnewDepth（= depth - 1）の半分
            // （yaneuraou-search.cpp:3758）
            let singular_depth = (depth - 1) / 2;
            self.stack[ply + STACK_OFFSET].excluded_move = tt_move;
            let mut verify_pv = Vec::new();
            let v = self.search(
                singular_beta - 1,
                singular_beta,
                singular_depth,
                ply,
                prev,
                &mut verify_pv,
                false,
                // 検証探索はcut_nodeを引き継ぐ（ADR-0109のG0）
                cut_node,
            );
            self.stack[ply + STACK_OFFSET].excluded_move = Move::NONE;
            // 検証探索の再帰でstatic_evalが同値で上書きされる。念のため戻す
            self.stack[ply + STACK_OFFSET].static_eval = static_eval;
            if self.stopped() {
                return VALUE_ZERO;
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
                depth += 1;
            } else if v >= beta && v.abs() < VALUE_MATE_IN_MAX_PLY {
                // multi-cut（yaneuraou-search.cpp:3817-3821）。TT手を除いた
                // 浅い探索でもβを超えたので、このノードは「1手だけ傑出」では
                // なく複数の手がfail highすると見て、部分木をまとめて刈る。
                // 返す値はsoftbound（真の値がこれ以上と分かっている値）である
                self.hist
                    .tt_move
                    .update((-424 - 107 * depth as i32).max(-3375));
                return v;
            } else if tt_value >= beta {
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

        let mut picker = MovePicker::new(&self.pos, tt_move, depth_pre_singular, ply);
        // continuation historyの面（1手前から6手前まで。ADR-0109のG1）
        let cont = self.cont_bases(ply);
        let mut best = -VALUE_INFINITE;
        let mut best_move = Move::NONE;
        let mut best_move_is_capture = false;
        let mut count = 0u32;
        // 最善にならなかった手を良い順に覚える（yaneuraou-search.cpp:2343-2344）
        let mut quiets_searched: Vec<Move> = Vec::new();
        let mut captures_searched: Vec<Move> = Vec::new();
        let mut child_pv = Vec::new();

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
                Some(e) if m == tt_move => (e, depth_pre_singular),
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
            // Step 14: 浅い深さでの枝刈り（yaneuraou-search.cpp:3586-3698）。
            // 前提条件は「rootでない」「bestValueが敗勢でない」の2つだけで、
            // search()は常にrootでない。bestは1手目を読み終えるまで
            // -VALUE_INFINITEなので、第1手はこのブロックに入らない
            if best > VALUE_MATED_IN_MAX_PLY {
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
                            continue;
                        }
                    }

                    // 取る手・王手する手のSEE枝刈り
                    // （yaneuraou-search.cpp:3634-3641）。許す損の額が
                    // capture historyで動く。alphaが負のときは刈らない
                    let margin =
                        (SEE_CAPTURE_COEF * depth as i32 + capt_hist * SEE_CAPT_HIST / 1024).max(0);
                    if alpha >= VALUE_DRAW && !self.pos.see_ge(m, -margin) {
                        continue;
                    }
                } else if !follow_pv || !is_pv {
                    // 前回の反復深化のPV上にいるPVノードでは、静かな手の
                    // 枝刈りを一切かけない（yaneuraou-search.cpp:3644）。
                    // 前回のPVを浅い枝刈りで壊さないための仕掛けである
                    //
                    // 静かな手の履歴（yaneuraou-search.cpp:3646-3648）。
                    // 1手前・2手前のcontinuation historyとpawn historyの和
                    let to = m.to();
                    let pc = m.piece_after();
                    let pawn_slot = PawnHistory::slot(self.pos.pawn_key());
                    let mut history = self.hist.cont.get(cont[0], pc, to)
                        + self.hist.cont.get(cont[1], pc, to)
                        + self.hist.pawn.get(pawn_slot, pc, to);

                    // continuation historyによる枝刈り
                    // （yaneuraou-search.cpp:3650-3651）。履歴が極端に
                    // 悪い手は読まない
                    if history < -CONT_HIST_PRUNE_COEF * depth as i32 {
                        continue;
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
                        if best <= futility_value
                            && best.abs() < VALUE_MATE_IN_MAX_PLY
                            && futility_value < VALUE_MATE_IN_MAX_PLY
                        {
                            best = futility_value;
                        }
                        continue;
                    }

                    // 負のSEEを持つ手の枝刈り（yaneuraou-search.cpp:3691-3698）。
                    // 参照実装はここで0止めする
                    let lmr_depth = lmr_depth.max(0);
                    if !self.pos.see_ge(m, -SEE_QUIET_COEF * lmr_depth * lmr_depth) {
                        continue;
                    }
                }
            }

            // リダクションの加減算（yaneuraou-search.cpp:3879-3941）。
            // 参照実装はdo_moveの後に置くが、読む材料は進める前の局面で
            // 決まるのでここでまとめる
            //
            // 項2: ttPvノードは大きく戻す。TTの値がalphaを超える、TTの
            // 深さが足りている、といった手掛かりがあるほど戻す
            if self.stack[ply + STACK_OFFSET].tt_pv {
                r -= 2819
                    + i32::from(is_pv) * 973
                    + i32::from(tt_value > alpha) * 905
                    + i32::from(tt_depth >= depth as i32) * (935 + i32::from(cut_node) * 959);
            }
            // 項3: 他の調整を補正する基準オフセット
            r += 691;
            // 項4: 手数が進むほど戻す
            r -= count as i32 * 65;
            // 項5: correction historyの補正が大きい局面は戻す
            r -= corr_value.abs() / 25600;
            // 項6: cutNodeは削る。TT手がなければさらに削る
            if cut_node {
                r += 3611 + 985 * i32::from(tt_move == Move::NONE);
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
            if m == tt_move {
                r -= 2239;
            }

            // その手の履歴の強さを控える（yaneuraou-search.cpp:3924-3932）。
            // 子のhistory更新量にも効くのでdo_moveの前に測る
            let stat_score = self.stat_score(m, &cont);
            self.stack[ply + STACK_OFFSET].stat_score = stat_score;
            // 項10: 履歴の良い手は戻し、悪い手は削る
            r -= stat_score * 428 / 4096;
            // 項11: allNodeでは全体を割り増す
            if all_node {
                r += r * 273 / (256 * depth as i32 + 260);
            }

            self.set_current_move(ply, m, is_capture);
            self.pos.do_move(m);
            self.evaluator.push(&self.pos);

            let mut value = -VALUE_INFINITE;
            if depth >= 2 && count > 1 {
                // LMR（yaneuraou-search.cpp:3954-4010）。参照実装の発動条件は
                // 深さと手数だけで、取る手も王手する手も対象にする。
                // リダクションが負なら `new_depth + 2` まで深く読む
                let d = (new_depth - r / 1024).min(new_depth + 2).max(1) + i32::from(is_pv);
                self.stack[ply + STACK_OFFSET].reduction = new_depth - d;
                value = -self.search(
                    -alpha - 1,
                    -alpha,
                    d as u32,
                    ply + 1,
                    m,
                    &mut child_pv,
                    false,
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
                        value = -self.search(
                            -alpha - 1,
                            -alpha,
                            new_depth.max(0) as u32,
                            ply + 1,
                            m,
                            &mut child_pv,
                            false,
                            !cut_node,
                        );
                    }
                    // LMR後のcontinuation history更新
                    // （yaneuraou-search.cpp:4008）
                    self.update_continuation_histories(ply, m.piece_after(), m.to(), 1426);
                }
            } else if !is_pv || count > 1 {
                // LMRを省いたときの調整（yaneuraou-search.cpp:4017-4030）。
                // 項12: TT手がなければ削る。削る量が大きければ深さを落とす
                if tt_move == Move::NONE {
                    r += 1057;
                }
                let d = new_depth - i32::from(r > 4628) - i32::from(r > 5772 && new_depth > 2);
                value = -self.search(
                    -alpha - 1,
                    -alpha,
                    d.max(0) as u32,
                    ply + 1,
                    m,
                    &mut child_pv,
                    false,
                    !cut_node,
                );
            }
            // PVノードは第1手とfail highの後だけ全窓で読み直す
            // （yaneuraou-search.cpp:4043-4061）
            if is_pv && (count == 1 || value > alpha) && !self.stopped() {
                // 静止探索へ直行する手前で、TT手だけは1手残す
                // （yaneuraou-search.cpp:4053-4057）。負の延長でnew_depthが
                // 0以下になったTT手をqsearchへ落とすと、詰みの発見が鈍る
                if m == tt_move
                    && ((tt_value != VALUE_NONE
                        && tt_value.abs() >= VALUE_MATE_IN_MAX_PLY
                        && tt_depth > 0)
                        || tt_depth > 1)
                {
                    new_depth = new_depth.max(1);
                }
                value = -self.search(
                    -beta,
                    -alpha,
                    new_depth.max(0) as u32,
                    ply + 1,
                    m,
                    &mut child_pv,
                    true,
                    false,
                );
            }
            self.evaluator.pop();
            self.pos.undo_move(m);
            if self.stopped() {
                return VALUE_ZERO;
            }

            if value > best {
                best = value;
                if value > alpha {
                    best_move = m;
                    best_move_is_capture = is_capture;
                    if is_pv {
                        pv.clear();
                        pv.push(m);
                        pv.extend_from_slice(&child_pv);
                    }
                    if value >= beta {
                        // 次plyのfail highの多さをリダクションへ渡す
                        // （yaneuraou-search.cpp:4214）。2手以上延長した手の
                        // カットは数えない（G5で延長が最大+3になった）
                        if extension < 2 || is_pv {
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
                return alpha;
            }
            // 合法手なし = 詰み（将棋はステイルメイトも負け）
            return mated_in(ply);
        }

        // 指し手の統計を更新する（yaneuraou-search.cpp:4299-4356）。
        // βカットしていなくても、alphaを更新した手があれば更新する
        if best_move != Move::NONE {
            self.update_all_stats(
                ply,
                depth,
                best_move,
                tt_move,
                prev_sq,
                prior_capture,
                &quiets_searched,
                &captures_searched,
            );
            // 非PVノードのbestMove確定時の更新（yaneuraou-search.cpp:4308）。
            // もう1か所、multi-cutが減点する（yaneuraou-search.cpp:3819）
            if !is_pv {
                self.hist
                    .tt_move
                    .update(if best_move == tt_move { 805 } else { -787 });
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
                let slot = PawnHistory::slot(self.pos.pawn_key());
                self.hist
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
            } else if is_pv && best_move != Move::NONE {
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
        best
    }

    /// 静止探索（ADR-0024, 0109のG6）。出典はやねうら王の `qsearch()`
    /// （yaneuraou-search.cpp:4441-5145）。参照実装は `qsearch<PV>` と
    /// `qsearch<NonPV>` をテンプレートで分けるので、`is_pv` で受ける。
    fn qsearch(&mut self, mut alpha: Value, beta: Value, ply: usize, is_pv: bool) -> Value {
        if self.stopped() {
            return VALUE_ZERO;
        }
        self.sel_depth = self.sel_depth.max(ply as u32);
        self.nodes += 1;
        self.check_limits();
        if ply >= MAX_PLY {
            return self.evaluator.evaluate(&self.pos);
        }
        match self.pos.repetition_state() {
            Repetition::Draw => return self.draw_value(),
            Repetition::Win => return mate_in(ply),
            Repetition::Lose => return mated_in(ply),
            Repetition::Superior => return VALUE_SUPERIOR,
            Repetition::Inferior => return -VALUE_SUPERIOR,
            Repetition::None => {}
        }
        // 最大手数の到達（yaneuraou-search.cpp:4620）。通常探索と同じ扱いにする
        if self.max_moves_to_draw > 0 && self.pos.game_ply() >= self.max_moves_to_draw {
            return self.draw_value();
        }
        // 入玉宣言勝ち（ADR-0030）。ADR-0109で唯一の例外として位置を残す
        if self.pos.can_declare_win() {
            return mate_in(ply);
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
            if !is_pv
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
        let mut picker = MovePicker::new(&self.pos, tt_move, 0, ply);
        let cont = self.cont_bases(ply);
        let mut count = 0u32;
        let mut best_move = Move::NONE;
        while let Some(m) = picker.next(&self.pos, &self.hist, &cont) {
            if !self.pos.is_legal(m) {
                continue;
            }
            count += 1;

            // futility（ADR-0077）: 王手をかけず取り返しでもない手を、
            // 取る駒の価値を足してもalphaへ届かないなら捨てる。
            // fail-softを保つため、捨てる前にbestを引き上げる
            if !in_check
                && futility_base > VALUE_MATED_IN_MAX_PLY
                && !self.pos.gives_check(m)
                && Some(m.to()) != prev_sq
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

            self.set_current_move(
                ply,
                m,
                !m.is_drop() && !self.pos.piece_on(m.to()).is_empty(),
            );
            self.pos.do_move(m);
            self.evaluator.push(&self.pos);
            let value = -self.qsearch(-beta, -alpha, ply + 1, is_pv);
            self.evaluator.pop();
            self.pos.undo_move(m);
            if self.stopped() {
                return VALUE_ZERO;
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
            return mated_in(ply);
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
    }

    /// correction historyを6要素まとめて更新する（ADR-0109のG1）。
    /// 出典はやねうら王の `update_correction_history()`
    /// （yaneuraou-search.cpp:748-771）。系統ごとに重みが違う。
    fn update_correction_history(&mut self, ply: usize, bonus: i32) {
        self.hist.corr.update_all(&self.pos, bonus);
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
        let slot = PawnHistory::slot(self.pos.pawn_key());
        let scaled = bonus * if bonus > 0 { 850 } else { 550 } / 1024;
        self.hist.pawn.update(slot, m.piece_after(), m.to(), scaled);
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
    use himawari_core::Position;

    /// eval hashあり/なしで探索した (総ノード数, 最善手) を返す。
    fn search_nodes_best(sfen: &str, depth: u32, eval_hash: bool) -> (u64, Move) {
        let pos = Position::from_sfen(sfen).unwrap();
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            nodes: AtomicU64::new(0),
            tt: Tt::new(16),
            eval_hash: if eval_hash {
                EvalHash::new()
            } else {
                EvalHash::disabled()
            },
        });
        let limits = Limits {
            depth,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, pos.side_to_move(), pos.game_ply(), 120, 1120);
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
