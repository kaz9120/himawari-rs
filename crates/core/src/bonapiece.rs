//! BonaPiece番号付け（やねうら王互換、ADR-0034）。
//!
//! HalfKP特徴量のインデックスを構成する駒の通し番号。成り小駒
//! （と・成香・成桂・成銀）は金として数え、玉は特徴に含めない。
//! 番号はやねうら王のBonaPiece enumと同一で、公開評価関数の
//! 読み込み互換（検証フィクスチャ）の前提になる。
//! 盤上駒の升番号は当プロジェクトのSquare（筋×9+段）がやねうら王の
//! SQ番号と一致するため、変換なしで使える。

use crate::bitboard::Bitboard;
use crate::piece::{Piece, PieceType};
use crate::position::Position;
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

/// 視点cから見た盤上駒(pc, sq)のBonaPiece。玉には使えない。
#[inline]
pub fn board_bona_piece(c: Color, pc: Piece, sq: Square) -> u16 {
    debug_assert!(pc.piece_type() != PieceType::KING);
    let sq = if c == Color::Black { sq } else { sq.inv() };
    board_base(pc.color() == c, pc.piece_type()) + sq.index() as u16
}

/// 視点cから見た、owner側の手駒ptのi枚目（1始まり）のBonaPiece。
#[inline]
pub fn hand_bona_piece(c: Color, owner: Color, pt: PieceType, i: u32) -> u16 {
    debug_assert!(i >= 1);
    hand_base(owner == c, pt) + (i - 1) as u16
}

/// BonaPieceの集合を持つビットセットの語数。
pub const BP_WORDS: usize = (FE_END as usize).div_ceil(64);

/// 盤上駒のカテゴリと、視点側ブロックの起点（ADR-0164）。
///
/// 相手側のブロックは起点の直後81升に並ぶ。金の動きをする5駒種は
/// `board_base` の `gold_like` で1つに畳まれるため、ここでも
/// `PieceType::GOLD` の1カテゴリで代表し、盤は `Position::golds` で引く。
/// 玉は特徴に入らないのでカテゴリを持たない。
const BOARD_CATEGORIES: [(PieceType, u16); 9] = [
    (PieceType::PAWN, F_PAWN),
    (PieceType::LANCE, F_LANCE),
    (PieceType::KNIGHT, F_KNIGHT),
    (PieceType::SILVER, F_SILVER),
    (PieceType::GOLD, F_GOLD),
    (PieceType::BISHOP, F_BISHOP),
    (PieceType::HORSE, F_HORSE),
    (PieceType::ROOK, F_ROOK),
    (PieceType::DRAGON, F_DRAGON),
];

/// 81升のビット並びを180度回す。`Square::inv`（80 - sq）と同じ写像。
#[inline]
const fn rev81(x: u128) -> u128 {
    x.reverse_bits() >> (128 - 81)
}

/// ビットセットのbit位置posへ、64ビットの値vをORする。
#[inline]
fn or_at(out: &mut [u64; BP_WORDS], pos: u16, v: u64) {
    if v == 0 {
        return;
    }
    let w = pos as usize / 64;
    let sh = pos as usize % 64;
    out[w] |= v << sh;
    // 語をまたぐぶん。sh==0のときシフト量が64になるので分ける
    if sh != 0 {
        out[w + 1] |= v >> (64 - sh);
    }
}

/// 視点cのBonaPiece集合をビットセットへ書く（ADR-0164）。
///
/// 結果は `board_bona_piece`・`hand_bona_piece` を駒1枚ずつ呼んで
/// ビットを立てたものと一致する。**1枚ずつ回らずに済むのは、BonaPieceの
/// 番号付けが盤上は81升ぶんのブロックの並び、手駒は枚数ぶんの連番に
/// なっているためである。** 盤はbitboardをブロックの起点へシフトして入れ、
/// 手駒は連続ビットのマスクを1回ORすれば同じ集合になる。
pub fn bona_piece_bits(pos: &Position, c: Color, out: &mut [u64; BP_WORDS]) {
    *out = [0; BP_WORDS];

    for owner in [Color::Black, Color::White] {
        let hand = pos.hand(owner);
        for pt in PieceType::HAND_KINDS {
            let n = hand.count(pt);
            if n == 0 {
                continue;
            }
            // 枚数のフィールド幅は最大5ビット（歩）なのでシフトは溢れない
            debug_assert!(n < 64, "手駒の枚数がフィールド幅を超えた: {n}");
            or_at(out, hand_base(owner == c, pt), (1u64 << n) - 1);
        }
    }

    for (pt, f) in BOARD_CATEGORIES {
        for owner in [Color::Black, Color::White] {
            let bb = if pt == PieceType::GOLD {
                pos.golds(owner)
            } else {
                pos.pieces(owner, pt)
            };
            if bb == Bitboard::EMPTY {
                continue;
            }
            // 後手視点は盤を180度回す。board_bona_pieceのsq.inv()にあたる
            let bits = if c == Color::Black {
                bb.raw()
            } else {
                rev81(bb.raw())
            };
            let base = if owner == c { f } else { f + 81 };
            or_at(out, base, bits as u64);
            or_at(out, base + 64, (bits >> 64) as u64);
        }
    }
}

/// HalfKPの特徴インデックス: 視点cの自玉位置 × FE_END + BonaPiece。
#[inline]
pub fn halfkp_index(c: Color, own_king: Square, bp: u16) -> u32 {
    let k = if c == Color::Black {
        own_king
    } else {
        own_king.inv()
    };
    k.index() as u32 * u32::from(FE_END) + u32::from(bp)
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

    #[test]
    fn perspective_flip_symmetry() {
        // 先手視点の先手歩@5五 と 後手視点の後手歩@5五(回転で同じ相対位置)
        let sq = Square::new(File(4), Rank(4));
        let b = board_bona_piece(Color::Black, Piece::new(Color::Black, PieceType::PAWN), sq);
        let w = board_bona_piece(
            Color::White,
            Piece::new(Color::White, PieceType::PAWN),
            sq.inv(),
        );
        assert_eq!(b, w);
        // 5五の回転は5五
        assert_eq!(sq.inv(), sq);
    }

    #[test]
    fn promoted_smalls_are_gold() {
        let sq = Square::new(File(0), Rank(0));
        let gold = board_bona_piece(Color::Black, Piece::new(Color::Black, PieceType::GOLD), sq);
        let tokin = board_bona_piece(
            Color::Black,
            Piece::new(Color::Black, PieceType::PRO_PAWN),
            sq,
        );
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
    fn halfkp_index_range() {
        // 最大値: 玉が81升目、BonaPieceがFE_END-1
        let max = halfkp_index(Color::Black, Square::new(File(8), Rank(8)), FE_END - 1);
        assert_eq!(max, 80 * u32::from(FE_END) + u32::from(FE_END) - 1);
        assert!(max < 81 * u32::from(FE_END));
    }
}
