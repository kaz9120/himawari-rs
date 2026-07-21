//! Zobristハッシュ（ADR-0015）。
//!
//! board_keyの乱数はseed固定のsplitmix64でconst fn生成する。
//! 全乱数はbit 0を0にし、手番はbit 0のXORで表す（board_keyの
//! bit 0 = 手番）。hand_keyはHandの生の値なので乱数は不要。

use crate::piece::Piece;
use crate::types::{Color, Square};

/// 手番のXOR値。
pub const SIDE: u64 = 1;

/// 置換表キー合成用の拡散定数（splitmix64の増分、奇数）。
pub const HAND_MIX: u64 = 0x9E37_79B9_7F4A_7C15;

const fn splitmix64(state: u64) -> (u64, u64) {
    let s = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = s;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (s, z ^ (z >> 31))
}

/// PSQ[piece][square]。空きスロット（EMPTY等）は0のまま。
static PSQ: [[u64; 81]; 32] = {
    let mut t = [[0u64; 81]; 32];
    let mut state = 20260717u64;
    let mut p = 2; // 先手の駒 2..=15、後手 18..=31。EMPTY/予約は0
    while p < 32 {
        if p % 16 >= 2 {
            let mut sq = 0;
            while sq < 81 {
                let (s, v) = splitmix64(state);
                state = s;
                t[p][sq] = v & !1; // bit 0は手番用に空ける
                sq += 1;
            }
        }
        p += 1;
    }
    t
};

#[inline]
pub fn psq(pc: Piece, sq: Square) -> u64 {
    PSQ[pc.index()][sq.index()]
}

/// 歩構造キー用の持ち歩テーブル（ADR-0046）。[色][枚数 0..=18]。
/// PSQと同じsplitmix64のconst fn生成に合わせる。
static HAND_PAWN: [[u64; 19]; 2] = {
    let mut t = [[0u64; 19]; 2];
    let mut state = 20260721u64;
    let mut c = 0;
    while c < 2 {
        let mut n = 0;
        while n < 19 {
            let (s, v) = splitmix64(state);
            state = s;
            t[c][n] = v;
            n += 1;
        }
        c += 1;
    }
    t
};

/// 色cの持ち歩count枚に対応する歩構造キー成分（ADR-0046）。
#[inline]
pub fn hand_pawn(c: Color, count: u32) -> u64 {
    HAND_PAWN[c.index()][count as usize]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::piece::PieceType;
    use crate::types::Color;

    #[test]
    fn all_entries_have_bit0_clear() {
        for (p, row) in PSQ.iter().enumerate() {
            for (sq, v) in row.iter().enumerate() {
                assert_eq!(v & 1, 0, "p={p} sq={sq}");
            }
        }
    }

    #[test]
    fn piece_entries_are_nonzero_and_distinct() {
        let mut seen = std::collections::HashSet::new();
        for c in [Color::Black, Color::White] {
            for ptv in 2..=15u8 {
                let pc = Piece::new(c, PieceType(ptv));
                for sq in 0..81 {
                    let v = psq(pc, Square::from_index(sq));
                    assert_ne!(v, 0);
                    assert!(seen.insert(v), "duplicate zobrist value");
                }
            }
        }
    }
}
