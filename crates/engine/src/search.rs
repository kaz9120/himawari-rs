//! 探索v1（ADR-0024, 0026）。
//!
//! fail-softのalpha-beta＋反復深化＋aspiration window。
//! PVは三角配列（Vecの連結）。詰みスコアはTT境界でply補正する。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use himawari_core::{GenType, Move, MoveList, Position, Repetition, generate, generate_legal};

use crate::eval::Evaluator;
use crate::movepick::{ContinuationHistory, CorrectionHistory, CounterMoves, History, MovePicker};
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
/// move count pruningの最大深さ。
const LMP_MAX_DEPTH: u32 = 8;
/// IIR: TTに手がないノードを1浅く読む最小深さ。
const IIR_MIN_DEPTH: u32 = 4;

/// この手数を超えた静かな手を捨てる（ADR-0028）。
fn lmp_limit(depth: u32, improving: bool) -> u32 {
    let base = 3 + depth * depth;
    if improving { base } else { base / 2 }
}

/// LMRのリダクション表。r = 0.5 + ln(depth)・ln(count) / 2.25。
static LMR_TABLE: std::sync::OnceLock<[[u8; 64]; 64]> = std::sync::OnceLock::new();

fn lmr_reduction(depth: u32, count: u32) -> u32 {
    let t = LMR_TABLE.get_or_init(|| {
        let mut t = [[0u8; 64]; 64];
        for (d, row) in t.iter_mut().enumerate().skip(1) {
            for (c, r) in row.iter_mut().enumerate().skip(1) {
                *r = (0.5 + (d as f64).ln() * (c as f64).ln() / 2.25) as u8;
            }
        }
        t
    });
    u32::from(t[depth.min(63) as usize][count.min(63) as usize])
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

/// 反復深化1周分の報告（MultiPVでは1ラインごとに1回）。
pub struct IterInfo {
    pub depth: u32,
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
}

pub struct SearchResult {
    pub best: Move,
    pub score: Value,
    /// 相手の予測応手（PVの2手目。なければNONE。ADR-0033）。
    pub ponder: Move,
}

pub struct Worker {
    pub pos: Position,
    pub evaluator: Evaluator,
    pub history: History,
    pub counters: CounterMoves,
    /// 静的評価のcorrection history（ADR-0046）。
    pub corr: CorrectionHistory,
    /// continuation history（ADR-0047）。
    pub cont: ContinuationHistory,
    /// plyごとの指し手スタック（ADR-0047）。move_stack[ply]はその
    /// plyで指した手（null moveはMove::NONE）。1手前・2手前の参照に使う。
    move_stack: Vec<Move>,
    /// plyごとの静的評価（improving判定用。王手中はVALUE_NONE）。
    eval_stack: Vec<Value>,
    /// plyごとの除外手（singular extension用。ADR-0050）。検証探索中は
    /// そのplyのexcluded_stackにTT手が入り、ムーブループで飛ばす。
    excluded_stack: Vec<Move>,
    killers: Vec<[Move; 2]>,
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
        history: History,
        counters: CounterMoves,
        corr: CorrectionHistory,
        cont: ContinuationHistory,
    ) -> Worker {
        Worker {
            pos,
            evaluator,
            history,
            counters,
            corr,
            cont,
            move_stack: vec![Move::NONE; MAX_PLY + 2],
            eval_stack: vec![VALUE_NONE; MAX_PLY + 2],
            excluded_stack: vec![Move::NONE; MAX_PLY + 2],
            killers: vec![[Move::NONE; 2]; MAX_PLY + 2],
            nodes: 0,
            shared,
            tm,
            limits,
            max_moves_to_draw,
            multi_pv: multi_pv.max(1),
            root_moves: Vec::new(),
        }
    }

    #[inline]
    fn stopped(&self) -> bool {
        self.shared.stop.load(Ordering::Relaxed)
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

    /// 生の静的評価にcorrection historyの補正を加える（ADR-0046）。
    /// 補正後が詰み圏に入らないようクランプする。
    #[inline]
    fn to_corrected(&self, raw: Value) -> Value {
        let stm = self.pos.side_to_move();
        let corr = self.corr.get(stm, self.pos.pawn_key()) / 8;
        (raw + corr).clamp(VALUE_MATED_IN_MAX_PLY + 1, VALUE_MATE_IN_MAX_PLY - 1)
    }

    /// 反復深化。各イテレーション完了時にon_iterを呼ぶ。
    pub fn iterate(&mut self, on_iter: &mut dyn FnMut(IterInfo)) -> SearchResult {
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
        let max_depth = if self.limits.depth > 0 {
            self.limits.depth
        } else {
            (MAX_PLY - 1) as u32
        };

        'deepening: for depth in 1..=max_depth {
            for rm in &mut self.root_moves {
                rm.prev_score = rm.score;
            }
            let lines = self.multi_pv.min(self.root_moves.len());
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
                    let (score, best_idx, pv) = self.search_root(depth, alpha, beta, pv_idx);
                    if self.stopped() {
                        break 'deepening;
                    }
                    if score <= alpha {
                        beta = (alpha + beta) / 2;
                        alpha = (score - delta).max(-VALUE_INFINITE);
                        delta += delta / 2;
                    } else if score >= beta {
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
                        if pv_idx == 0 {
                            last_score = score;
                        }
                        on_iter(IterInfo {
                            depth,
                            multipv: if self.multi_pv > 1 { pv_idx + 1 } else { 0 },
                            score,
                            pv,
                            // 全ワーカー合算（単スレッドではローカル値と一致）
                            nodes: self.shared.nodes.load(Ordering::Relaxed).max(self.nodes),
                            elapsed_ms: self.tm.elapsed().as_millis() as u64,
                            hashfull: self.shared.tt.hashfull(),
                        });
                        break;
                    }
                }
            }
            if self.stopped() || self.tm.over_optimum() {
                break;
            }
            if self.limits.nodes > 0 && self.nodes >= self.limits.nodes {
                break;
            }
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
    fn search_root(
        &mut self,
        depth: u32,
        mut alpha: Value,
        beta: Value,
        pv_idx: usize,
    ) -> (Value, usize, Vec<Move>) {
        let mut best = -VALUE_INFINITE;
        let mut best_idx = pv_idx;
        let mut best_pv: Vec<Move> = Vec::new();
        let moves: Vec<Move> = self.root_moves[pv_idx..].iter().map(|rm| rm.mv).collect();
        for (j, &m) in moves.iter().enumerate() {
            let i = pv_idx + j;
            self.move_stack[0] = m;
            self.pos.do_move(m);
            self.evaluator.push(&self.pos);
            let mut child_pv = Vec::new();
            let value = if j == 0 {
                -self.search(-beta, -alpha, depth - 1, 1, m, &mut child_pv, true)
            } else {
                let v = -self.search(-alpha - 1, -alpha, depth - 1, 1, m, &mut child_pv, false);
                if v > alpha && !self.stopped() {
                    -self.search(-beta, -alpha, depth - 1, 1, m, &mut child_pv, true)
                } else {
                    v
                }
            };
            self.evaluator.pop();
            self.pos.undo_move(m);
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
    ) -> Value {
        pv.clear();
        if self.stopped() {
            return VALUE_ZERO;
        }
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

        // 除外手（singular extension用。ADR-0050）。検証探索中はTT手が入る
        let excluded = self.excluded_stack[ply];

        // 置換表（ADR-0022, 0024）
        let key = self.pos.key();
        let tt_hit = self.shared.tt.probe(key);
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
        let in_check = self.pos.in_check();
        let raw_eval = if in_check {
            VALUE_NONE
        } else {
            match &tt_hit {
                Some(d) if Value::from(d.eval) != VALUE_NONE => Value::from(d.eval),
                // TTにevalがなければeval hash経由で計算する（ADR-0049）
                _ => self.eval_cached(key),
            }
        };
        let static_eval = if in_check {
            VALUE_NONE
        } else {
            self.to_corrected(raw_eval)
        };

        self.eval_stack[ply] = static_eval;
        // 2手前より静的評価が改善しているか（枝刈りの強弱に使う）
        let improving = !in_check
            && ply >= 2
            && self.eval_stack[ply - 2] != VALUE_NONE
            && static_eval > self.eval_stack[ply - 2];

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
            self.move_stack[ply] = Move::NONE;
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
                self.move_stack[ply] = m;
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
                        is_pv,
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
            // 検証探索はkillers[ply]を書き換える（update_quiet_stats）。
            // このノードのpickerが読む前に退避・復元する（ADR-0050の注意点）
            let saved_killers = self.killers[ply];
            self.excluded_stack[ply] = tt_move;
            let mut verify_pv = Vec::new();
            let v = self.search(
                singular_beta - 1,
                singular_beta,
                depth / 2,
                ply,
                prev,
                &mut verify_pv,
                false,
            );
            self.excluded_stack[ply] = Move::NONE;
            self.killers[ply] = saved_killers;
            // 検証探索の再帰でeval_stack[ply]が同値で上書きされる。念のため戻す
            self.eval_stack[ply] = static_eval;
            if self.stopped() {
                return VALUE_ZERO;
            }
            // TT手が唯一の合法手なら検証探索はmated値を返し、必ず
            // singular=trueになる（唯一手の延長として意図どおり）
            singular = v < singular_beta;
        }

        let mut picker = MovePicker::new(
            &self.pos,
            tt_move,
            self.killers[ply],
            self.counters.get(prev),
        );
        // continuation history用の1手前・2手前（ADR-0047）。NONEはget側で0になる
        let prev1 = if ply >= 1 {
            self.move_stack[ply - 1]
        } else {
            Move::NONE
        };
        let prev2 = if ply >= 2 {
            self.move_stack[ply - 2]
        } else {
            Move::NONE
        };
        let mut best = -VALUE_INFINITE;
        let mut best_move = Move::NONE;
        let mut best_move_is_capture = false;
        let mut count = 0u32;
        let mut tried_quiets: Vec<Move> = Vec::new();
        let mut child_pv = Vec::new();

        while let Some(m) = picker.next(&self.pos, &self.history, &self.cont, prev1, prev2) {
            // 除外手はスキップ（singular検証探索。ADR-0050）。通常はexcluded==NONE
            if m == excluded {
                continue;
            }
            if !self.pos.is_legal(m) {
                continue;
            }
            count += 1;
            let is_capture = !m.is_drop() && !self.pos.piece_on(m.to()).is_empty();
            let gives_check = self.pos.gives_check(m);

            // move count pruning（ADR-0028）: 浅い深さで手数を使い切ったら
            // 残りの静かな手を捨てる。詰まされ筋では無効
            if !is_pv
                && !in_check
                && !is_capture
                && !gives_check
                && depth <= LMP_MAX_DEPTH
                && best > VALUE_MATED_IN_MAX_PLY
                && count > lmp_limit(depth, improving)
            {
                continue;
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

            // 王手延長（ADR-0024）とsingular延長（ADR-0050）。どちらもTT手/王手を
            // +1する。両立時はmaxで重複させない（depthのまま、depth+1にしない）
            let new_depth = if gives_check || (singular && m == tt_move) {
                depth
            } else {
                depth - 1
            };

            self.move_stack[ply] = m;
            self.pos.do_move(m);
            self.evaluator.push(&self.pos);
            let value = if count == 1 {
                -self.search(-beta, -alpha, new_depth, ply + 1, m, &mut child_pv, is_pv)
            } else {
                // LMR（ADR-0028）: 遅い静かな手は浅いnull windowで読み、
                // alphaを超えたときだけ元の深さで読み直す
                let mut d = new_depth;
                if depth >= LMR_MIN_DEPTH
                    && count >= LMR_MIN_COUNT
                    && !is_capture
                    && !gives_check
                    && !in_check
                {
                    let mut r = lmr_reduction(depth, count);
                    if is_pv {
                        r = r.saturating_sub(1);
                    }
                    d = new_depth.saturating_sub(r).max(1);
                }
                let mut v = -self.search(-alpha - 1, -alpha, d, ply + 1, m, &mut child_pv, false);
                if v > alpha && d < new_depth && !self.stopped() {
                    v = -self.search(
                        -alpha - 1,
                        -alpha,
                        new_depth,
                        ply + 1,
                        m,
                        &mut child_pv,
                        false,
                    );
                }
                if v > alpha && is_pv && !self.stopped() {
                    -self.search(-beta, -alpha, new_depth, ply + 1, m, &mut child_pv, true)
                } else {
                    v
                }
            };
            self.evaluator.pop();
            self.pos.undo_move(m);
            if self.stopped() {
                return VALUE_ZERO;
            }

            if !is_capture {
                tried_quiets.push(m);
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
                        if !is_capture {
                            self.update_quiet_stats(m, prev, ply, depth, &tried_quiets);
                        }
                        break;
                    }
                    alpha = value;
                }
            }
        }

        if count == 0 {
            // 合法手なし = 詰み（将棋はステイルメイトも負け）
            return mated_in(ply);
        }

        // 除外手つき探索中はTT store・correction history更新をしない（ADR-0050）。
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
                is_pv,
            );

            // correction history更新（ADR-0046）。main searchのノード確定後のみ。
            // 静かな結論で、boundと補正後静的評価が矛盾しないときだけ蓄積する
            if !in_check
                && best.abs() < VALUE_MATE_IN_MAX_PLY
                && (best_move == Move::NONE || !best_move_is_capture)
                && !(best >= beta && best <= static_eval)
                && !(best_move == Move::NONE && best >= static_eval)
            {
                let diff = best - static_eval;
                let bonus = (diff * depth as i32 / 8).clamp(-128, 128);
                let stm = self.pos.side_to_move();
                self.corr.update(stm, self.pos.pawn_key(), bonus);
            }
        }
        best
    }

    fn qsearch(&mut self, mut alpha: Value, beta: Value, ply: usize, qdepth: i32) -> Value {
        if self.stopped() {
            return VALUE_ZERO;
        }
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

        let in_check = self.pos.in_check();
        let mut best = -VALUE_INFINITE;
        if !in_check {
            // stand pat（ADR-0024）。eval hash経由で生評価を得（ADR-0049）、
            // correction historyで補正する（ADR-0046）
            let raw = self.eval_cached(self.pos.key());
            let stand = self.to_corrected(raw);
            if stand >= beta {
                return stand;
            }
            if stand > alpha {
                alpha = stand;
            }
            best = stand;
        }

        // 入口plyだけ静かな王手も読む（ADR-0028の項目7）
        let mut picker = MovePicker::new_qsearch(&self.pos, qdepth == 0);
        let mut count = 0u32;
        // qsearchのオーダリングはcontを使わない（ADR-0047のスコープ外）
        while let Some(m) =
            picker.next(&self.pos, &self.history, &self.cont, Move::NONE, Move::NONE)
        {
            if !self.pos.is_legal(m) {
                continue;
            }
            count += 1;
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
                    alpha = value;
                    if value >= beta {
                        break;
                    }
                }
            }
        }
        if in_check && count == 0 {
            return mated_in(ply);
        }
        best
    }

    fn update_quiet_stats(&mut self, m: Move, prev: Move, ply: usize, depth: u32, tried: &[Move]) {
        let k = &mut self.killers[ply];
        if k[0] != m {
            k[1] = k[0];
            k[0] = m;
        }
        self.counters.update(prev, m);
        let bonus = (depth * depth + 2 * depth) as i32;
        self.history.update(m, bonus);
        // continuation history: 1手前・2手前の文脈にbonus/malusを与える（ADR-0047）
        let prev1 = if ply >= 1 {
            self.move_stack[ply - 1]
        } else {
            Move::NONE
        };
        let prev2 = if ply >= 2 {
            self.move_stack[ply - 2]
        } else {
            Move::NONE
        };
        self.cont.update(prev1, m, bonus);
        self.cont.update(prev2, m, bonus);
        for &q in tried {
            if q != m {
                self.history.update(q, -bonus);
                self.cont.update(prev1, q, -bonus);
                self.cont.update(prev2, q, -bonus);
            }
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
            History::default(),
            CounterMoves::default(),
            CorrectionHistory::default(),
            ContinuationHistory::default(),
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
