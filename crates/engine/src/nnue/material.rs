//! 学習する駒割りの入力。駒種ごとの枚数の差（手番側−相手側）を数える。
//!
//! 駒の価値を固定せず、ネットと一緒に評価値の単位で学習させる。単位が初めから
//! 評価値と揃うので、固定の駒割りと違って換算の係数を決めなくてよい。

use himawari_core::{PieceType, Position, Square};

use crate::nnue::NnueNetwork;
use crate::nnue_acc::EvalBoard;
use crate::value::Value;

/// 盤上の駒種。玉を除く13種で、成駒を別に数える。
pub const MATERIAL_BOARD_KINDS: [PieceType; 13] = [
    PieceType::PAWN,
    PieceType::LANCE,
    PieceType::KNIGHT,
    PieceType::SILVER,
    PieceType::GOLD,
    PieceType::BISHOP,
    PieceType::ROOK,
    PieceType::PRO_PAWN,
    PieceType::PRO_LANCE,
    PieceType::PRO_KNIGHT,
    PieceType::PRO_SILVER,
    PieceType::HORSE,
    PieceType::DRAGON,
];

/// 入力の数。盤上13種と持ち駒7種（`PieceType::HAND_KINDS` の順）。
pub const MATERIAL_N: usize = MATERIAL_BOARD_KINDS.len() + PieceType::HAND_KINDS.len();

/// 手番側から見た駒種ごとの枚数の差。
pub fn material_counts(pos: &Position) -> [i8; MATERIAL_N] {
    let us = pos.side_to_move();
    let them = us.flip();
    let mut out = [0i8; MATERIAL_N];
    for (i, &pt) in MATERIAL_BOARD_KINDS.iter().enumerate() {
        out[i] = diff(pos.pieces(us, pt).count(), pos.pieces(them, pt).count());
    }
    let base = MATERIAL_BOARD_KINDS.len();
    for (i, &pt) in PieceType::HAND_KINDS.iter().enumerate() {
        out[base + i] = diff(pos.hand(us).count(pt), pos.hand(them).count(pt));
    }
    out
}

/// 駒種（`PieceType` の番号）から盤上の入力の位置への表。玉と空は-1。
const BOARD_SLOT: [i8; 16] = {
    let mut s = [-1i8; 16];
    let mut i = 0;
    while i < MATERIAL_BOARD_KINDS.len() {
        s[MATERIAL_BOARD_KINDS[i].0 as usize] = i as i8;
        i += 1;
    }
    s
};

/// 学習した駒の価値の和（手番視点、評価値の単位）。駒の価値を持たないネットでは0。
///
/// 盤は `EvalBoard` 越しに81升を数える。coreの局面でも探索の盤でも同じ値になる。
pub fn material_term<B: EvalBoard + ?Sized>(net: &NnueNetwork, pos: &B) -> Value {
    if net.mat_w.is_empty() {
        return 0;
    }
    let us = pos.side_to_move();
    let mut sum = 0i32;
    for i in 0..81u8 {
        let pc = pos.piece_on(Square::from_index(i));
        if pc.is_empty() {
            continue;
        }
        let slot = BOARD_SLOT[pc.piece_type().0 as usize];
        if slot >= 0 {
            let w = net.mat_w[slot as usize];
            sum += if pc.color() == us { w } else { -w };
        }
    }
    let base = MATERIAL_BOARD_KINDS.len();
    for (i, &pt) in PieceType::HAND_KINDS.iter().enumerate() {
        let d = pos.hand_count(us, pt) as i32 - pos.hand_count(us.flip(), pt) as i32;
        sum += net.mat_w[base + i] * d;
    }
    sum
}

fn diff(a: u32, b: u32) -> i8 {
    // 1駒種の枚数は18以下なので、差はi8に収まる
    (a as i32 - b as i32) as i8
}

#[cfg(test)]
mod tests {
    use super::*;
    use himawari_core::SFEN_STARTPOS;

    /// 81升を数える経路と、ビットボードで数える経路が同じ値を出す。
    #[test]
    fn term_matches_counts() {
        let mut net = NnueNetwork::random(7);
        net.mat_w = (0..MATERIAL_N as i32).map(|i| 13 * i - 100).collect();
        for sfen in [
            SFEN_STARTPOS,
            "lnsgkgsnl/1r7/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL w B 1",
            "lnsgk1snl/1r4g2/+P1pppp1pp/6p2/1p7/2P6/1PSPPPPPP/7R1/LN1GKGSNL b Bbp 1",
        ] {
            let pos = Position::from_sfen(sfen).unwrap();
            let want: i32 = material_counts(&pos)
                .iter()
                .zip(&net.mat_w)
                .map(|(&c, &w)| i32::from(c) * w)
                .sum();
            assert_eq!(material_term(&net, &pos), want, "{sfen}");
        }
    }

    #[test]
    fn startpos_is_balanced() {
        let pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
        assert_eq!(material_counts(&pos), [0i8; MATERIAL_N]);
    }

    #[test]
    fn counts_from_side_to_move() {
        // 先手が角を1枚持ち、後手の盤上の角がない局面。後手番なら符号が逆になる
        let b = "lnsgkgsnl/1r7/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b B 1";
        let w = "lnsgkgsnl/1r7/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL w B 1";
        let hand_bishop = MATERIAL_BOARD_KINDS.len() + 5;
        let board_bishop = 5;
        let cb = material_counts(&Position::from_sfen(b).unwrap());
        let cw = material_counts(&Position::from_sfen(w).unwrap());
        assert_eq!(cb[board_bishop], 1);
        assert_eq!(cb[hand_bishop], 1);
        assert_eq!(cw[board_bishop], -1);
        assert_eq!(cw[hand_bishop], -1);
    }
}
