//! 指し手オーダリング（ADR-0025）。
//!
//! MovePickerは段階生成の状態機械。カットが早いノードでは
//! Quietsの生成自体を省く。borrow衝突を避けるため、
//! next()は毎回&Positionと&Historyを受け取る。

use himawari_core::{Color, GenType, Move, MoveList, Position, generate, piece_value};

/// main history（[移動後の駒 32][移動先 81]。駒打ちも表現できる）。
pub struct History {
    table: Box<[[i16; 81]; 32]>,
}

impl Default for History {
    fn default() -> Self {
        History {
            table: Box::new([[0; 81]; 32]),
        }
    }
}

impl History {
    #[inline]
    pub fn get(&self, m: Move) -> i32 {
        i32::from(self.table[m.piece_after().index()][m.to().index()])
    }

    /// gravity方式の更新。bonusは±depth²程度。
    pub fn update(&mut self, m: Move, bonus: i32) {
        let bonus = bonus.clamp(-4000, 4000);
        let h = &mut self.table[m.piece_after().index()][m.to().index()];
        *h += (bonus - i32::from(*h) * bonus.abs() / 16384) as i16;
    }

    pub fn clear(&mut self) {
        *self.table = [[0; 81]; 32];
    }
}

/// 静的評価の系統誤差を補正する履歴（ADR-0046）。
/// [手番][pawn_key下位14bit]に、探索値と静的評価の乖離を蓄積する。
pub struct CorrectionHistory {
    table: Box<[[i16; 16384]; 2]>,
}

impl Default for CorrectionHistory {
    fn default() -> Self {
        CorrectionHistory {
            table: Box::new([[0; 16384]; 2]),
        }
    }
}

impl CorrectionHistory {
    #[inline]
    fn slot(pawn_key: u64) -> usize {
        (pawn_key & 0x3FFF) as usize
    }

    #[inline]
    pub fn get(&self, stm: Color, pawn_key: u64) -> i32 {
        i32::from(self.table[stm.index()][Self::slot(pawn_key)])
    }

    /// gravity方式の更新（値域±1024）。bonusは呼び出し側で±128にクランプ済み。
    pub fn update(&mut self, stm: Color, pawn_key: u64, bonus: i32) {
        let e = &mut self.table[stm.index()][Self::slot(pawn_key)];
        *e += (bonus - i32::from(*e) * bonus.abs() / 1024) as i16;
    }

    pub fn clear(&mut self) {
        *self.table = [[0; 16384]; 2];
    }
}

/// continuation correction history（ADR-0085）。
/// [1手前の駒 32][移動先 81]に、探索値と静的評価の乖離を蓄積する。
/// 局面のキーではなく直前の指し手を条件にする点が
/// [`CorrectionHistory`] と違う。
pub struct ContinuationCorrectionHistory {
    table: Box<[[i16; 81]; 32]>,
}

impl Default for ContinuationCorrectionHistory {
    fn default() -> Self {
        ContinuationCorrectionHistory {
            table: Box::new([[0; 81]; 32]),
        }
    }
}

impl ContinuationCorrectionHistory {
    #[inline]
    pub fn get(&self, prev: Move) -> i32 {
        if prev == Move::NONE || prev.is_special() {
            return 0;
        }
        i32::from(self.table[prev.piece_after().index()][prev.to().index()])
    }

    /// gravity方式の更新（値域±1024）。[`CorrectionHistory`] と同一。
    pub fn update(&mut self, prev: Move, bonus: i32) {
        if prev == Move::NONE || prev.is_special() {
            return;
        }
        let e = &mut self.table[prev.piece_after().index()][prev.to().index()];
        *e += (bonus - i32::from(*e) * bonus.abs() / 1024) as i16;
    }

    pub fn clear(&mut self) {
        *self.table = [[0; 81]; 32];
    }
}

/// counter move（[直前の手の駒 32][移動先 81]）。
pub struct CounterMoves {
    table: Box<[[Move; 81]; 32]>,
}

impl Default for CounterMoves {
    fn default() -> Self {
        CounterMoves {
            table: Box::new([[Move::NONE; 81]; 32]),
        }
    }
}

impl CounterMoves {
    #[inline]
    pub fn get(&self, prev: Move) -> Move {
        if prev == Move::NONE || prev.is_special() {
            return Move::NONE;
        }
        self.table[prev.piece_after().index()][prev.to().index()]
    }

    pub fn update(&mut self, prev: Move, response: Move) {
        if prev == Move::NONE || prev.is_special() {
            return;
        }
        self.table[prev.piece_after().index()][prev.to().index()] = response;
    }

    pub fn clear(&mut self) {
        *self.table = [[Move::NONE; 81]; 32];
    }
}

