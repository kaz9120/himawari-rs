//! 探索が手を進め戻す盤面（ADR-0239の手順0）。
//!
//! 探索のワーカーと、盤面操作だけを測る `boardbench` が同じ進め戻しを
//! 呼ぶために1か所へ集める。利きの表のように局面に付いて回る状態を
//! 足すときは、この型に持たせ、`do_move` などへ追従を書く。そうすれば
//! 探索とミクロな計測の両方が同じ費用を払う。
//!
//! 利き数のバイト表（ADR-0239の案B）をここで維持する。局面の読み取りは
//! `Deref` で `Position` のメソッドをそのまま呼べる。進め戻しはこの型の同名のメソッドが先に
//! 解決されるので、包んだ側を素通りしない。`DerefMut` は実装しない。
//! 局面を可変で借りる経路を、この型の進め戻しに限るためである。

use std::ops::Deref;

use himawari_core::{EffectCounts, Move, Piece, Position};

/// 探索が進め戻す盤面。`Position` と、それに追従する状態を持つ。
#[derive(Clone)]
pub struct SearchBoard {
    pos: Position,
    /// 利き数のバイト表の段（ADR-0239の案B）。`counts[ply]` が今の局面の表で、
    /// 進めるときは1つ上の段へ複写してから差分を当て、戻すときは段を下げる
    counts: Vec<EffectCounts>,
    ply: usize,
}

impl SearchBoard {
    pub fn new(pos: Position) -> SearchBoard {
        let mut counts = Vec::with_capacity(Self::INITIAL_DEPTH);
        counts.push(EffectCounts::build(&pos));
        SearchBoard {
            pos,
            counts,
            ply: 0,
        }
    }

    /// 最初に確保する段の数。探索の最大の深さより大きくしておけば伸ばさずに済む
    const INITIAL_DEPTH: usize = 256;

    /// 包んでいる局面。
    #[inline(always)]
    pub fn position(&self) -> &Position {
        &self.pos
    }

    /// 今の局面の利き数の表。
    #[inline(always)]
    pub fn counts(&self) -> &EffectCounts {
        &self.counts[self.ply]
    }

    /// 1手進める。
    #[inline(always)]
    pub fn do_move(&mut self, m: Move) {
        let occ = self.pos.occupied();
        let captured = if m.is_drop() {
            Piece::EMPTY
        } else {
            self.pos.piece_on(m.to())
        };
        self.pos.do_move(m);
        let next = self.ply + 1;
        if next == self.counts.len() {
            self.counts.push(EffectCounts::default());
        }
        let (lo, hi) = self.counts.split_at_mut(next);
        hi[0].copy_from(&lo[self.ply]);
        self.ply = next;
        self.counts[next].update(&self.pos, m, captured, occ);
        debug_assert_eq!(
            self.counts[next],
            EffectCounts::build(&self.pos),
            "利き数の表が作り直しと食い違う: {}",
            m.to_usi()
        );
    }

    /// `do_move` を戻す。表は段を下げるだけで、差分を当て直さない。
    #[inline(always)]
    pub fn undo_move(&mut self, m: Move) {
        self.pos.undo_move(m);
        self.ply -= 1;
        debug_assert_eq!(self.counts[self.ply], EffectCounts::build(&self.pos));
    }

    /// `Position::see_ge` と同じ値を返す。取り合いなしの判定を、地図の代わりに
    /// 表から読む（ADR-0239の案B）。
    #[inline(always)]
    pub fn see_ge(&self, m: Move, threshold: i32) -> bool {
        self.pos.see_ge_by(m, threshold, || {
            let t = self.counts();
            let them = self.pos.side_to_move().flip();
            t.count(them, m.to()) == 0 && (m.is_drop() || t.dirs(them, m.from_sq()) == 0)
        })
    }

    /// `Position::is_legal` と同じ値を返す。玉の行き先を表から読む。
    #[inline(always)]
    pub fn is_legal(&self, m: Move) -> bool {
        self.pos.is_legal_by(m, |to| {
            self.counts().count(self.pos.side_to_move().flip(), to) == 0
        })
    }

    /// `mate_1ply` と同じ手を返す。候補を表から読んで落とす。
    #[inline(always)]
    pub fn mate_1ply(&self) -> Option<Move> {
        crate::mate::mate_1ply_with_counts(&self.pos, self.counts())
    }

    /// null moveで1手進める。盤面が変わらないので表はそのまま使う。
    #[inline(always)]
    pub fn do_null_move(&mut self) {
        self.pos.do_null_move();
    }

    /// `do_null_move` を戻す。
    #[inline(always)]
    pub fn undo_null_move(&mut self) {
        self.pos.undo_null_move();
    }
}

impl Deref for SearchBoard {
    type Target = Position;

    #[inline(always)]
    fn deref(&self) -> &Position {
        &self.pos
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use himawari_core::{GenType, MoveList, SFEN_STARTPOS, generate, generate_legal};

    /// 表から読む `see_ge`・`is_legal`・`mate_1ply` が、`Position` 側と同じ値を返す。
    fn walk(b: &mut SearchBoard, depth: u32) {
        if !b.in_check() {
            assert_eq!(b.mate_1ply(), crate::mate::mate_1ply(b.position()));
            let mut pseudo = MoveList::default();
            generate(b, GenType::NonEvasions, false, &mut pseudo);
            for &m in pseudo.as_slice() {
                assert_eq!(b.is_legal(m), b.position().is_legal(m), "{}", m.to_usi());
            }
        }
        let mut list = MoveList::default();
        generate_legal(b, true, &mut list);
        for &m in list.as_slice() {
            for th in [-90, 0, 90] {
                assert_eq!(
                    b.see_ge(m, th),
                    b.position().see_ge(m, th),
                    "{}",
                    m.to_usi()
                );
            }
            if depth > 0 {
                b.do_move(m);
                walk(b, depth - 1);
                b.undo_move(m);
            }
        }
    }

    #[test]
    fn table_reads_match_position() {
        for sfen in [
            SFEN_STARTPOS,
            "l6nl/5+P1gk/2np1S3/p1p4Pp/3P2Sp1/1PPb2P1P/P5GS1/R8/LN4bKL w GR5pnsg 1",
            "+R1G4nl/1g4+Ss1/1kspp2p1/ppp2pS1p/4n4/P4Gp1P/1P1PP1P2/1+n2K2R1/7NL w G2P2b2lp 24",
        ] {
            let mut b = SearchBoard::new(Position::from_sfen(sfen).expect("sfen"));
            walk(&mut b, 1);
        }
    }
}
