//! 利き生成（ADR-0011）。
//!
//! 近接駒はconst fn生成のテーブル引き（ADR-0005）、遠距離駒（飛角香と
//! 馬龍の遠距離部分）はQugiy系（増加方向は減算の桁借り、減少方向は
//! MSB切り詰め）で計算する。between/lineテーブルもここで持つ。

use crate::bitboard::Bitboard;
use crate::piece::{Piece, PieceType};
use crate::types::{Color, Square};

/// 8方向の(筋, 段)増分。0〜3がビット増加方向、4〜7が減少方向。
/// opp(d) = d ^ 4。
const DF: [i32; 8] = [0, 1, 1, 1, 0, -1, -1, -1];
const DR: [i32; 8] = [1, -1, 0, 1, -1, 1, 0, -1];

const DIR_DOWN: usize = 0; // +1
const DIR_UP: usize = 4; // −1

const fn ray(sq: usize, d: usize) -> u128 {
    let mut m = 0u128;
    let mut f = (sq / 9) as i32 + DF[d];
    let mut r = (sq % 9) as i32 + DR[d];
    while 0 <= f && f < 9 && 0 <= r && r < 9 {
        m |= 1u128 << (f * 9 + r);
        f += DF[d];
        r += DR[d];
    }
    m
}

const RAY: [[u128; 81]; 8] = {
    let mut t = [[0u128; 81]; 8];
    let mut d = 0;
    while d < 8 {
        let mut sq = 0;
        while sq < 81 {
            t[d][sq] = ray(sq, d);
            sq += 1;
        }
        d += 1;
    }
    t
};

/// BETWEEN[a][b]: aとbが筋・段・斜めで一直線のとき、その間（両端を除く）。
static BETWEEN: [[u128; 81]; 81] = {
    let mut t = [[0u128; 81]; 81];
    let mut a = 0;
    while a < 81 {
        let mut d = 0;
        while d < 8 {
            let mut acc = 0u128;
            let mut f = (a / 9) as i32 + DF[d];
            let mut r = (a % 9) as i32 + DR[d];
            while 0 <= f && f < 9 && 0 <= r && r < 9 {
                let b = (f * 9 + r) as usize;
                t[a][b] = acc;
                acc |= 1u128 << b;
                f += DF[d];
                r += DR[d];
            }
            d += 1;
        }
        a += 1;
    }
    t
};

/// LINE[a][b]: aとbが一直線のとき、その直線全体（両端を含む）。
static LINE: [[u128; 81]; 81] = {
    let mut t = [[0u128; 81]; 81];
    let mut a = 0;
    while a < 81 {
        let mut d = 0;
        while d < 4 {
            let line = RAY[d][a] | RAY[d + 4][a] | (1u128 << a);
            // dと反対方向の両方に同じ直線を張る
            let mut dd = 0;
            while dd < 2 {
                let dir = d + dd * 4;
                let mut f = (a / 9) as i32 + DF[dir];
                let mut r = (a % 9) as i32 + DR[dir];
                while 0 <= f && f < 9 && 0 <= r && r < 9 {
                    t[a][(f * 9 + r) as usize] = line;
                    f += DF[dir];
                    r += DR[dir];
                }
                dd += 1;
            }
            d += 1;
        }
        a += 1;
    }
    t
};

const fn step_masks(deltas: &[(i32, i32)]) -> [[u128; 81]; 2] {
    let mut t = [[0u128; 81]; 2];
    let mut c = 0;
    while c < 2 {
        let mut sq = 0;
        while sq < 81 {
            let mut m = 0u128;
            let mut i = 0;
            while i < deltas.len() {
                let dr = if c == 0 { deltas[i].1 } else { -deltas[i].1 };
                let f = (sq / 9) as i32 + deltas[i].0;
                let r = (sq % 9) as i32 + dr;
                if 0 <= f && f < 9 && 0 <= r && r < 9 {
                    m |= 1u128 << (f * 9 + r);
                }
                i += 1;
            }
            t[c][sq] = m;
            sq += 1;
        }
        c += 1;
    }
    t
}

static PAWN_ATTACKS: [[u128; 81]; 2] = step_masks(&[(0, -1)]);
static KNIGHT_ATTACKS: [[u128; 81]; 2] = step_masks(&[(1, -2), (-1, -2)]);
static SILVER_ATTACKS: [[u128; 81]; 2] = step_masks(&[(0, -1), (1, -1), (-1, -1), (1, 1), (-1, 1)]);
static GOLD_ATTACKS: [[u128; 81]; 2] =
    step_masks(&[(0, -1), (1, -1), (-1, -1), (1, 0), (-1, 0), (0, 1)]);
static KING_ATTACKS: [[u128; 81]; 2] = step_masks(&[
    (0, -1),
    (1, -1),
    (-1, -1),
    (1, 0),
    (-1, 0),
    (0, 1),
    (1, 1),
    (-1, 1),
]);

