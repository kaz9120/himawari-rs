//! 指し手オーダリング（ADR-0025）。
//!
//! MovePickerは段階生成の状態機械。カットが早いノードでは
//! Quietsの生成自体を省く。borrow衝突を避けるため、
//! next()は毎回&Positionと&Historyを受け取る。

use himawari_core::{
    Color, GenType, Move, MoveList, Piece, Position, Square, generate, piece_value,
};

// ---- テーブルの寸法 ----

/// 駒の種類数（先後込み）。参照実装のPIECE_NB。
const PIECE_NB: usize = 32;
/// マスの数。参照実装のSQUARE_NB。
const SQUARE_NB: usize = 81;
/// correction historyのスロット数（history.h:29）。
const CORRHIST_BASE_SIZE: usize = 65536;
/// correction historyの値域（history.h:30）。
const D_CORRECTION: i32 = 1024;
/// continuation correction historyの初期値（yaneuraou-search.cpp:2153）。
const INIT_CONT_CORR: i16 = 6;
/// 応手側の面の広さ（[駒][マス]）。
const CONT_STRIDE: usize = PIECE_NB * SQUARE_NB;

/// gravity方式の更新（history.h:91-125のStatsEntry::operator<<）。
/// bonusを±dに丸めたうえで、値が±dを超えないよう自然にゼロへ引き戻す。
#[inline]
fn stats_update(entry: &mut i16, bonus: i32, d: i32) {
    let b = bonus.clamp(-d, d);
    let v = i32::from(*entry);
    *entry = (v + b - v * b.abs() / d) as i16;
}

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

/// 静的評価の系統誤差を補正する履歴（ADR-0046, 0109）。
///
/// 参照実装のUnifiedCorrectionHistory（history.h:337-339）に対応する。
/// 1本の表を4系統（歩・小駒・先手非歩・後手非歩）で共有し、系統ごとに
/// 別のキーで引く。添字は `[キー下位16bit][手番][系統]`。
pub struct CorrectionHistory {
    table: Box<[i16]>,
}

/// correction historyの系統（history.h:296-309のCorrectionBundle）。
const CORR_PAWN: usize = 0;
const CORR_MINOR: usize = 1;
const CORR_NON_PAWN_BLACK: usize = 2;
const CORR_NON_PAWN_WHITE: usize = 3;
/// 1スロットが持つ系統数。
const CORR_KINDS: usize = 4;

impl Default for CorrectionHistory {
    fn default() -> Self {
        CorrectionHistory {
            table: vec![0i16; CORRHIST_BASE_SIZE * 2 * CORR_KINDS].into_boxed_slice(),
        }
    }
}

impl CorrectionHistory {
    #[inline]
    fn index(key: u64, stm: usize, kind: usize) -> usize {
        ((key as usize & (CORRHIST_BASE_SIZE - 1)) * 2 + stm) * CORR_KINDS + kind
    }

    #[inline]
    fn get(&self, key: u64, stm: usize, kind: usize) -> i32 {
        i32::from(self.table[Self::index(key, stm, kind)])
    }

    fn update(&mut self, key: u64, stm: usize, kind: usize, bonus: i32) {
        stats_update(
            &mut self.table[Self::index(key, stm, kind)],
            bonus,
            D_CORRECTION,
        );
    }

    /// 4系統の生の値を取り出す（yaneuraou-search.cpp:728-731）。
    /// 返り値は (歩, 小駒, 先手非歩, 後手非歩)。
    #[inline]
    pub fn probe(&self, pos: &Position) -> (i32, i32, i32, i32) {
        let stm = pos.side_to_move().index();
        (
            self.get(pos.pawn_key(), stm, CORR_PAWN),
            self.get(pos.minor_piece_key(), stm, CORR_MINOR),
            self.get(pos.non_pawn_key(Color::Black), stm, CORR_NON_PAWN_BLACK),
            self.get(pos.non_pawn_key(Color::White), stm, CORR_NON_PAWN_WHITE),
        )
    }

    /// 4系統をまとめて更新する（yaneuraou-search.cpp:759-762）。
    /// 系統ごとの重みも参照実装のものを使う。
    pub fn update_all(&mut self, pos: &Position, bonus: i32) {
        /// 非歩系統の重み（yaneuraou-search.cpp:755）。
        const NON_PAWN_WEIGHT: i32 = 187;
        let stm = pos.side_to_move().index();
        self.update(pos.pawn_key(), stm, CORR_PAWN, bonus);
        self.update(pos.minor_piece_key(), stm, CORR_MINOR, bonus * 153 / 128);
        self.update(
            pos.non_pawn_key(Color::Black),
            stm,
            CORR_NON_PAWN_BLACK,
            bonus * NON_PAWN_WEIGHT / 128,
        );
        self.update(
            pos.non_pawn_key(Color::White),
            stm,
            CORR_NON_PAWN_WHITE,
            bonus * NON_PAWN_WEIGHT / 128,
        );
    }

    pub fn clear(&mut self) {
        self.table.fill(0);
    }
}

/// continuation correction history（ADR-0085, 0109）。
/// 論理次元は[条件手の駒 32][条件手の移動先 81][駒 32][マス 81]
/// （history.h:325-327）。条件手の側は2手前・4手前を見る。
pub struct ContinuationCorrectionHistory {
    table: Box<[i16]>,
}