/// continuation history（ADR-0047）。
/// 論理次元は[条件手 piece_after 32][条件手 to 81][応手 piece_after 32][応手 to 81]。
/// 条件手が直前手（1手前）・2手前のとき、この応手が良かったかをスコアで持つ。
/// 巨大ネスト配列のスタック経由初期化はオーバーフローの危険があるため、
/// フラットなboxed sliceで確保し添字を計算する（約13.4MB）。
pub struct ContinuationHistory {
    table: Box<[i16]>,
}

const CONT_PIECE: usize = 32;
const CONT_SQ: usize = 81;
const CONT_LEN: usize = CONT_PIECE * CONT_SQ * CONT_PIECE * CONT_SQ;

impl Default for ContinuationHistory {
    fn default() -> Self {
        ContinuationHistory {
            table: vec![0i16; CONT_LEN].into_boxed_slice(),
        }
    }
}

impl ContinuationHistory {
    #[inline]
    fn index(prev: Move, m: Move) -> usize {
        ((prev.piece_after().index() * CONT_SQ + prev.to().index()) * CONT_PIECE
            + m.piece_after().index())
            * CONT_SQ
            + m.to().index()
    }

    #[inline]
    pub fn get(&self, prev: Move, m: Move) -> i32 {
        if prev == Move::NONE || prev.is_special() {
            return 0;
        }
        i32::from(self.table[Self::index(prev, m)])
    }

    /// gravity方式の更新。main historyと同一（クランプ±4000、divisor 16384）。
    pub fn update(&mut self, prev: Move, m: Move, bonus: i32) {
        if prev == Move::NONE || prev.is_special() {
            return;
        }
        let bonus = bonus.clamp(-4000, 4000);
        let h = &mut self.table[Self::index(prev, m)];
        *h += (bonus - i32::from(*h) * bonus.abs() / 16384) as i16;
    }

    pub fn clear(&mut self) {
        self.table.iter_mut().for_each(|x| *x = 0);
    }
}

#[derive(PartialEq, Eq)]
enum Stage {
    TtMove,
    CapturesInit,
    GoodCaptures,
    Killer(usize),
    Counter,
    QuietsInit,
    Quiets,
    BadCaptures,
    EvasionsInit,
    Evasions,
    QCapturesInit,
    QCaptures,
    QChecksInit,
    QChecks,
    Done,
}

/// 取る手のスコア（MVV優先＋成りボーナス）。
fn capture_score(pos: &Position, m: Move) -> i32 {
    let victim = piece_value(pos.piece_on(m.to()).piece_type());
    let promo = if m.is_promote() {
        piece_value(m.piece_after().piece_type()) - piece_value(m.piece_before().piece_type())
    } else {
        0
    };
    victim * 16 + promo * 16 - piece_value(m.piece_before().piece_type())
}

pub struct MovePicker {
    stage: Stage,
    tt_move: Move,
    killers: [Move; 2],
    counter: Move,
    scored: Vec<(Move, i32)>,
    bad_captures: Vec<Move>,
    yielded_quiet_stage: [Move; 3],
    qsearch: bool,
    /// qsearchの入口plyだけ、取る手の後に静かな王手も返す（ADR-0028）。
    with_checks: bool,
}

impl MovePicker {
    pub fn new(pos: &Position, tt_move: Move, killers: [Move; 2], counter: Move) -> Self {
        let stage = if pos.in_check() {
            Stage::EvasionsInit
        } else if tt_move != Move::NONE {
            Stage::TtMove
        } else {
            Stage::CapturesInit
        };
        MovePicker {
            stage,
            tt_move,
            killers,
            counter,
            scored: Vec::with_capacity(64),
            bad_captures: Vec::new(),
            yielded_quiet_stage: [Move::NONE; 3],
            qsearch: false,
            with_checks: false,
        }
    }

    pub fn new_qsearch(pos: &Position, tt_move: Move, with_checks: bool) -> Self {
        // 王手中はTT手を試さずEvasionsから始める（main searchのnewと同流儀）。
        let stage = if pos.in_check() {
            Stage::EvasionsInit
        } else if tt_move != Move::NONE {
            Stage::TtMove
        } else {
            Stage::QCapturesInit
        };
        MovePicker {
            stage,
            tt_move,
            killers: [Move::NONE; 2],
            counter: Move::NONE,
            scored: Vec::with_capacity(32),
            bad_captures: Vec::new(),
            yielded_quiet_stage: [Move::NONE; 3],
            qsearch: true,
            with_checks,
        }
    }

    /// 最大スコアの手を取り出す（部分選択ソート）。
    fn pick_best(&mut self) -> Option<Move> {
        if self.scored.is_empty() {
            return None;
        }
        let mut best = 0;
        for i in 1..self.scored.len() {
            if self.scored[i].1 > self.scored[best].1 {
                best = i;
            }
        }
        Some(self.scored.swap_remove(best).0)
    }

    fn already_yielded(&self, m: Move) -> bool {
        m == self.tt_move || self.yielded_quiet_stage.contains(&m)
    }

