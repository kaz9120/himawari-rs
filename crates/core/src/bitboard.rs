//! Bitboard（ADR-0010、u128単一ワード）。
//!
//! bit i = Square i。bit 81〜127は常に0を維持する。
//! ビット走査は64bitワード2本に分割して回す（ADR-0010の実装規約）。

use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Not};

use crate::types::{Color, File, Rank, Square};

#[derive(Copy, Clone, PartialEq, Eq, Default, Debug)]
pub struct Bitboard(pub(crate) u128);

/// 盤上81マスすべて。
const BOARD: u128 = (1u128 << 81) - 1;

const fn rank_mask(rank: u8) -> u128 {
    let mut m = 0u128;
    let mut f = 0;
    while f < 9 {
        m |= 1u128 << (f * 9 + rank);
        f += 1;
    }
    m
}

const fn file_mask(file: u8) -> u128 {
    0x1FFu128 << (file * 9)
}

impl Bitboard {
    pub const EMPTY: Bitboard = Bitboard(0);
    pub const ALL: Bitboard = Bitboard(BOARD);

    #[inline]
    pub const fn from_square(sq: Square) -> Bitboard {
        Bitboard(1u128 << sq.index())
    }

    #[inline]
    pub const fn file(f: File) -> Bitboard {
        Bitboard(file_mask(f.0))
    }

    #[inline]
    pub const fn rank(r: Rank) -> Bitboard {
        Bitboard(rank_mask(r.0))
    }

    /// 手番cから見た敵陣（1〜3段目）。
    #[inline]
    pub const fn promotion_zone(c: Color) -> Bitboard {
        const BLACK_ZONE: u128 = {
            let mut m = 0u128;
            let mut r = 0;
            while r < 3 {
                m |= rank_mask(r);
                r += 1;
            }
            m
        };
        match c {
            Color::Black => Bitboard(BLACK_ZONE),
            Color::White => {
                let mut m = 0u128;
                let mut r = 6;
                while r < 9 {
                    m |= rank_mask(r);
                    r += 1;
                }
                Bitboard(m)
            }
        }
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[inline]
    pub const fn test(self, sq: Square) -> bool {
        self.0 & (1u128 << sq.index()) != 0
    }

    #[inline]
    pub fn set(&mut self, sq: Square) {
        self.0 |= 1u128 << sq.index();
    }

    #[inline]
    pub fn clear(&mut self, sq: Square) {
        self.0 &= !(1u128 << sq.index());
    }

    #[inline]
    pub const fn count(self) -> u32 {
        self.0.count_ones()
    }

    #[inline]
    pub const fn more_than_one(self) -> bool {
        self.0 & self.0.wrapping_sub(1) != 0
    }

    /// 最下位ビットのマス。空ならNONE相当ではなくdebug_assertで防御。
    #[inline]
    pub const fn lsb(self) -> Square {
        debug_assert!(!self.is_empty());
        Square::from_index(self.0.trailing_zeros() as u8)
    }

    #[inline]
    pub fn pop_lsb(&mut self) -> Square {
        let sq = self.lsb();
        self.0 &= self.0.wrapping_sub(1);
        sq
    }

    /// 段方向+1（先手視点で下）へのシフト。
    #[inline]
    pub const fn shift_down(self) -> Bitboard {
        const OK: u128 = {
            let m = rank_mask(0);
            BOARD & !m
        };
        Bitboard((self.0 << 1) & OK)
    }

    /// 段方向−1（先手視点で上）へのシフト。
    #[inline]
    pub const fn shift_up(self) -> Bitboard {
        const OK: u128 = {
            let m = rank_mask(8);
            BOARD & !m
        };
        Bitboard((self.0 >> 1) & OK)
    }

    /// 内部値（クレート内部の演算用）。
    #[inline]
    pub(crate) const fn raw(self) -> u128 {
        self.0
    }
}

impl BitAnd for Bitboard {
    type Output = Bitboard;
    #[inline]
    fn bitand(self, rhs: Bitboard) -> Bitboard {
        Bitboard(self.0 & rhs.0)
    }
}

impl BitOr for Bitboard {
    type Output = Bitboard;
    #[inline]
    fn bitor(self, rhs: Bitboard) -> Bitboard {
        Bitboard(self.0 | rhs.0)
    }
}

impl BitXor for Bitboard {
    type Output = Bitboard;
    #[inline]
    fn bitxor(self, rhs: Bitboard) -> Bitboard {
        Bitboard(self.0 ^ rhs.0)
    }
}

impl Not for Bitboard {
    type Output = Bitboard;
    #[inline]
    fn not(self) -> Bitboard {
        Bitboard(!self.0 & BOARD)
    }
}

impl BitAndAssign for Bitboard {
    #[inline]
    fn bitand_assign(&mut self, rhs: Bitboard) {
        self.0 &= rhs.0;
    }
}

impl BitOrAssign for Bitboard {
    #[inline]
    fn bitor_assign(&mut self, rhs: Bitboard) {
        self.0 |= rhs.0;
    }
}

impl BitXorAssign for Bitboard {
    #[inline]
    fn bitxor_assign(&mut self, rhs: Bitboard) {
        self.0 ^= rhs.0;
    }
}

/// ビット走査イテレータ。ADR-0010の規約どおり64bitワード2本に分割して回す。
pub struct SquareIter {
    lo: u64,
    hi: u64,
}

impl Iterator for SquareIter {
    type Item = Square;