/// 増加方向のレイ。最初の駒まで（駒を含む）。
#[inline]
fn ray_inc(occ: u128, mask: u128) -> u128 {
    let t = occ & mask;
    (t ^ t.wrapping_sub(1)) & mask
}

/// 減少方向のレイ。`t | 1` はclzを非ゼロにする番兵（結果に影響しない）。
#[inline]
fn ray_dec(occ: u128, mask: u128) -> u128 {
    let t = occ & mask;
    mask & (u128::MAX << (127 - (t | 1).leading_zeros()))
}

#[inline]
pub fn pawn_attacks(c: Color, sq: Square) -> Bitboard {
    Bitboard(PAWN_ATTACKS[c.index()][sq.index()])
}

#[inline]
pub fn knight_attacks(c: Color, sq: Square) -> Bitboard {
    Bitboard(KNIGHT_ATTACKS[c.index()][sq.index()])
}

#[inline]
pub fn silver_attacks(c: Color, sq: Square) -> Bitboard {
    Bitboard(SILVER_ATTACKS[c.index()][sq.index()])
}

#[inline]
pub fn gold_attacks(c: Color, sq: Square) -> Bitboard {
    Bitboard(GOLD_ATTACKS[c.index()][sq.index()])
}

#[inline]
pub fn king_attacks(sq: Square) -> Bitboard {
    Bitboard(KING_ATTACKS[0][sq.index()])
}

#[inline]
pub fn lance_attacks(c: Color, sq: Square, occ: Bitboard) -> Bitboard {
    match c {
        Color::Black => Bitboard(ray_dec(occ.raw(), RAY[DIR_UP][sq.index()])),
        Color::White => Bitboard(ray_inc(occ.raw(), RAY[DIR_DOWN][sq.index()])),
    }
}

#[inline]
pub fn bishop_attacks(sq: Square, occ: Bitboard) -> Bitboard {
    let i = sq.index();
    let o = occ.raw();
    Bitboard(
        ray_inc(o, RAY[1][i])
            | ray_inc(o, RAY[3][i])
            | ray_dec(o, RAY[5][i])
            | ray_dec(o, RAY[7][i]),
    )
}

#[inline]
pub fn rook_attacks(sq: Square, occ: Bitboard) -> Bitboard {
    let i = sq.index();
    let o = occ.raw();
    Bitboard(
        ray_inc(o, RAY[0][i])
            | ray_inc(o, RAY[2][i])
            | ray_dec(o, RAY[4][i])
            | ray_dec(o, RAY[6][i]),
    )
}

/// 駒pcがsqから利かすマス（occは全体の占有）。
pub fn attacks(pc: Piece, sq: Square, occ: Bitboard) -> Bitboard {
    let c = pc.color();
    let pt = pc.piece_type();
    if pt.is_gold_like() {
        return gold_attacks(c, sq);
    }
    match pt {
        PieceType::PAWN => pawn_attacks(c, sq),
        PieceType::LANCE => lance_attacks(c, sq, occ),
        PieceType::KNIGHT => knight_attacks(c, sq),
        PieceType::SILVER => silver_attacks(c, sq),
        PieceType::KING => king_attacks(sq),
        PieceType::BISHOP => bishop_attacks(sq, occ),
        PieceType::ROOK => rook_attacks(sq, occ),
        PieceType::HORSE => bishop_attacks(sq, occ) | king_attacks(sq),
        PieceType::DRAGON => rook_attacks(sq, occ) | king_attacks(sq),
        _ => unreachable!("attacks() called with non-piece"),
    }
}

/// aとbの間のマス（両端を除く。非直線なら空）。
#[inline]
pub fn between(a: Square, b: Square) -> Bitboard {
    Bitboard(BETWEEN[a.index()][b.index()])
}

/// aとbを通る直線全体（両端を含む。非直線なら空）。
#[inline]
pub fn line(a: Square, b: Square) -> Bitboard {
    Bitboard(LINE[a.index()][b.index()])
}

