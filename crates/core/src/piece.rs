//! 駒のエンコーディング（ADR-0009、GPS将棋/OSL式）。
//!
//! 成駒を下位（2〜7）、生駒を上位（8〜15）に置く。bit 3の意味が
//! 「生駒である」として全値で一貫し、成り系の演算に例外がない。

use crate::types::Color;

/// 駒種（先後なし）。0 = 空、1 = 予約（旧EDGE）。
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct PieceType(pub u8);

impl PieceType {
    pub const NB: usize = 16;

    pub const EMPTY: PieceType = PieceType(0);
    pub const PRO_PAWN: PieceType = PieceType(2);
    pub const PRO_LANCE: PieceType = PieceType(3);
    pub const PRO_KNIGHT: PieceType = PieceType(4);
    pub const PRO_SILVER: PieceType = PieceType(5);
    pub const HORSE: PieceType = PieceType(6);
    pub const DRAGON: PieceType = PieceType(7);
    pub const KING: PieceType = PieceType(8);
    pub const GOLD: PieceType = PieceType(9);
    pub const PAWN: PieceType = PieceType(10);
    pub const LANCE: PieceType = PieceType(11);
    pub const KNIGHT: PieceType = PieceType(12);
    pub const SILVER: PieceType = PieceType(13);
    pub const BISHOP: PieceType = PieceType(14);
    pub const ROOK: PieceType = PieceType(15);

    /// 手駒に持てる駒種（駒打ち生成のループ順）。
    pub const HAND_KINDS: [PieceType; 7] = [
        PieceType::GOLD,
        PieceType::PAWN,
        PieceType::LANCE,
        PieceType::KNIGHT,
        PieceType::SILVER,
        PieceType::BISHOP,
        PieceType::ROOK,
    ];

    /// 金の動きをする駒（金・と・成香・成桂・成銀）の集合マスク。
    const GOLDS_MASK: u16 = (1 << 9) | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5);

    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    #[inline]
    pub const fn is_piece(self) -> bool {
        self.0 >= 2
    }

    /// 成駒か。
    #[inline]
    pub const fn is_promoted(self) -> bool {
        debug_assert!(self.is_piece());
        self.0 < 8
    }

    /// 成れる駒か（成駒・金・玉はfalse）。
    #[inline]
    pub const fn can_promote(self) -> bool {
        debug_assert!(self.is_piece());
        self.0 > 9
    }

    /// 成る。can_promoteが前提。
    #[inline]
    pub const fn promote(self) -> PieceType {
        debug_assert!(self.can_promote());
        PieceType(self.0 - 8)
    }

    /// 生駒に戻す。全駒に適用できる全域関数（玉・金は無変更）。
    #[inline]
    pub const fn unpromote(self) -> PieceType {
        debug_assert!(self.is_piece());
        PieceType(self.0 | 8)
    }

    /// 金の動きをする駒か。
    #[inline]
    pub const fn is_gold_like(self) -> bool {
        Self::GOLDS_MASK & (1 << self.0) != 0
    }

    /// SFEN文字（大文字）。空・予約はNone。
    pub fn to_sfen(self) -> Option<&'static str> {
        Some(match self {
            PieceType::PAWN => "P",
            PieceType::LANCE => "L",
            PieceType::KNIGHT => "N",
            PieceType::SILVER => "S",
            PieceType::GOLD => "G",
            PieceType::BISHOP => "B",
            PieceType::ROOK => "R",
            PieceType::KING => "K",
            PieceType::PRO_PAWN => "+P",
            PieceType::PRO_LANCE => "+L",
            PieceType::PRO_KNIGHT => "+N",
            PieceType::PRO_SILVER => "+S",
            PieceType::HORSE => "+B",
            PieceType::DRAGON => "+R",
            _ => return None,
        })
    }

    /// SFEN文字（大文字1字）からのパース。成りは呼び出し側でpromoteする。
    pub fn from_sfen_char(c: char) -> Option<PieceType> {
        Some(match c {
            'P' => PieceType::PAWN,
            'L' => PieceType::LANCE,
            'N' => PieceType::KNIGHT,
            'S' => PieceType::SILVER,
            'G' => PieceType::GOLD,
            'B' => PieceType::BISHOP,
            'R' => PieceType::ROOK,
            'K' => PieceType::KING,
            _ => return None,
        })
    }
}

