//! 探索が局面に問う規則と、評価関数の特徴の材料（ADR-0241の第13回）。
//!
//! 千日手・入玉宣言・`check_squares`・駒種ごとの升の集合を持つ。どれもcoreの同名の関数と同じ値を返す
//! （照合のテストで確かめる）。千日手は過去の局面のキーを要るので、探索の根の局面を作るときに
//! `import_history` でcoreの過去の局面のキーを写す。

use crate::board::*;
use crate::position::{Position, StateInfo};

/// 千日手の分類。coreの `Repetition` と同じ並び。WinとLoseは連続王手の千日手。
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Repetition {
    None,
    Draw,
    Win,
    Lose,
    Superior,
    Inferior,
}

/// 探索の千日手の走査で遡る手数の上限（coreの `REPETITION_SCAN_MAX` と同じ）。
pub const REPETITION_SCAN_MAX: usize = 16;

/// 手駒の詰め方で、各駒種の欄の1つ上のビット（coreの `Hand` の借りの検出と同じ）。
const BORROW_MASK: u32 =
    (1 << 5) | (1 << 9) | (1 << 13) | (1 << 17) | (1 << 21) | (1 << 24) | (1 << 27);

/// 千日手の判定に要る、過去の局面1つ分の値。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryKey {
    pub board_key: u64,
    pub hand_key: u64,
    pub continuous_check: [u16; 2],
    pub plies_from_null: u16,
}

/// 入玉宣言の点数（飛角馬龍5点、ほかは1点）。
#[inline]
const fn declaration_points(pt: u8) -> u32 {
    match pt {
        BISHOP | ROOK | HORSE | DRAGON => 5,
        _ => 1,
    }
}

impl Position {
    /// 過去の局面のキーを、状態の積み上げの下へ写す。`hist` の最後が今の局面である。
    ///
    /// 写した過去の局面は千日手の走査だけが読む。根より前へは戻さないので、取った駒などは持たない。
    /// 今の局面の連続王手の数とnull moveからの手数も、`hist` の最後の値に揃える。
    pub fn import_history(&mut self, hist: &[HistoryKey]) {
        let Some((cur, past)) = hist.split_last() else {
            return;
        };
        let mut now = *self.state();
        debug_assert_eq!(now.board_key, cur.board_key, "今の局面のキーが合わない");
        debug_assert_eq!(now.hand_key, cur.hand_key, "今の局面の手駒のキーが合わない");
        now.continuous_check = cur.continuous_check;
        now.plies_from_null = cur.plies_from_null;
        self.states.clear();
        self.check_slots.clear();
        for h in past {
            let st = StateInfo {
                board_key: h.board_key,
                hand_key: h.hand_key,
                continuous_check: h.continuous_check,
                plies_from_null: h.plies_from_null,
                ..StateInfo::default()
            };
            self.push_state(st);
        }
        self.push_state(now);
    }

    /// 千日手・優等局面の判定（coreの `repetition_state` と同じ値）。
    #[inline]
    pub fn repetition_state(&self, ply: usize) -> Repetition {
        self.scan_repetition(ply, REPETITION_SCAN_MAX)
    }

    /// 走査の本体。`scan_max` 手まで遡って最初の一致を返す。
    fn scan_repetition(&self, ply: usize, scan_max: usize) -> Repetition {
        let cur = self.states.len() - 1;
        let st = &self.states[cur];
        let us = self.side.index();
        let them = 1 - us;
        let limit = (st.plies_from_null as usize).min(cur).min(scan_max);
        let mut i = 4;
        while i <= limit {
            let prev = &self.states[cur - i];
            if prev.board_key == st.board_key {
                if prev.hand_key == st.hand_key {
                    if st.continuous_check[them] as usize * 2 >= i {
                        return Repetition::Win;
                    }
                    if st.continuous_check[us] as usize * 2 >= i {
                        return Repetition::Lose;
                    }
                    return Repetition::Draw;
                }
                // 根を跨ぐ優等・劣等は判定しない。走査は続ける
                if i < ply {
                    let my_now = (st.hand_key >> (32 * us)) as u32;
                    let my_then = (prev.hand_key >> (32 * us)) as u32;
                    if my_now.wrapping_sub(my_then) & BORROW_MASK == 0 {
                        return Repetition::Superior;
                    }
                    if my_then.wrapping_sub(my_now) & BORROW_MASK == 0 {
                        return Repetition::Inferior;
                    }
                }
            }
            i += 2;
        }
        Repetition::None
    }

    /// 入玉宣言勝ち（27点法）が成立するか（coreの `can_declare_win` と同じ値）。
    pub fn can_declare_win(&self) -> bool {
        let us = self.side;
        let ui = us.index();
        if rel_rank(us, self.king[ui]) > 2 || self.in_check() {
            return false;
        }
        let mut count = 0u32;
        let mut points = 0u32;
        let mut m = self.on_board[ui];
        while m != 0 {
            let id = m.trailing_zeros() as usize;
            m &= m - 1;
            let s = self.sq_of[id];
            let pt = piece_type(self.board[s as usize]);
            if pt == KING || rel_rank(us, s) > 2 {
                continue;
            }
            count += 1;
            points += declaration_points(pt);
        }
        if count < 10 {
            return false;
        }
        for pt in HAND_KINDS {
            points += self.hand_count(us, pt) * declaration_points(pt);
        }
        let need = if us == Color::Black { 28 } else { 27 };
        points >= need
    }

    /// 手番側の駒種ptの駒が置けば相手玉へ直接王手になる升（coreの `check_squares` と同じ集合）。
    ///
    /// 相手玉の升から、相手の駒種ptとして利きを引いた集合である。飛び利きは、ノードごとに持つ
    /// 王手の手がかり（玉から8方向の最初の駒までの線）をそのまま使う。
    pub fn check_squares(&self, pt: u8) -> u128 {
        let them = self.side.flip();
        let p = make_piece(them, pt);
        let ksq = self.king[them.index()];
        let m = &MOVES[(p & 31) as usize];
        let mut out = 0u128;
        for k in 0..m.n_short as usize {
            let t = (ksq as i32 + m.short[k] as i32) as u8;
            if self.board[t as usize] != WALL {
                out |= bit81(t);
            }
        }
        let mut dirs = m.long;
        if dirs != 0 {
            let rays = self.check_rays();
            while dirs != 0 {
                let d = dirs.trailing_zeros() as usize;
                dirs &= dirs - 1;
                out |= rays[d];
            }
        }
        out
    }

    /// 持ち主ごと・駒の符号の駒種ごとの、盤上の駒の升の集合（81升の通し番号のビット）。
    ///
    /// 評価関数の駒の特徴（HalfKP）を作り直すときの材料である。盤を書き換えるたびに差分で持つ集合を返す。
    /// 添字は `[持ち主][駒種]` である。駒種1の欄は金の動きの5駒種（金と成駒4種）の和で、駒種0の欄は常に空である。
    #[inline]
    pub fn piece_sets(&self) -> &[[u128; 16]; 2] {
        &self.psets
    }
}