    pub fn next(
        &mut self,
        pos: &Position,
        history: &History,
        cont: &ContinuationHistory,
        prev1: Move,
        prev2: Move,
    ) -> Option<Move> {
        loop {
            match self.stage {
                Stage::TtMove => {
                    self.stage = if self.qsearch {
                        Stage::QCapturesInit
                    } else {
                        Stage::CapturesInit
                    };
                    if pos.pseudo_legal(self.tt_move) {
                        return Some(self.tt_move);
                    }
                }
                Stage::CapturesInit => {
                    let mut list = MoveList::default();
                    generate(pos, GenType::Captures, false, &mut list);
                    for &m in &list {
                        if m != self.tt_move {
                            self.scored.push((m, capture_score(pos, m)));
                        }
                    }
                    self.stage = Stage::GoodCaptures;
                }
                Stage::GoodCaptures => match self.pick_best() {
                    Some(m) => {
                        if pos.see_ge(m, 0) {
                            return Some(m);
                        }
                        self.bad_captures.push(m);
                    }
                    None => {
                        self.stage = Stage::Killer(0);
                    }
                },
                Stage::Killer(i) => {
                    let m = self.killers[i];
                    self.stage = if i == 0 {
                        Stage::Killer(1)
                    } else {
                        Stage::Counter
                    };
                    if m != Move::NONE
                        && m != self.tt_move
                        && pos.piece_on(m.to()).is_empty()
                        && pos.pseudo_legal(m)
                    {
                        self.yielded_quiet_stage[i] = m;
                        return Some(m);
                    }
                }
                Stage::Counter => {
                    self.stage = Stage::QuietsInit;
                    let m = self.counter;
                    if m != Move::NONE
                        && !self.already_yielded(m)
                        && pos.piece_on(m.to()).is_empty()
                        && pos.pseudo_legal(m)
                    {
                        self.yielded_quiet_stage[2] = m;
                        return Some(m);
                    }
                }
                Stage::QuietsInit => {
                    let mut list = MoveList::default();
                    generate(pos, GenType::Quiets, false, &mut list);
                    for &m in &list {
                        if !self.already_yielded(m) {
                            let score = history.get(m) + cont.get(prev1, m) + cont.get(prev2, m);
                            self.scored.push((m, score));
                        }
                    }
                    self.stage = Stage::Quiets;
                }
                Stage::Quiets => match self.pick_best() {
                    Some(m) => return Some(m),
                    None => {
                        self.stage = Stage::BadCaptures;
                    }
                },
                Stage::BadCaptures => {
                    return if self.bad_captures.is_empty() {
                        self.stage = Stage::Done;
                        None
                    } else {
                        Some(self.bad_captures.remove(0))
                    };
                }
                Stage::EvasionsInit => {
                    let mut list = MoveList::default();
                    generate(pos, GenType::Evasions, false, &mut list);
                    for &m in &list {
                        let score = if !m.is_drop() && !pos.piece_on(m.to()).is_empty() {
                            100_000 + capture_score(pos, m)
                        } else {
                            history.get(m)
                        };
                        self.scored.push((m, score));
                    }
                    self.stage = Stage::Evasions;
                }
                Stage::Evasions => {
                    return match self.pick_best() {
                        Some(m) => Some(m),
                        None => {
                            self.stage = Stage::Done;
                            None
                        }
                    };
                }
                Stage::QCapturesInit => {
                    let mut list = MoveList::default();
                    generate(pos, GenType::Captures, false, &mut list);
                    for &m in &list {
                        if m != self.tt_move {
                            self.scored.push((m, capture_score(pos, m)));
                        }
                    }
                    self.stage = Stage::QCaptures;
                }
                Stage::QCaptures => match self.pick_best() {
                    Some(m) => {
                        // 損な取り合いは静止探索では捨てる（ADR-0024）
                        if pos.see_ge(m, 0) {
                            return Some(m);
                        }
                    }
                    None => {
                        if self.with_checks {
                            self.stage = Stage::QChecksInit;
                        } else {
                            self.stage = Stage::Done;
                            return None;
                        }
                    }
                },
                Stage::QChecksInit => {
                    let mut list = MoveList::default();
                    generate(pos, GenType::Quiets, false, &mut list);
                    for &m in &list {
                        // 駒損しない静かな王手だけを読む（ADR-0028）。TT手は重複回避
                        if m != self.tt_move && pos.gives_check(m) && pos.see_ge(m, 0) {
                            self.scored.push((m, history.get(m)));
                        }
                    }
                    self.stage = Stage::QChecks;
                }
                Stage::QChecks => {
                    return match self.pick_best() {
                        Some(m) => Some(m),
                        None => {
                            self.stage = Stage::Done;
                            None
                        }
                    };
                }
                Stage::Done => return None,
            }
            if self.stage == Stage::Done && self.qsearch {
                return None;
            }
        }
    }
}