    #[inline]
    fn next(&mut self) -> Option<Square> {
        if self.lo != 0 {
            let sq = Square::from_index(self.lo.trailing_zeros() as u8);
            self.lo &= self.lo.wrapping_sub(1);
            Some(sq)
        } else if self.hi != 0 {
            let sq = Square::from_index(64 + self.hi.trailing_zeros() as u8);
            self.hi &= self.hi.wrapping_sub(1);
            Some(sq)
        } else {
            None
        }
    }
}

impl IntoIterator for Bitboard {
    type Item = Square;
    type IntoIter = SquareIter;

    #[inline]
    fn into_iter(self) -> SquareIter {
        SquareIter {
            lo: self.0 as u64,
            hi: (self.0 >> 64) as u64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_test_clear() {
        let mut bb = Bitboard::EMPTY;
        let sq = Square::new(File(6), Rank(6));
        bb.set(sq);
        assert!(bb.test(sq));
        assert_eq!(bb.count(), 1);
        assert_eq!(bb.lsb(), sq);
        bb.clear(sq);
        assert!(bb.is_empty());
    }

    #[test]
    fn shift_down_up() {
        let sq = Square::new(File(4), Rank(4));
        let bb = Bitboard::from_square(sq);
        assert!(bb.shift_down().test(Square::new(File(4), Rank(5))));
        assert!(bb.shift_up().test(Square::new(File(4), Rank(3))));
        // 端の段からのシフトは盤外に消え、隣の筋へ漏れない
        let bottom = Bitboard::from_square(Square::new(File(4), Rank(8)));
        assert!(bottom.shift_down().is_empty());
        let top = Bitboard::from_square(Square::new(File(4), Rank(0)));
        assert!(top.shift_up().is_empty());
    }

    #[test]
    fn iterate_across_word_boundary() {
        // bit 63前後をまたぐ走査（lo/hi分割の境界確認）
        let mut bb = Bitboard::EMPTY;
        let squares = [0u8, 62, 63, 64, 80];
        for &i in &squares {
            bb.set(Square::from_index(i));
        }
        let collected: Vec<usize> = bb.into_iter().map(Square::index).collect();
        assert_eq!(collected, vec![0, 62, 63, 64, 80]);
    }

    #[test]
    fn promotion_zone() {
        assert!(Bitboard::promotion_zone(Color::Black).test(Square::new(File(4), Rank(2))));
        assert!(!Bitboard::promotion_zone(Color::Black).test(Square::new(File(4), Rank(3))));
        assert!(Bitboard::promotion_zone(Color::White).test(Square::new(File(4), Rank(6))));
        assert_eq!(Bitboard::promotion_zone(Color::Black).count(), 27);
    }

    #[test]
    fn file_rank_masks() {
        assert_eq!(Bitboard::file(File(0)).count(), 9);
        assert_eq!(Bitboard::rank(Rank(0)).count(), 9);
        for i in 0..81 {
            let sq = Square::from_index(i);
            assert!(Bitboard::file(sq.file()).test(sq));
            assert!(Bitboard::rank(sq.rank()).test(sq));
        }
    }
}