impl Default for ContinuationCorrectionHistory {
    fn default() -> Self {
        ContinuationCorrectionHistory {
            table: vec![INIT_CONT_CORR; CONT_STRIDE * CONT_STRIDE].into_boxed_slice(),
        }
    }
}

impl ContinuationCorrectionHistory {
    /// 条件手から面の先頭添字を作る。指し手がないplyは番兵の面（先頭）を指す
    /// （yaneuraou-search.cpp:1469, 3256）。
    #[inline]
    pub fn base(m: Move) -> usize {
        if m.is_special() {
            return 0;
        }
        (m.piece_after().index() * SQUARE_NB + m.to().index()) * CONT_STRIDE
    }

    #[inline]
    pub fn get(&self, base: usize, pc: Piece, to: Square) -> i32 {
        i32::from(self.table[base + pc.index() * SQUARE_NB + to.index()])
    }

    pub fn update(&mut self, base: usize, pc: Piece, to: Square, bonus: i32) {
        stats_update(
            &mut self.table[base + pc.index() * SQUARE_NB + to.index()],
            bonus,
            D_CORRECTION,
        );
    }

    pub fn clear(&mut self) {
        self.table.fill(INIT_CONT_CORR);
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

/// 最大値の位置を返す。同点なら最小の添字を選ぶ（ADR-0100）。
///
/// 素直な線形走査と同じ結果を返す。前半で最大値をSIMDで求め、後半で
/// その値が最初に現れる位置を探す。どちらも1要素ずつの比較より
/// 8倍幅で進むため、走査は2回になっても速い。
///
/// `scores` が空のときは呼べない。
#[inline]
fn argmax_first(scores: &[i32]) -> usize {
    use std::simd::Simd;
    use std::simd::cmp::{SimdOrd, SimdPartialEq};
    use std::simd::num::SimdInt;

    const LANES: usize = 8;
    debug_assert!(!scores.is_empty());
    let (chunks, rest) = scores.as_chunks::<LANES>();

    let mut vmax = Simd::<i32, LANES>::splat(i32::MIN);
    for c in chunks {
        vmax = vmax.simd_max(Simd::from_array(*c));
    }
    let mut best = vmax.reduce_max();
    for &v in rest {
        if v > best {
            best = v;
        }
    }

    let target = Simd::<i32, LANES>::splat(best);
    for (ci, c) in chunks.iter().enumerate() {
        let bits = Simd::from_array(*c).simd_eq(target).to_bitmask();
        if bits != 0 {
            return ci * LANES + bits.trailing_zeros() as usize;
        }
    }
    let tail = rest
        .iter()
        .position(|&v| v == best)
        .expect("最大値はscoresのどこかにある");
    chunks.len() * LANES + tail
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
    /// 採点済みの手。スコアは `scores` の同じ添字に持つ（ADR-0100）
    moves: Vec<Move>,
    scores: Vec<i32>,
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
            moves: Vec::with_capacity(64),
            scores: Vec::with_capacity(64),
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
            moves: Vec::with_capacity(32),
            scores: Vec::with_capacity(32),
            bad_captures: Vec::new(),
            yielded_quiet_stage: [Move::NONE; 3],
            qsearch: true,
            with_checks,
        }
    }

    /// 採点済みの手を1つ積む。指し手とスコアは同じ添字で対応する。
    #[inline]
    fn push_scored(&mut self, m: Move, score: i32) {
        self.moves.push(m);
        self.scores.push(score);
    }

    /// 最大スコアの手を取り出す（部分選択ソート）。
    fn pick_best(&mut self) -> Option<Move> {
        if self.scores.is_empty() {
            return None;
        }
        let best = argmax_first(&self.scores);
        self.scores.swap_remove(best);
        Some(self.moves.swap_remove(best))
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
                            self.push_scored(m, capture_score(pos, m));
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
                            self.push_scored(m, score);
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
                        self.push_scored(m, score);
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
                            self.push_scored(m, capture_score(pos, m));
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
                            self.push_scored(m, history.get(m));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 最大値の位置が素直な線形走査と一致すること（ADR-0100）。
    /// 同点は最小の添字を選ぶ。SIMDの幅で割り切れない長さも通す。
    #[test]
    fn argmax_first_matches_linear_scan() {
        fn linear(s: &[i32]) -> usize {
            let mut best = 0;
            for i in 1..s.len() {
                if s[i] > s[best] {
                    best = i;
                }
            }
            best
        }

        let mut x = 0x1234_5678u64;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for len in 1..40usize {
            for trial in 0..50 {
                // 値域を狭くして同点を多く作る。同点の扱いが要点のため
                let range = if trial % 2 == 0 { 5 } else { 1_000_000 };
                let v: Vec<i32> = (0..len).map(|_| (next() % range) as i32 - 2).collect();
                assert_eq!(argmax_first(&v), linear(&v), "len={len} v={v:?}");
            }
        }

        assert_eq!(argmax_first(&[i32::MIN]), 0);
        assert_eq!(argmax_first(&[-5, -5, -5]), 0);
        assert_eq!(argmax_first(&[i32::MIN, i32::MAX]), 1);
    }
}
