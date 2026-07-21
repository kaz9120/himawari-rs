//! 探索v1（ADR-0024, 0026）。
//!
//! fail-softのalpha-beta＋反復深化＋aspiration window。
//! PVは三角配列（Vecの連結）。詰みスコアはTT境界でply補正する。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use himawari_core::{Move, MoveList, Position, Repetition, generate_legal};

use crate::eval::Evaluator;
use crate::movepick::{CorrectionHistory, CounterMoves, History, MovePicker};
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
    /// plyごとの静的評価（improving判定用。王手中はVALUE_NONE）。
    eval_stack: Vec<Value>,
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
    ) -> Worker {
        Worker {
            pos,
            evaluator,
            history,
            counters,
            corr,
            eval_stack: vec![VALUE_NONE; MAX_PLY + 2],
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
                _ => self.evaluator.evaluate(&self.pos),
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
        let mut best_move_is_capture = false;
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
            // stand pat（ADR-0024）。correction historyで補正する（ADR-0046）
            let raw = self.evaluator.evaluate(&self.pos);
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
        while let Some(m) = picker.next(&self.pos, &self.history) {
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
        for &q in tried {
            if q != m {
                self.history.update(q, -bonus);
            }
        }
    }
}
