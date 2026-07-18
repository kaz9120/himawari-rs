//! 探索v1（ADR-0024, 0026）。
//!
//! fail-softのalpha-beta＋反復深化＋aspiration window。
//! PVは三角配列（Vecの連結）。詰みスコアはTT境界でply補正する。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use himawari_core::{Move, MoveList, Position, Repetition, generate_legal};

use crate::eval::Evaluator;
use crate::movepick::{CounterMoves, History, MovePicker};
use crate::timeman::{Limits, TimeManager};
use crate::tt::{Bound, Tt};
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
}

impl Shared {
    pub fn new(hash_mb: usize) -> Shared {
        Shared {
            stop: AtomicBool::new(false),
            nodes: AtomicU64::new(0),
            tt: Tt::new(hash_mb),
        }
    }
}

/// 反復深化1周分の報告。
pub struct IterInfo {
    pub depth: u32,
    pub score: Value,
    pub pv: Vec<Move>,
    pub nodes: u64,
    pub elapsed_ms: u64,
    pub hashfull: usize,
}

pub struct SearchResult {
    pub best: Move,
    pub score: Value,
}

pub struct Worker {
    pub pos: Position,
    pub evaluator: Evaluator,
    pub history: History,
    pub counters: CounterMoves,
    /// plyごとの静的評価（improving判定用。王手中はVALUE_NONE）。
    eval_stack: Vec<Value>,
    killers: Vec<[Move; 2]>,
    nodes: u64,
    shared: Arc<Shared>,
    tm: TimeManager,
    limits: Limits,
    max_moves_to_draw: u16,
    root_moves: Vec<Move>,
}

