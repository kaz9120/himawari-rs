//! 基本型: Color / File / Rank / Square（ADR-0008）。
//!
//! 座標は筋優先で `sq = file * 9 + rank`。SQ_11 = 0（盤の右上）、
//! SQ_99 = 80（左下）。方向デルタは先手視点で 下=+1、上=−1、左=+9、右=−9。

/// 手番・駒の先後。
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Color {
    Black = 0,
    White = 1,
}

impl Color {
    pub const NB: usize = 2;

    #[inline]
    pub const fn flip(self) -> Self {
        match self {
            Color::Black => Color::White,
            Color::White => Color::Black,
        }
    }

    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// 筋。0が1筋（盤の右端）、8が9筋（左端）。
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct File(pub u8);

/// 段。0が一段（盤の上端）、8が九段（下端）。
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Rank(pub u8);

impl Rank {
    /// 手番cから見た段（先手はそのまま、後手は反転）。
    #[inline]
    pub const fn relative(self, c: Color) -> Rank {
        match c {
            Color::Black => self,
            Color::White => Rank(8 - self.0),
        }
    }
}

/// マス。0〜80。81は番兵（NONE）。
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Square(u8);

impl Square {
    pub const NB: usize = 81;
    pub const NONE: Square = Square(81);

    #[inline]
    pub const fn new(file: File, rank: Rank) -> Square {
        debug_assert!(file.0 < 9 && rank.0 < 9);
        Square(file.0 * 9 + rank.0)
    }

    #[inline]
    pub const fn from_index(i: u8) -> Square {
        debug_assert!(i < 81);
        Square(i)
    }

    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    #[inline]
    pub const fn file(self) -> File {
        File(self.0 / 9)
    }

    #[inline]
    pub const fn rank(self) -> Rank {
        Rank(self.0 % 9)
    }

    /// 180度回転。
    #[inline]
    pub const fn inv(self) -> Square {
        Square(80 - self.0)
    }

    /// 左右反転。
    #[inline]
    pub const fn mir(self) -> Square {
        Square((8 - self.0 / 9) * 9 + self.0 % 9)
    }

    #[inline]
    pub const fn is_none(self) -> bool {
        self.0 >= 81
    }

    /// USI表記（例: "7g"）。
    pub fn to_usi(self) -> String {
        debug_assert!(!self.is_none());
        format!("{}{}", self.file().0 + 1, char::from(b'a' + self.rank().0))
    }

    /// USI表記からのパース（例: "7g"）。
    pub fn from_usi(s: &str) -> Option<Square> {
        let b = s.as_bytes();
        if b.len() != 2 {
            return None;
        }
        let file = b[0].checked_sub(b'1')?;
        let rank = b[1].checked_sub(b'a')?;
        if file < 9 && rank < 9 {
            Some(Square::new(File(file), Rank(rank)))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_file_rank_roundtrip() {
        for i in 0..81 {
            let sq = Square::from_index(i);
            assert_eq!(Square::new(sq.file(), sq.rank()), sq);
        }
    }

    #[test]
    fn square_layout_is_file_major() {
        // SQ_11 = 0、SQ_19 = 8、SQ_21 = 9、SQ_99 = 80
        assert_eq!(Square::new(File(0), Rank(0)).index(), 0);
        assert_eq!(Square::new(File(0), Rank(8)).index(), 8);
        assert_eq!(Square::new(File(1), Rank(0)).index(), 9);
        assert_eq!(Square::new(File(8), Rank(8)).index(), 80);
    }

    #[test]
    fn square_inv_mir() {
        let sq = Square::new(File(2), Rank(1)); // 3二
        assert_eq!(sq.inv(), Square::new(File(6), Rank(7))); // 7八
        assert_eq!(sq.mir(), Square::new(File(6), Rank(1))); // 7二
        for i in 0..81 {
            let sq = Square::from_index(i);
            assert_eq!(sq.inv().inv(), sq);
            assert_eq!(sq.mir().mir(), sq);
        }
    }

    #[test]
    fn square_usi_roundtrip() {
        assert_eq!(Square::new(File(6), Rank(6)).to_usi(), "7g");
        for i in 0..81 {
            let sq = Square::from_index(i);
            assert_eq!(Square::from_usi(&sq.to_usi()), Some(sq));
        }
        assert_eq!(Square::from_usi("0a"), None);
        assert_eq!(Square::from_usi("1j"), None);
    }

    #[test]
    fn rank_relative() {
        assert_eq!(Rank(2).relative(Color::Black), Rank(2));
        assert_eq!(Rank(2).relative(Color::White), Rank(6));
    }
}
