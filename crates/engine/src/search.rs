//! 探索v1（ADR-0024, 0026）。
//!
//! fail-softのalpha-beta＋反復深化＋aspiration window。
//! PVは三角配列（Vecの連結）。詰みスコアはTT境界でply補正する。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use himawari_core::{
    GenType, Move, MoveList, Piece, PieceType, Position, Repetition, Square, generate,
    generate_legal,
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

/// NMPの最小深さ。
const NMP_MIN_DEPTH: u32 = 3;
/// NMPのリダクション: 3 + depth / 4。
const NMP_BASE_REDUCTION: u32 = 3;
/// LMRの最小深さと最小手数（この手数以降の静かな手を浅く読む）。
const LMR_MIN_DEPTH: u32 = 3;
const LMR_MIN_COUNT: u32 = 3;
/// reverse futilityの最大深さとdepthあたりのマージン。
const RFP_MAX_DEPTH: u32 = 6;
const RFP_MARGIN: Value = 120;
/// 子ノードfutilityの最大深さとマージン（基本 + depth比例）。
const FUTILITY_MAX_DEPTH: u32 = 6;
const FUTILITY_BASE: Value = 200;
const FUTILITY_MARGIN: Value = 120;
/// 「今読んでいる手」をinfoで出し始める経過時間（ADR-0086）。
/// USIの慣例に合わせ、短い探索では出さない
const CURRMOVE_MIN_MS: u64 = 3000;
/// IIR: TTに手がないノードを1浅く読む最小深さ。
const IIR_MIN_DEPTH: u32 = 4;
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
/// SEEベースの枝刈り（ADR-0090）。移動先での駒の取り合いを静的に解き、
/// この額より損をする手を捨てる。出典はやねうら王の
/// `-25*lmrDepth^2`（静かな手）と `-167*depth`（取る手、captHist項は除く）。
/// SEEの駒価値は歩=90でやねうら王と同系列のため絶対値のまま使える
/// （ADR-0074）。閾値が負なので「多少の駒損は許し、大きな損だけ刈る」
const SEE_QUIET_COEF: i32 = 25;
const SEE_CAPTURE_COEF: i32 = 167;
/// razoringの最大深さとマージン（ADR-0057）。
const RAZOR_MAX_DEPTH: u32 = 3;
const RAZOR_MARGIN: Value = 300;
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
    /// 子が `priorReduction` として読む側はG4で入れる
    #[allow(dead_code)]
    reduction: i32,
    /// このplyで今調べている手の履歴の強さ（G2。yaneuraou-search.cpp:3924-3932）。
    /// リダクションの減算と、子でのhistory更新量の2方向へ効く
    stat_score: i32,
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
                };
                MAX_PLY + 10
            ],
            sel_depth: 0,
            // 0除算を避ける番兵。search_rootが毎回入れ直す
            root_delta: 1,
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
        depth: u32,
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
        let mut tt_depth = 0u32;
        let mut tt_bound = Bound::None;
        if let Some(data) = &tt_hit {
            if let Some(m) = self.pos.to_move(data.mv)
                && self.pos.pseudo_legal(m)
            {
                tt_move = m;
            }
            tt_value = value_from_tt(data.value, ply);
            tt_depth = u32::from(data.depth);
            tt_bound = data.bound;
            // TTカット。除外手つき探索中はカットしない（probeは行い、eval再利用は可）
            if excluded == Move::NONE && !is_pv && tt_depth >= depth {
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

        if depth == 0 {
            return self.qsearch(alpha, beta, ply, 0);
        }

        // IIR（ADR-0028）: TTに手がないノードは良い順序を作れないので
        // 1浅く読み、再訪時にTT手付きで読み直す
        let depth = if depth >= IIR_MIN_DEPTH && tt_move == Move::NONE {
            depth - 1
        } else {
            depth
        };

        // 静的評価（ADR-0028）。王手中はVALUE_NONE。TTのevalを再利用する。
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
        let static_eval = if in_check {
            VALUE_NONE
        } else {
            self.to_corrected_with(raw_eval, corr_value)
        };

        self.stack[ply + STACK_OFFSET].static_eval = static_eval;
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

        // 2手前より静的評価が改善しているか（枝刈りの強弱に使う）。
        // 余白の初期値はVALUE_NONEなので、ply < 2でもこの検査でfalseになる
        let prev2_eval = self.stack[ply + STACK_OFFSET - 2].static_eval;
        let improving = !in_check && prev2_eval != VALUE_NONE && static_eval > prev2_eval;

        // reverse futility（ADR-0028）: 静的評価がβを大きく超えるなら刈る。
        // 除外手つき探索中はスキップ（ADR-0050）
        if excluded == Move::NONE
            && !is_pv
            && !in_check
            && depth <= RFP_MAX_DEPTH
            && beta.abs() < VALUE_MATE_IN_MAX_PLY
            && static_eval - RFP_MARGIN * depth as Value >= beta
        {
            return static_eval;
        }

        // razoring（ADR-0057）: 静的評価がalphaを大きく下回るなら
        // 通常探索を省略してqsearchに降格する
        if excluded == Move::NONE
            && !is_pv
            && !in_check
            && depth <= RAZOR_MAX_DEPTH
            && alpha.abs() < VALUE_MATE_IN_MAX_PLY
            && static_eval + RAZOR_MARGIN <= alpha
        {
            return self.qsearch(alpha, beta, ply, 0);
        }

        // NMP（ADR-0028）。手番を渡して浅く探索し、それでもβ以上なら刈る。
        // 除外手つき探索中はスキップ（ADR-0050）
        if excluded == Move::NONE
            && !is_pv
            && !in_check
            && prev != Move::NULL
            && depth >= NMP_MIN_DEPTH
            && static_eval >= beta
            && beta.abs() < VALUE_MATE_IN_MAX_PLY
        {
            let r = NMP_BASE_REDUCTION + depth / 4;
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
            if v >= beta {
                // パス由来の詰みスコアは信用せずβに丸める
                return if v >= VALUE_MATE_IN_MAX_PLY { beta } else { v };
            }
        }

        // ProbCut（ADR-0051）。betaを大きく超えそうなノードでは、浅い確認探索で
        // 「十分良い取る手が1つある」ことを示せれば高深度の全探索を省いてカットする。
        // non-PV・非王手・除外手なし・depth>=5で発動。除外手つき探索中はスキップ。
        const PROBCUT_MARGIN: Value = 200;
        const PROBCUT_DEPTH_REDUCTION: u32 = 4;
        const PROBCUT_MIN_DEPTH: u32 = 5;
        let probcut_beta = beta + PROBCUT_MARGIN;
        if excluded == Move::NONE
            && !is_pv
            && !in_check
            && depth >= PROBCUT_MIN_DEPTH
            && beta.abs() < VALUE_MATE_IN_MAX_PLY
            // TTに深い情報があり矛盾する（probcut_beta未満と分かっている）ならスキップ
            && !(tt_hit.is_some()
                && tt_depth >= depth.saturating_sub(3)
                && tt_value < probcut_beta)
        {
            let mut list = MoveList::default();
            generate(&self.pos, GenType::Captures, false, &mut list);
            for &m in &list {
                // SEE>=0の取る手だけを確認対象にする
                if !self.pos.see_ge(m, 0) || !self.pos.is_legal(m) {
                    continue;
                }
                self.set_current_move(ply, m, !self.pos.piece_on(m.to()).is_empty());
                self.pos.do_move(m);
                self.evaluator.push(&self.pos);
                // まずqsearchで確認（窓は (-probcut_beta, -probcut_beta+1)）
                let mut v = -self.qsearch(-probcut_beta, -probcut_beta + 1, ply + 1, 0);
                // 通ったら同じ窓で通常探索 depth-4 を確認する
                if v >= probcut_beta {
                    let mut child_pv = Vec::new();
                    v = -self.search(
                        -probcut_beta,
                        -probcut_beta + 1,
                        depth - PROBCUT_DEPTH_REDUCTION,
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
                    // fail-soft。TTにlower bound・depth-3で保存してカットする
                    self.shared.tt.store(
                        key,
                        m.to_move16(),
                        value_to_tt(v, ply),
                        raw_eval as i16,
                        depth.saturating_sub(3).min(255) as u8,
                        Bound::Lower,
                        // 参照実装はttPvを書き戻す（yaneuraou-search.cpp:3418）
                        self.stack[ply + STACK_OFFSET].tt_pv,
                    );
                    return v;
                }
            }
        }

        // singular extension（ADR-0050）。TT手を除外した検証探索がsingular_beta
        // を下回れば、TT手だけが良い手と見て延長する。案A（単独延長のみ）
        let mut singular = false;
        if excluded == Move::NONE
            && depth >= 7
            && ply > 0
            && tt_move != Move::NONE
            && tt_bound != Bound::Upper
            && tt_bound != Bound::None
            && tt_depth >= depth.saturating_sub(3)
            && tt_value.abs() < VALUE_MATE_IN_MAX_PLY
            && self.pos.is_legal(tt_move)
        {
            let singular_beta = tt_value - 2 * depth as Value;
            self.stack[ply + STACK_OFFSET].excluded_move = tt_move;
            let mut verify_pv = Vec::new();
            let v = self.search(
                singular_beta - 1,
                singular_beta,
                depth / 2,
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
            // TT手が唯一の合法手なら検証探索はmated値を返し、必ず
            // singular=trueになる（唯一手の延長として意図どおり）
            singular = v < singular_beta;
        }

        // 置換表の下界による簡易ProbCut（ADR-0078）。探索を伴わない。
        // 除外手つき探索中はスキップする（ADR-0050）
        let tt_probcut_beta = beta + TT_PROBCUT_MARGIN;
        if excluded == Move::NONE
            && matches!(tt_bound, Bound::Lower | Bound::Exact)
            && tt_depth >= depth.saturating_sub(TT_PROBCUT_DEPTH_SLACK)
            && tt_value >= tt_probcut_beta
            && beta.abs() < VALUE_MATE_IN_MAX_PLY
            && tt_value.abs() < VALUE_MATE_IN_MAX_PLY
        {
            return tt_probcut_beta;
        }

        // 置換表の手が駒を取る手か（yaneuraou-search.cpp:2672）。
        // そうならこのノードの全手のリダクションを増やす
        let tt_capture = tt_move != Move::NONE
            && !tt_move.is_drop()
            && !self.pos.piece_on(tt_move.to()).is_empty();
        // PVでもcutでもないノード（yaneuraou-search.cpp:2251）。
        // 全手を調べる見込みなのでリダクションを強める
        let all_node = !(is_pv || cut_node);

        let mut picker = MovePicker::new(&self.pos, tt_move, depth as i32, ply, false);
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

            // 王手延長（ADR-0024）とsingular延長（ADR-0050）。どちらもTT手/王手を
            // +1する。両立時はmaxで重複させない（depthのまま、depth+1にしない）。
            // 枝刈りの尺度に使うため、ムーブループの枝刈りより前で決める
            let new_depth = if gives_check || (singular && m == tt_move) {
                depth
            } else {
                depth - 1
            };

            // LMRのリダクション量（1024倍の固定小数。G2）。枝刈りの尺度
            // （lmr_depth）と実際の浅い探索で同じ値を使う
            let delta = beta - alpha;
            let mut r = self.reduction(improving, depth, count, delta);
            // 項1: ttPvノードは削る（yaneuraou-search.cpp:3573-3574）。
            // 枝刈りの尺度に入るのはここまでで、残りはdo_moveの側で足す
            if self.stack[ply + STACK_OFFSET].tt_pv {
                r += 1013;
            }
            // lmr_depth: LMRで削ったあとに実際に読む深さ（ADR-0090）。
            // 生のdepthで枝刈りを判断すると、深いノードほど閾値が大きくなり
            // 刈りすぎる。実際に読む深さで測る。参照実装はここでクランプせず、
            // SEE枝刈りの直前で0止めする（yaneuraou-search.cpp:3600, 3691）
            let lmr_depth = new_depth as i32 - r / 1024;

            // move count pruning（ADR-0028, 0109）: 手数を使い切ったら、
            // MovePickerに静かな手の生成そのものをやめさせる。詰まされ筋では
            // 無効。参照実装は「rootでない」「bestValueが敗勢でない」の2条件
            // だけを課す（yaneuraou-search.cpp:3586-3594）
            if best > VALUE_MATED_IN_MAX_PLY && count >= lmp_limit(depth, improving) {
                picker.skip_quiet_moves();
            }

            // futility（ADR-0028）: 評価がalphaに遠く及ばない浅い静かな手を
            // 飛ばす。最初の手は必ず読む（countは既に加算済み）
            if !in_check
                && !is_capture
                && !gives_check
                && count > 1
                && depth <= FUTILITY_MAX_DEPTH
                && alpha.abs() < VALUE_MATE_IN_MAX_PLY
                && static_eval + FUTILITY_BASE + FUTILITY_MARGIN * depth as Value <= alpha
            {
                continue;
            }

            // SEEベースの枝刈り（ADR-0090）。移動先の取り合いを静的に解き、
            // 大きく駒損する手を読まずに捨てる。静かな手は実効深さの2乗、
            // 取る手は深さに比例した額まで損を許す
            if !is_pv && best > VALUE_MATED_IN_MAX_PLY && count > 1 && !in_check {
                let threshold = if is_capture {
                    -SEE_CAPTURE_COEF * depth as i32
                } else {
                    // 参照実装はここで0止めする（yaneuraou-search.cpp:3691）
                    let lmr_depth = lmr_depth.max(0);
                    -SEE_QUIET_COEF * lmr_depth * lmr_depth
                };
                if !self.pos.see_ge(m, threshold) {
                    continue;
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
                    + i32::from(tt_depth >= depth) * (935 + i32::from(cut_node) * 959);
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
            let value = if count == 1 {
                // 第1手はPVならcut_node = false、そうでなければ反転する
                -self.search(
                    -beta,
                    -alpha,
                    new_depth,
                    ply + 1,
                    m,
                    &mut child_pv,
                    is_pv,
                    !is_pv && !cut_node,
                )
            } else {
                // LMR（ADR-0028）: 遅い静かな手は浅いnull windowで読み、
                // alphaを超えたときだけ元の深さで読み直す
                let mut d = new_depth;
                let did_lmr = depth >= LMR_MIN_DEPTH
                    && count >= LMR_MIN_COUNT
                    && !is_capture
                    && !gives_check
                    && !in_check;
                if did_lmr {
                    let red = (r / 1024).max(0) as u32;
                    d = new_depth.saturating_sub(red).max(1);
                }
                // 削った浅い探索はcut_node = true、削らなかった全深さの
                // null windowは反転する（ADR-0109のG0）
                let child_cut = if d != new_depth { true } else { !cut_node };
                let mut v = -self.search(
                    -alpha - 1,
                    -alpha,
                    d,
                    ply + 1,
                    m,
                    &mut child_pv,
                    false,
                    child_cut,
                );
                // 減深探索がalphaを超えたかを、再探索の前に控える。
                // 参照実装が加点の条件に使うのは減深探索の値である
                // （yaneuraou-search.cpp:3989, 4008）
                let lmr_raised_alpha = did_lmr && v > alpha;
                if v > alpha && d < new_depth && !self.stopped() {
                    // LMR後の再探索も反転する
                    v = -self.search(
                        -alpha - 1,
                        -alpha,
                        new_depth,
                        ply + 1,
                        m,
                        &mut child_pv,
                        false,
                        !cut_node,
                    );
                }
                if lmr_raised_alpha {
                    // LMR後のcontinuation history更新
                    // （yaneuraou-search.cpp:4008）
                    self.update_continuation_histories(ply, m.piece_after(), m.to(), 1426);
                }
                if v > alpha && is_pv && !self.stopped() {
                    // PVの再探索はcut_node = false
                    -self.search(
                        -beta,
                        -alpha,
                        new_depth,
                        ply + 1,
                        m,
                        &mut child_pv,
                        true,
                        false,
                    )
                } else {
                    v
                }
            };
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
                        // （yaneuraou-search.cpp:4214）。本エンジンの延長は
                        // 最大1手なので `extension < 2` は常に成立する
                        self.stack[ply + STACK_OFFSET].cutoff_cnt += 1;
                        break;
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
            // ttMoveHistoryはここでしか更新しない。multi-cut由来の加点
            // （yaneuraou-search.cpp:3819）はsingularの多段化と同じ群なのでG5で足す
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
                depth.min(255) as u8,
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

    fn qsearch(&mut self, mut alpha: Value, beta: Value, ply: usize, qdepth: i32) -> Value {
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
        // 入玉宣言勝ち（ADR-0030）
        if self.pos.can_declare_win() {
            return mate_in(ply);
        }

        // 置換表probe（ADR-0054）。qsearchはPVノードのdepth 0からfull windowでも
        // 呼ばれる。boundと窓を照合し、条件を満たせば即カットする（fail-soft）。
        // TT手はpickerの先頭で試す。
        let orig_alpha = alpha;
        let key = self.pos.key();
        let tt_hit = self.shared.tt.probe(key);
        // 参照実装はqsearchでもStackへ記録する（yaneuraou-search.cpp:4648）
        self.stack[ply + STACK_OFFSET].tt_hit = tt_hit.is_some();
        let mut tt_move = Move::NONE;
        let mut tt_eval = VALUE_NONE;
        if let Some(data) = &tt_hit {
            if let Some(m) = self.pos.to_move(data.mv)
                && self.pos.pseudo_legal(m)
            {
                tt_move = m;
            }
            tt_eval = Value::from(data.eval);
            let tt_value = value_from_tt(data.value, ply);
            let usable = match data.bound {
                Bound::Exact => true,
                Bound::Lower => tt_value >= beta,
                Bound::Upper => tt_value <= alpha,
                Bound::None => false,
            };
            if usable {
                return tt_value;
            }
        }

        let in_check = self.pos.in_check();
        // continuation historyの面を決める材料（ADR-0109のG1）
        self.stack[ply + STACK_OFFSET].in_check = in_check;
        // stand patの生評価（王手中はなし）。TTのeval欄があればそれを優先し、
        // なければeval hash経由で計算する（ADR-0054, 0049）。store時のeval欄にも使う。
        let raw_eval = if in_check {
            VALUE_NONE
        } else if tt_eval != VALUE_NONE {
            tt_eval
        } else {
            self.eval_cached(key)
        };
        let mut best = -VALUE_INFINITE;
        // 静止探索のfutilityの基準（ADR-0077）。王手中は定義しない
        let mut futility_base = -VALUE_INFINITE;
        if !in_check {
            // stand pat（ADR-0024）。correction historyで補正する（ADR-0046）
            let stand = self.to_corrected(raw_eval, ply);
            if stand >= beta {
                return stand;
            }
            if stand > alpha {
                alpha = stand;
            }
            best = stand;
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

        // 入口plyだけ静かな王手も読む（ADR-0028の項目7）
        let mut picker = MovePicker::new(&self.pos, tt_move, 0, ply, qdepth == 0);
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
                // やねうら王はここでbestをfutility値まで引き上げるが、
                // 本エンジンのMultiPVはライン確定ごとに出力し、Stockfishの
                // ような確定後のソートを持たない。窓に依存する値をbestへ
                // 入れるとライン間のスコア順序が崩れるため引き上げない。
                // fail-softの下限を報告しないだけで、探索の正しさは保たれる
                let futility_value = futility_base + gain;
                if futility_value <= alpha {
                    continue;
                }
                if !self.pos.see_ge(m, alpha - futility_base) {
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
            let value = -self.qsearch(-beta, -alpha, ply + 1, qdepth - 1);
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
        if in_check && count == 0 {
            // 王手回避で手なし = 詰み。mated値をTTにも保存する（ADR-0054）
            best = mated_in(ply);
        }

        // 置換表store（ADR-0054）。深さは0固定。boundはfail-high/low/exactで決める。
        let bound = if best >= beta {
            Bound::Lower
        } else if best > orig_alpha {
            Bound::Exact
        } else {
            Bound::Upper
        };
        self.shared.tt.store(
            key,
            best_move.to_move16(),
            value_to_tt(best, ply),
            raw_eval as i16,
            0,
            bound,
            false,
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
