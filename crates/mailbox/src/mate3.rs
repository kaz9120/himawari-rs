//! 3手詰めの判定。王手を指し、玉側のどの応手にも1手詰めが残るかを手を進めて調べる。
//!
//! 1手詰めの判定（`mate_1ply`）は候補を玉の近くに絞るので、詰みを見落とすことはあっても、
//! 詰みでない手を詰みとは言わない。その上に組む3手詰めも、見落としはあるが誤りは返さない。

use crate::movegen::{GenType, MoveList};
use crate::moves::Move;
use crate::position::Position;

impl Position {
    /// 3手以内の詰みがあれば初手を返す。王手されていない局面で呼ぶ。1手詰めは先に調べる。
    pub fn mate_3ply(&mut self) -> Option<Move> {
        debug_assert!(!self.in_check());
        if let Some(m) = self.mate_1ply() {
            return Some(m);
        }
        let mut moves = MoveList::default();
        self.generate_checks(false, &mut moves);
        for &m in moves.as_slice() {
            self.do_move(m);
            let mated = self.all_evasions_mated();
            self.undo_move(m);
            if mated {
                return Some(m);
            }
        }
        None
    }

    /// 王手された手番側の、どの応手の後にも相手に1手詰めがあるか。応手が無ければ詰みである。
    fn all_evasions_mated(&mut self) -> bool {
        let mut evasions = MoveList::default();
        self.generate(GenType::Evasions, false, &mut evasions);
        for &e in evasions.as_slice() {
            self.do_move(e);
            // 応手が逆王手なら1手詰めの判定は使えない。詰みとはみなさない
            let mated = !self.in_check() && self.mate_1ply().is_some();
            self.undo_move(e);
            if !mated {
                return false;
            }
        }
        true
    }
}
