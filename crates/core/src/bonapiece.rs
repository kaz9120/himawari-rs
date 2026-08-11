//! BonaPiece番号付け（やねうら王互換、ADR-0034）。
//!
//! HalfKP特徴量のインデックスを構成する駒の通し番号。成り小駒
//! （と・成香・成桂・成銀）は金として数え、玉は特徴に含めない。
//! 番号はやねうら王のBonaPiece enumと同一で、公開評価関数の
//! 読み込み互換（検証フィクスチャ）の前提になる。
//! 盤上駒の升番号は当プロジェクトのSquare（筋×9+段）がやねうら王の
//! SQ番号と一致するため、変換なしで使える。

use crate::piece::{Piece, PieceType};
use crate::types::{Color, Square};

// 手駒（f_=視点側、e_=相手側。歩は最大18枚、香桂銀金4、角飛2）
pub const F_HAND_PAWN: u16 = 1;
pub const E_HAND_PAWN: u16 = 20;
pub const F_HAND_LANCE: u16 = 39;
pub const E_HAND_LANCE: u16 = 44;
pub const F_HAND_KNIGHT: u16 = 49;
pub const E_HAND_KNIGHT: u16 = 54;
pub const F_HAND_SILVER: u16 = 59;
pub const E_HAND_SILVER: u16 = 64;
pub const F_HAND_GOLD: u16 = 69;
pub const E_HAND_GOLD: u16 = 74;
pub const F_HAND_BISHOP: u16 = 79;
pub const E_HAND_BISHOP: u16 = 82;
pub const F_HAND_ROOK: u16 = 85;
pub const E_HAND_ROOK: u16 = 88;
pub const FE_HAND_END: u16 = 90;

// 盤上駒（81升ずつ）
pub const F_PAWN: u16 = FE_HAND_END;
pub const E_PAWN: u16 = F_PAWN + 81;
pub const F_LANCE: u16 = E_PAWN + 81;
pub const E_LANCE: u16 = F_LANCE + 81;
pub const F_KNIGHT: u16 = E_LANCE + 81;
pub const E_KNIGHT: u16 = F_KNIGHT + 81;
pub const F_SILVER: u16 = E_KNIGHT + 81;
pub const E_SILVER: u16 = F_SILVER + 81;
pub const F_GOLD: u16 = E_SILVER + 81;
pub const E_GOLD: u16 = F_GOLD + 81;
pub const F_BISHOP: u16 = E_GOLD + 81;
pub const E_BISHOP: u16 = F_BISHOP + 81;
pub const F_HORSE: u16 = E_BISHOP + 81;
pub const E_HORSE: u16 = F_HORSE + 81;
pub const F_ROOK: u16 = E_HORSE + 81;
pub const E_ROOK: u16 = F_ROOK + 81;
pub const F_DRAGON: u16 = E_ROOK + 81;
pub const E_DRAGON: u16 = F_DRAGON + 81;
pub const FE_END: u16 = E_DRAGON + 81;

/// 手駒のBonaPiece起点。us_viewは視点側の駒か。
fn hand_base(us_view: bool, pt: PieceType) -> u16 {
    match (pt, us_view) {
        (PieceType::PAWN, true) => F_HAND_PAWN,
        (PieceType::PAWN, false) => E_HAND_PAWN,
        (PieceType::LANCE, true) => F_HAND_LANCE,
        (PieceType::LANCE, false) => E_HAND_LANCE,
        (PieceType::KNIGHT, true) => F_HAND_KNIGHT,
        (PieceType::KNIGHT, false) => E_HAND_KNIGHT,
        (PieceType::SILVER, true) => F_HAND_SILVER,
        (PieceType::SILVER, false) => E_HAND_SILVER,
        (PieceType::GOLD, true) => F_HAND_GOLD,
        (PieceType::GOLD, false) => E_HAND_GOLD,
        (PieceType::BISHOP, true) => F_HAND_BISHOP,
        (PieceType::BISHOP, false) => E_HAND_BISHOP,
        (PieceType::ROOK, true) => F_HAND_ROOK,
        (PieceType::ROOK, false) => E_HAND_ROOK,
        _ => unreachable!("手駒になれない駒種: {pt:?}"),
    }
}