impl Worker {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pos: Position,
        shared: Arc<Shared>,
        limits: Limits,
        tm: TimeManager,
        max_moves_to_draw: u16,
        evaluator: Evaluator,
        history: History,
        counters: CounterMoves,
    ) -> Worker {
        Worker {
            pos,
            evaluator,
            history,
            counters,
            eval_stack: vec![VALUE_NONE; MAX_PLY + 2],
            killers: vec![[Move::NONE; 2]; MAX_PLY + 2],
            nodes: 0,
            shared,
            tm,
            limits,
            max_moves_to_draw,
            root_moves: Vec::new(),
        }
    }

    #[inline]
    fn stopped(&self) -> bool {
        self.shared.stop.load(Ordering::Relaxed)
    }

    /// 定期的な時間・ノード制限の検査（メイン探索スレッドの責務。ADR-0020）。
    #[inline]
    fn check_limits(&self) {
        if self.nodes.is_multiple_of(2048) {
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

    /// 反復深化。各イテレーション完了時にon_iterを呼ぶ。
    pub fn iterate(&mut self, on_iter: &mut dyn FnMut(IterInfo)) -> SearchResult {
        self.shared.tt.new_search();
        self.evaluator.new_search(&self.pos);
        let mut list = MoveList::default();
        generate_legal(&self.pos, false, &mut list);
        self.root_moves = list.as_slice().to_vec();
        if self.root_moves.is_empty() {
            return SearchResult {
                best: Move::RESIGN,
                score: mated_in(0),
            };
        }
        let mut best_move = self.root_moves[0];
        let mut last_score = VALUE_ZERO;
        let max_depth = if self.limits.depth > 0 {
            self.limits.depth
        } else {
            (MAX_PLY - 1) as u32
        };

        for depth in 1..=max_depth {
            let mut delta = 20;
            let (mut alpha, mut beta) = if depth >= 5 {
                (last_score - delta, last_score + delta)
            } else {
                (-VALUE_INFINITE, VALUE_INFINITE)
            };
            loop {
                let (score, pv) = self.search_root(depth, alpha, beta);
                if self.stopped() {
                    break;
                }
                if score <= alpha {
                    beta = (alpha + beta) / 2;
                    alpha = (score - delta).max(-VALUE_INFINITE);
                    delta += delta / 2;
                } else if score >= beta {
                    beta = (score + delta).min(VALUE_INFINITE);
                    delta += delta / 2;
                } else {
                    last_score = score;
                    if let Some(&m) = pv.first() {
                        best_move = m;
                        // 次イテレーションで最善手から探索する
                        if let Some(i) = self.root_moves.iter().position(|&r| r == m) {
                            self.root_moves.remove(i);
                            self.root_moves.insert(0, m);
                        }
                    }
                    on_iter(IterInfo {
                        depth,
                        score,
                        pv,
                        nodes: self.nodes,
                        elapsed_ms: self.tm.elapsed().as_millis() as u64,
                        hashfull: self.shared.tt.hashfull(),
                    });
                    break;
                }
            }
            if self.stopped() || self.tm.over_optimum() {
                break;
            }
            if self.limits.nodes > 0 && self.nodes >= self.limits.nodes {
                break;
            }
        }
        self.shared.nodes.fetch_add(self.nodes, Ordering::Relaxed);
        SearchResult {
            best: best_move,
            score: last_score,
        }
    }

    fn search_root(&mut self, depth: u32, mut alpha: Value, beta: Value) -> (Value, Vec<Move>) {
        let mut best = -VALUE_INFINITE;
        let mut best_pv: Vec<Move> = Vec::new();
        let moves = self.root_moves.clone();
        for (i, &m) in moves.iter().enumerate() {
            self.pos.do_move(m);
            self.evaluator.push(&self.pos);
            let mut child_pv = Vec::new();
            let value = if i == 0 {
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
                return (best, best_pv);
            }
            if value > best {
                best = value;
                if value > alpha {
                    alpha = value;
                    best_pv.clear();
                    best_pv.push(m);
                    best_pv.extend_from_slice(&child_pv);
                    if value >= beta {
                        break;
                    }
                }
            }
        }
        (best, best_pv)
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

        // mate distance pruning
        alpha = alpha.max(mated_in(ply));
        let beta = beta.min(mate_in(ply + 1));
        if alpha >= beta {
            return alpha;
        }

        // 置換表（ADR-0022, 0024）
        let key = self.pos.key();
        let tt_hit = self.shared.tt.probe(key);
        let mut tt_move = Move::NONE;
        if let Some(data) = &tt_hit {
            if let Some(m) = self.pos.to_move(data.mv)
                && self.pos.pseudo_legal(m)
            {
                tt_move = m;
            }
            if !is_pv && u32::from(data.depth) >= depth {
                let v = value_from_tt(data.value, ply);
                let usable = match data.bound {
                    Bound::Exact => true,
                    Bound::Lower => v >= beta,
                    Bound::Upper => v <= alpha,
                    Bound::None => false,
                };
                if usable {
                    return v;
                }
            }
        }

        if depth == 0 {
            return self.qsearch(alpha, beta, ply);
        }

        // 静的評価（ADR-0028）。王手中はVALUE_NONE。TTのevalを再利用する
        let in_check = self.pos.in_check();
        let static_eval = if in_check {
            VALUE_NONE
        } else {
            match &tt_hit {
                Some(d) if Value::from(d.eval) != VALUE_NONE => Value::from(d.eval),
                _ => self.evaluator.evaluate(&self.pos),
            }
        };

        self.eval_stack[ply] = static_eval;
        // 2手前より静的評価が改善しているか（枝刈りの強弱に使う）
        let improving = !in_check
            && ply >= 2
            && self.eval_stack[ply - 2] != VALUE_NONE
            && static_eval > self.eval_stack[ply - 2];

        // reverse futility（ADR-0028）: 静的評価がβを大きく超えるなら刈る
        if !is_pv
            && !in_check
            && depth <= RFP_MAX_DEPTH
            && beta.abs() < VALUE_MATE_IN_MAX_PLY
            && static_eval - RFP_MARGIN * depth as Value >= beta
        {
            return static_eval;
        }

        // NMP（ADR-0028）。手番を渡して浅く探索し、それでもβ以上なら刈る
        if !is_pv
            && !in_check
            && prev != Move::NULL
            && depth >= NMP_MIN_DEPTH
            && static_eval >= beta
            && beta.abs() < VALUE_MATE_IN_MAX_PLY
        {
            let r = NMP_BASE_REDUCTION + depth / 4;
            let mut null_pv = Vec::new();
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

        let mut picker = MovePicker::new(
            &self.pos,
            tt_move,
            self.killers[ply],
            self.counters.get(prev),
        );
        let mut best = -VALUE_INFINITE;
        let mut best_move = Move::NONE;
        let mut count = 0u32;
        let mut tried_quiets: Vec<Move> = Vec::new();
        let mut child_pv = Vec::new();

        while let Some(m) = picker.next(&self.pos, &self.history) {
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

            // 王手延長（ADR-0024の骨格。深さは減らさない）
            let new_depth = if gives_check { depth } else { depth - 1 };

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
            static_eval as i16,
            depth.min(255) as u8,
            bound,
            is_pv,
        );
        best
    }

    fn qsearch(&mut self, mut alpha: Value, beta: Value, ply: usize) -> Value {
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

        let in_check = self.pos.in_check();
        let mut best = -VALUE_INFINITE;
        if !in_check {
            // stand pat（ADR-0024）
            let stand = self.evaluator.evaluate(&self.pos);
            if stand >= beta {
                return stand;
            }
            if stand > alpha {
                alpha = stand;
            }
            best = stand;
        }

        let mut picker = MovePicker::new_qsearch(&self.pos);
        let mut count = 0u32;
        while let Some(m) = picker.next(&self.pos, &self.history) {
            if !self.pos.is_legal(m) {
                continue;
            }
            count += 1;
            self.pos.do_move(m);
            self.evaluator.push(&self.pos);
            let value = -self.qsearch(-beta, -alpha, ply + 1);
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
        for &q in tried {
            if q != m {
                self.history.update(q, -bonus);
            }
        }
    }
}