/// a・b・cが一直線上にあるか（pin判定用）。
#[inline]
pub fn aligned(a: Square, b: Square, c: Square) -> bool {
    line(a, c).test(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{File, Rank};

    /// 素朴なマス走査による基準実装。
    fn naive_attacks(pc: Piece, sq: Square, occ: Bitboard) -> Bitboard {
        let c = pc.color();
        let pt = pc.piece_type();
        let sign = if c == Color::Black { -1 } else { 1 };
        let steps: Vec<(i32, i32, bool)> = match pt {
            PieceType::PAWN => vec![(0, sign, false)],
            PieceType::LANCE => vec![(0, sign, true)],
            PieceType::KNIGHT => vec![(1, 2 * sign, false), (-1, 2 * sign, false)],
            PieceType::SILVER => vec![
                (0, sign, false),
                (1, sign, false),
                (-1, sign, false),
                (1, -sign, false),
                (-1, -sign, false),
            ],
            PieceType::KING => (-1..=1)
                .flat_map(|f| (-1..=1).map(move |r| (f, r, false)))
                .filter(|&(f, r, _)| (f, r) != (0, 0))
                .collect(),
            PieceType::BISHOP => vec![(1, 1, true), (1, -1, true), (-1, 1, true), (-1, -1, true)],
            PieceType::ROOK => vec![(0, 1, true), (0, -1, true), (1, 0, true), (-1, 0, true)],
            PieceType::HORSE => {
                let mut v = naive_steps(PieceType::BISHOP);
                v.extend(naive_steps(PieceType::KING));
                v
            }
            PieceType::DRAGON => {
                let mut v = naive_steps(PieceType::ROOK);
                v.extend(naive_steps(PieceType::KING));
                v
            }
            _ if pt.is_gold_like() => vec![
                (0, sign, false),
                (1, sign, false),
                (-1, sign, false),
                (1, 0, false),
                (-1, 0, false),
                (0, -sign, false),
            ],
            _ => panic!("non-piece"),
        };
        walk(sq, occ, &steps)
    }

    fn naive_steps(pt: PieceType) -> Vec<(i32, i32, bool)> {
        match pt {
            PieceType::BISHOP => vec![(1, 1, true), (1, -1, true), (-1, 1, true), (-1, -1, true)],
            PieceType::ROOK => vec![(0, 1, true), (0, -1, true), (1, 0, true), (-1, 0, true)],
            PieceType::KING => (-1..=1)
                .flat_map(|f| (-1..=1).map(move |r| (f, r, false)))
                .filter(|&(f, r, _)| (f, r) != (0, 0))
                .collect(),
            _ => unreachable!(),
        }
    }

    fn walk(sq: Square, occ: Bitboard, steps: &[(i32, i32, bool)]) -> Bitboard {
        let mut result = Bitboard::EMPTY;
        for &(df, dr, slide) in steps {
            let mut f = i32::from(sq.file().0) + df;
            let mut r = i32::from(sq.rank().0) + dr;
            while (0..9).contains(&f) && (0..9).contains(&r) {
                let to = Square::new(File(f as u8), Rank(r as u8));
                result.set(to);
                if !slide || occ.test(to) {
                    break;
                }
                f += df;
                r += dr;
            }
        }
        result
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn bb(&mut self) -> Bitboard {
            Bitboard((u128::from(self.next()) | (u128::from(self.next()) << 64)) & Bitboard::ALL.0)
        }
    }

    #[test]
    fn attacks_match_naive_for_all_pieces() {
        let mut rng = Rng(0x1234_5678_9ABC_DEF0);
        let kinds = [
            PieceType::PAWN,
            PieceType::LANCE,
            PieceType::KNIGHT,
            PieceType::SILVER,
            PieceType::GOLD,
            PieceType::KING,
            PieceType::BISHOP,
            PieceType::ROOK,
            PieceType::HORSE,
            PieceType::DRAGON,
            PieceType::PRO_PAWN,
            PieceType::PRO_SILVER,
        ];
        for _ in 0..32 {
            let occ = rng.bb();
            for i in 0..81 {
                let sq = Square::from_index(i);
                for c in [Color::Black, Color::White] {
                    for pt in kinds {
                        let pc = Piece::new(c, pt);
                        assert_eq!(
                            attacks(pc, sq, occ),
                            naive_attacks(pc, sq, occ),
                            "pt={pt:?} c={c:?} sq={i}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn between_and_line() {
        let a = Square::new(File(0), Rank(0));
        let b = Square::new(File(0), Rank(4));
        assert_eq!(between(a, b).count(), 3);
        assert!(line(a, b).test(a) && line(a, b).test(b));
        assert_eq!(line(a, b).count(), 9);

        // 斜め
        let c = Square::new(File(4), Rank(4));
        assert_eq!(between(a, c).count(), 3);
        assert!(aligned(a, Square::new(File(2), Rank(2)), c));

        // 非直線
        let d = Square::new(File(1), Rank(3));
        assert!(between(a, d).is_empty());
        assert!(line(a, d).is_empty());
        assert!(!aligned(a, Square::new(File(2), Rank(2)), d));

        // 対称性
        for x in 0..81 {
            for y in 0..81 {
                let (x, y) = (Square::from_index(x), Square::from_index(y));
                assert_eq!(between(x, y), between(y, x));
                assert_eq!(line(x, y), line(y, x));
            }
        }
    }

    #[test]
    fn lance_blocked() {
        // 先手香が5五、5三に駒 → 利きは5四・5三まで
        let sq = Square::new(File(4), Rank(4));
        let blocker = Square::new(File(4), Rank(2));
        let occ = Bitboard::from_square(blocker);
        let att = lance_attacks(Color::Black, sq, occ);
        assert_eq!(att.count(), 2);
        assert!(att.test(Square::new(File(4), Rank(3))));
        assert!(att.test(blocker));
    }
}