/// 先後付きの駒。後手 = 駒種 + 16。5bit。
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Piece(pub u8);

impl Piece {
    pub const NB: usize = 32;
    pub const EMPTY: Piece = Piece(0);

    #[inline]
    pub const fn new(c: Color, pt: PieceType) -> Piece {
        debug_assert!(pt.is_piece());
        Piece(pt.0 | ((c as u8) << 4))
    }

    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[inline]
    pub const fn color(self) -> Color {
        debug_assert!(!self.is_empty());
        if self.0 & 16 != 0 {
            Color::White
        } else {
            Color::Black
        }
    }

    #[inline]
    pub const fn piece_type(self) -> PieceType {
        PieceType(self.0 & 15)
    }

    /// 成る（駒種側のbit操作。先後ビットは不変）。
    #[inline]
    pub const fn promote(self) -> Piece {
        debug_assert!(self.piece_type().can_promote());
        Piece(self.0 - 8)
    }

    /// 生駒に戻す（全域関数）。
    #[inline]
    pub const fn unpromote(self) -> Piece {
        Piece(self.0 | 8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promote_unpromote() {
        assert_eq!(PieceType::PAWN.promote(), PieceType::PRO_PAWN);
        assert_eq!(PieceType::ROOK.promote(), PieceType::DRAGON);
        assert_eq!(PieceType::PRO_PAWN.unpromote(), PieceType::PAWN);
        assert_eq!(PieceType::DRAGON.unpromote(), PieceType::ROOK);
        // unpromoteは全域関数（玉・金は無変更）
        assert_eq!(PieceType::KING.unpromote(), PieceType::KING);
        assert_eq!(PieceType::GOLD.unpromote(), PieceType::GOLD);
    }

    #[test]
    fn predicates() {
        assert!(PieceType::PRO_PAWN.is_promoted());
        assert!(PieceType::DRAGON.is_promoted());
        assert!(!PieceType::KING.is_promoted());
        assert!(!PieceType::GOLD.is_promoted());
        assert!(!PieceType::PAWN.is_promoted());

        assert!(PieceType::PAWN.can_promote());
        assert!(PieceType::ROOK.can_promote());
        assert!(!PieceType::GOLD.can_promote());
        assert!(!PieceType::KING.can_promote());
        assert!(!PieceType::HORSE.can_promote());
    }

    #[test]
    fn gold_like() {
        for pt in [
            PieceType::GOLD,
            PieceType::PRO_PAWN,
            PieceType::PRO_LANCE,
            PieceType::PRO_KNIGHT,
            PieceType::PRO_SILVER,
        ] {
            assert!(pt.is_gold_like());
        }
        for pt in [
            PieceType::PAWN,
            PieceType::SILVER,
            PieceType::HORSE,
            PieceType::DRAGON,
            PieceType::KING,
        ] {
            assert!(!pt.is_gold_like());
        }
    }

    #[test]
    fn piece_color_and_type() {
        let p = Piece::new(Color::White, PieceType::BISHOP);
        assert_eq!(p.color(), Color::White);
        assert_eq!(p.piece_type(), PieceType::BISHOP);
        let q = p.promote();
        assert_eq!(q.color(), Color::White);
        assert_eq!(q.piece_type(), PieceType::HORSE);
        assert_eq!(q.unpromote(), p);
    }

    #[test]
    fn sfen_char_roundtrip() {
        for pt in PieceType::HAND_KINDS {
            let s = pt.to_sfen().unwrap();
            assert_eq!(
                PieceType::from_sfen_char(s.chars().next().unwrap()),
                Some(pt)
            );
        }
    }
}