/// 盤上駒のBonaPiece起点。成り小駒は金に写像する。
fn board_base(us_view: bool, pt: PieceType) -> u16 {
    let gold_like = pt == PieceType::GOLD
        || pt == PieceType::PRO_PAWN
        || pt == PieceType::PRO_LANCE
        || pt == PieceType::PRO_KNIGHT
        || pt == PieceType::PRO_SILVER;
    let f = if gold_like {
        F_GOLD
    } else {
        match pt {
            PieceType::PAWN => F_PAWN,
            PieceType::LANCE => F_LANCE,
            PieceType::KNIGHT => F_KNIGHT,
            PieceType::SILVER => F_SILVER,
            PieceType::BISHOP => F_BISHOP,
            PieceType::HORSE => F_HORSE,
            PieceType::ROOK => F_ROOK,
            PieceType::DRAGON => F_DRAGON,
            _ => unreachable!("盤上特徴になれない駒種: {pt:?}"),
        }
    };
    // e_系はf_系の直後81升
    if us_view { f } else { f + 81 }
}

/// 視点cから見た、owner側の手駒ptのi枚目（1始まり）のBonaPiece。
#[inline]
pub fn hand_bona_piece(c: Color, owner: Color, pt: PieceType, i: u32) -> u16 {
    debug_assert!(i >= 1);
    hand_base(owner == c, pt) + (i - 1) as u16
}

/// 玉バケットの数（ADR-0157）。盤は左右対称なので、自玉の筋を1〜5へ
/// 正規化して5筋 × 9段に畳む。
pub const KING_BUCKETS: usize = 5 * 9;

/// 視点1つ分の特徴インデックスの作り方（ADR-0157）。
///
/// **自玉の位置から、盤面を左右反転するかと玉バケットの起点が同時に
/// 決まる。** 升の反転とバケットの選び方は必ず対で使うので、別々の関数に
/// 分けず、この型に閉じ込める。片方だけ適用すると静かに壊れる。
#[derive(Clone, Copy)]
pub struct View {
    c: Color,
    /// 盤面を左右反転するか。自玉の筋が中央より右のときに立つ。
    mirror: bool,
    /// バケットの先頭インデックス（バケット番号 × FE_END）。
    base: u32,
}

impl View {
    /// 視点cと、その視点の自玉位置から作る。
    #[inline]
    pub fn new(c: Color, own_king: Square) -> View {
        // 先に視点の向きへ回してから、左右を正規化する
        let k = if c == Color::Black {
            own_king
        } else {
            own_king.inv()
        };
        let mirror = k.file().0 >= 5;
        let k = if mirror { k.mir() } else { k };
        debug_assert!(k.file().0 < 5, "玉の筋が正規化されていない");
        let bucket = u32::from(k.file().0) * 9 + u32::from(k.rank().0);
        View {
            c,
            mirror,
            base: bucket * u32::from(FE_END),
        }
    }

    /// バケットの先頭インデックス。BonaPieceを足せば特徴インデックスになる。
    #[inline]
    pub fn base(self) -> u32 {
        self.base
    }

    /// 盤上駒(pc, sq)のBonaPiece。玉には使えない。
    #[inline]
    pub fn board_bona_piece(self, pc: Piece, sq: Square) -> u16 {
        debug_assert!(pc.piece_type() != PieceType::KING);
        let sq = if self.c == Color::Black { sq } else { sq.inv() };
        let sq = if self.mirror { sq.mir() } else { sq };
        board_base(pc.color() == self.c, pc.piece_type()) + sq.index() as u16
    }

    /// owner側の手駒ptのi枚目（1始まり）のBonaPiece。
    /// 手駒は升を持たないので、左右反転の影響を受けない。
    #[inline]
    pub fn hand_bona_piece(self, owner: Color, pt: PieceType, i: u32) -> u16 {
        hand_bona_piece(self.c, owner, pt, i)
    }

