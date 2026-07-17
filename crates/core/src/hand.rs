//! 手駒のパック表現（ADR-0013）。
//!
//! u32に7駒種をborrowガード付きで詰める。優等判定は減算1回＋AND1回。

use crate::piece::PieceType;

#[derive(Copy, Clone, PartialEq, Eq, Default, Debug)]
pub struct Hand(pub u32);

/// 駒種（OSL配列 9〜15）→ シフト量。それ以外の添字は使わない。
const SHIFT: [u32; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 18, 0, 6, 10, 14, 22, 25];

/// 駒種 → フィールドのビット幅マスク（シフト前）。
const FIELD: [u32; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 7, 31, 7, 7, 7, 3, 3];

/// 各フィールド直上のガードビット。
const BORROW_MASK: u32 =
    (1 << 5) | (1 << 9) | (1 << 13) | (1 << 17) | (1 << 21) | (1 << 24) | (1 << 27);

impl Hand {
    pub const EMPTY: Hand = Hand(0);

    #[inline]
    const fn shift(pt: PieceType) -> u32 {
        SHIFT[pt.index()]
    }

    #[inline]
    pub const fn count(self, pt: PieceType) -> u32 {
        (self.0 >> Self::shift(pt)) & FIELD[pt.index()]
    }

    #[inline]
    pub const fn has(self, pt: PieceType) -> bool {
        self.0 & (FIELD[pt.index()] << Self::shift(pt)) != 0
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[inline]
    pub fn add(&mut self, pt: PieceType) {
        self.0 += 1 << Self::shift(pt);
        debug_assert!(self.0 & BORROW_MASK == 0);
    }

    #[inline]
    pub fn sub(&mut self, pt: PieceType) {
        debug_assert!(self.has(pt));
        self.0 -= 1 << Self::shift(pt);
    }

    /// 全駒種で self ≥ other か（優等判定）。盤面一致の確認は呼び出し側。
    #[inline]
    pub const fn is_superior_or_equal(self, other: Hand) -> bool {
        self.0.wrapping_sub(other.0) & BORROW_MASK == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_sub_count() {
        let mut h = Hand::EMPTY;
        for _ in 0..18 {
            h.add(PieceType::PAWN);
        }
        h.add(PieceType::ROOK);
        h.add(PieceType::GOLD);
        assert_eq!(h.count(PieceType::PAWN), 18);
        assert_eq!(h.count(PieceType::ROOK), 1);
        assert_eq!(h.count(PieceType::GOLD), 1);
        assert_eq!(h.count(PieceType::BISHOP), 0);
        assert!(h.has(PieceType::PAWN));
        assert!(!h.has(PieceType::BISHOP));
        h.sub(PieceType::ROOK);
        assert!(!h.has(PieceType::ROOK));
    }

    #[test]
    fn max_counts_do_not_touch_guards() {
        // 全駒種を最大枚数まで積んでもガードビットが立たない
        let mut h = Hand::EMPTY;
        let maxes = [
            (PieceType::PAWN, 18),
            (PieceType::LANCE, 4),
            (PieceType::KNIGHT, 4),
            (PieceType::SILVER, 4),
            (PieceType::GOLD, 4),
            (PieceType::BISHOP, 2),
            (PieceType::ROOK, 2),
        ];
        for (pt, n) in maxes {
            for _ in 0..n {
                h.add(pt);
            }
        }
        for (pt, n) in maxes {
            assert_eq!(h.count(pt), n);
        }
    }

    #[test]
    fn superiority() {
        let mut a = Hand::EMPTY;
        let mut b = Hand::EMPTY;
        a.add(PieceType::PAWN);
        a.add(PieceType::GOLD);
        b.add(PieceType::PAWN);
        assert!(a.is_superior_or_equal(b));
        assert!(!b.is_superior_or_equal(a));
        assert!(a.is_superior_or_equal(a));

        // 駒種違いの交換は互いに優等でない
        let mut c = Hand::EMPTY;
        c.add(PieceType::ROOK);
        assert!(!b.is_superior_or_equal(c));
        assert!(!c.is_superior_or_equal(b));
    }

    #[test]
    fn hand_kinds_cover_all_fields() {
        // HAND_KINDSの7種のシフトが互いに重ならない
        let mut h = Hand::EMPTY;
        for pt in PieceType::HAND_KINDS {
            h.add(pt);
        }
        for pt in PieceType::HAND_KINDS {
            assert_eq!(h.count(pt), 1);
        }
    }
}