    /// 盤上駒の特徴インデックス。
    #[inline]
    pub fn board_index(self, pc: Piece, sq: Square) -> u32 {
        self.base + u32::from(self.board_bona_piece(pc, sq))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{File, Rank};

    #[test]
    fn constants_match_yaneuraou() {
        // やねうら王evaluate.hのBonaPiece定数と一致すること
        assert_eq!(FE_HAND_END, 90);
        assert_eq!(F_PAWN, 90);
        assert_eq!(E_PAWN, 171);
        assert_eq!(F_GOLD, 738);
        assert_eq!(F_DRAGON, 1386);
        assert_eq!(FE_END, 1548);
    }

    /// 玉を5五に置く視点。5筋は左右の正規化で反転しないので、
    /// 反転そのものを含まない性質だけを見るテストで使う。
    fn center_view(c: Color) -> View {
        View::new(c, Square::new(File(4), Rank(4)))
    }

    #[test]
    fn perspective_flip_symmetry() {
        // 先手視点の先手歩@5五 と 後手視点の後手歩@5五(回転で同じ相対位置)
        let sq = Square::new(File(4), Rank(4));
        let b = center_view(Color::Black)
            .board_bona_piece(Piece::new(Color::Black, PieceType::PAWN), sq);
        let w = center_view(Color::White)
            .board_bona_piece(Piece::new(Color::White, PieceType::PAWN), sq.inv());
        assert_eq!(b, w);
        // 5五の回転は5五
        assert_eq!(sq.inv(), sq);
    }

    #[test]
    fn promoted_smalls_are_gold() {
        let sq = Square::new(File(0), Rank(0));
        let v = center_view(Color::Black);
        let gold = v.board_bona_piece(Piece::new(Color::Black, PieceType::GOLD), sq);
        let tokin = v.board_bona_piece(Piece::new(Color::Black, PieceType::PRO_PAWN), sq);
        assert_eq!(gold, tokin);
    }

    #[test]
    fn hand_indexing() {
        // 視点側の歩1枚目は1、相手側の歩1枚目は20
        assert_eq!(
            hand_bona_piece(Color::Black, Color::Black, PieceType::PAWN, 1),
            1
        );
        assert_eq!(
            hand_bona_piece(Color::Black, Color::White, PieceType::PAWN, 1),
            20
        );
        // 後手視点では立場が入れ替わる
        assert_eq!(
            hand_bona_piece(Color::White, Color::White, PieceType::PAWN, 1),
            1
        );
        assert_eq!(
            hand_bona_piece(Color::White, Color::Black, PieceType::ROOK, 2),
            E_HAND_ROOK + 1
        );
    }

    #[test]
    fn king_buckets_cover_the_board_without_overflow() {
        // 81升のどこに玉があっても、バケットは45通りに収まる
        let mut seen = std::collections::HashSet::new();
        for i in 0..81u8 {
            for c in [Color::Black, Color::White] {
                let v = View::new(c, Square::from_index(i));
                let bucket = v.base() / u32::from(FE_END);
                assert!(bucket < KING_BUCKETS as u32, "バケットが範囲外: {bucket}");
                seen.insert(bucket);
            }
        }
        assert_eq!(seen.len(), KING_BUCKETS, "45バケットすべてが現れる");
        // 最大の特徴インデックスがFT_INに収まる
        let max = (KING_BUCKETS as u32) * u32::from(FE_END) - 1;
        assert_eq!(max, 45 * u32::from(FE_END) - 1);
    }

    #[test]
    fn mirrored_kings_share_a_bucket() {
        // 左右対称の位置にある玉は同じバケットへ落ちる
        for i in 0..81u8 {
            let sq = Square::from_index(i);
            let a = View::new(Color::Black, sq);
            let b = View::new(Color::Black, sq.mir());
            assert_eq!(a.base(), b.base(), "鏡像の玉が別バケットになった: {i}");
        }
    }

    #[test]
    fn mirrored_positions_give_identical_features() {
        // 玉と駒をまとめて左右反転すると、同じ特徴インデックスになる
        let king = Square::new(File(7), Rank(8));
        let pc = Piece::new(Color::Black, PieceType::SILVER);
        for i in 0..81u8 {
            let sq = Square::from_index(i);
            if sq == king {
                continue;
            }
            let a = View::new(Color::Black, king).board_index(pc, sq);
            let b = View::new(Color::Black, king.mir()).board_index(pc, sq.mir());
            assert_eq!(a, b, "鏡像で特徴が食い違う: 駒={i}");
        }
    }
}
