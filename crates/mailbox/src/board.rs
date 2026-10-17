//! 壁付きの配列盤の座標と、駒の動きの表。
//!
//! 升の添字は `idx = (筋 + 2) * 11 + 段 + 1`（筋・段は0始まり、1筋・一段が0）。
//! 1筋は縦11升で、上下に1升ずつ壁を持つ。左右は筋−1と筋9の列が壁になる。
//! 桂が盤の外へ2段跳んでも、隣の列の壁の行へ落ちるので、範囲の検査が要らない。
//! 添字の最小は10、最大は132で、配列の長さは143とする。
//!
//! 方向の差分は先手から見て 上=−1、下=+1、左（9筋側）=+11、右（1筋側）=−11。
//! 方向の番号は逆向きの組が `d ^ 1` になるように並べる。

/// 手番・駒の先後。
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Color {
    Black = 0,
    White = 1,
}

impl Color {
    #[inline]
    pub const fn flip(self) -> Color {
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

pub const SFEN_STARTPOS: &str = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";

/// 列の高さ（段9＋上下の壁）。筋をまたぐ差分になる。
pub const STRIDE: i32 = 11;
/// 盤の配列の長さ。
pub const LEN: usize = 143;

/// 筋・段（0始まり）から升の添字を作る。
#[inline]
pub const fn idx(file: u8, rank: u8) -> u8 {
    ((file as u16 + 2) * 11 + rank as u16 + 1) as u8
}

/// 升の添字から81升の通し番号（`file * 9 + rank`、coreのSquareと同じ）。壁は0xFF。
pub const SQ81: [u8; LEN] = {
    let mut t = [0xFFu8; LEN];
    let mut f = 0;
    while f < 9 {
        let mut r = 0;
        while r < 9 {
            t[idx(f, r) as usize] = f * 9 + r;
            r += 1;
        }
        f += 1;
    }
    t
};

/// 81升の通し番号から升の添字。
pub const IDX81: [u8; 81] = {
    let mut t = [0u8; 81];
    let mut i = 0;
    while i < 81 {
        t[i] = idx((i / 9) as u8, (i % 9) as u8);
        i += 1;
    }
    t
};

#[inline]
pub const fn file_of(i: u8) -> u8 {
    i / 11 - 2
}

#[inline]
pub const fn rank_of(i: u8) -> u8 {
    i % 11 - 1
}

/// 手番cから見た段（先手はそのまま、後手は反転）。
#[inline]
pub const fn rel_rank(c: Color, i: u8) -> u8 {
    match c {
        Color::Black => rank_of(i),
        Color::White => 8 - rank_of(i),
    }
}

/// USI表記の升（例: "7g"）。
pub fn usi_square(i: u8) -> String {
    format!("{}{}", file_of(i) + 1, char::from(b'a' + rank_of(i)))
}

/// USI表記から升の添字。
pub fn parse_usi_square(s: &[u8]) -> Option<u8> {
    if s.len() != 2 {
        return None;
    }
    let f = s[0].checked_sub(b'1')?;
    let r = s[1].checked_sub(b'a')?;
    (f < 9 && r < 9).then(|| idx(f, r))
}

// --- 方向 ---

pub const UP: usize = 0;
pub const DOWN: usize = 1;
pub const LEFT: usize = 2;
pub const RIGHT: usize = 3;
pub const UL: usize = 4;
pub const DR: usize = 5;
pub const UR: usize = 6;
pub const DL: usize = 7;

/// 方向の差分。逆向きは `d ^ 1`。
pub const DELTA: [i8; 8] = [-1, 1, 11, -11, 10, -10, -12, 12];

#[inline]
pub const fn opp(d: usize) -> usize {
    d ^ 1
}

/// 差分（−12..=12）から方向の番号。8方向でなければ8。
pub const DIR_OF_DELTA: [u8; 25] = {
    let mut t = [8u8; 25];
    let mut d = 0;
    while d < 8 {
        t[(DELTA[d] + 12) as usize] = d as u8;
        d += 1;
    }
    t
};

/// 盤上の升aから升bへ向かう方向。同じ筋・段・斜めの線に無ければNone。
#[inline]
pub fn dir_between(a: u8, b: u8) -> Option<usize> {
    let df = file_of(b) as i32 - file_of(a) as i32;
    let dr = rank_of(b) as i32 - rank_of(a) as i32;
    match (df.signum(), dr.signum()) {
        (0, 0) => None,
        (0, -1) => Some(UP),
        (0, 1) => Some(DOWN),
        (1, 0) => Some(LEFT),
        (-1, 0) => Some(RIGHT),
        _ if df.abs() != dr.abs() => None,
        (1, -1) => Some(UL),
        (-1, 1) => Some(DR),
        (-1, -1) => Some(UR),
        _ => Some(DL),
    }
}

// --- 81升の集合 ---
// 升の集合はcoreのBitboardと同じく、bit i = 81升の通し番号iのu128で持つ。
// 利きの計算には使わない。空き升の列挙と、生成の順をcoreに揃えるための並べ替えに使う。

/// 81升すべて。
pub const ALL81: u128 = (1u128 << 81) - 1;

/// 段rの升の集合。
pub const fn rank_mask(r: u8) -> u128 {
    let mut m = 0u128;
    let mut f = 0;
    while f < 9 {
        m |= 1u128 << (f * 9 + r);
        f += 1;
    }
    m
}

/// 筋fの升の集合。
pub const fn file_mask(f: u8) -> u128 {
    0x1FFu128 << (f * 9)
}

/// 升の添字の、81升の集合のビット。
#[inline]
pub const fn bit81(i: u8) -> u128 {
    1u128 << SQ81[i as usize]
}

/// 81升の集合を小さい番号から順に、升の添字で回す。
#[inline]
pub fn for_each81(mut m: u128, mut f: impl FnMut(u8)) {
    let mut lo = m as u64;
    while lo != 0 {
        f(IDX81[lo.trailing_zeros() as usize]);
        lo &= lo - 1;
    }
    m >>= 64;
    let mut hi = m as u64;
    while hi != 0 {
        f(IDX81[64 + hi.trailing_zeros() as usize]);
        hi &= hi - 1;
    }
}

// --- 駒 ---
// 駒の符号はcoreと同じOSL配列（ADR-0009）。成駒2〜7、生駒8〜15、後手は+16。

pub const EMPTY: u8 = 0;
/// 壁。coreで予約の1を使う。
pub const WALL: u8 = 1;
pub const WHITE_BIT: u8 = 16;

pub const PRO_PAWN: u8 = 2;
pub const HORSE: u8 = 6;
pub const DRAGON: u8 = 7;
pub const KING: u8 = 8;
pub const GOLD: u8 = 9;
pub const PAWN: u8 = 10;
pub const LANCE: u8 = 11;
pub const KNIGHT: u8 = 12;
pub const SILVER: u8 = 13;
pub const BISHOP: u8 = 14;
pub const ROOK: u8 = 15;

/// 手駒の駒種（SFENの表記の順）。
pub const HAND_KINDS: [u8; 7] = [ROOK, BISHOP, GOLD, SILVER, KNIGHT, LANCE, PAWN];

/// 駒打ちを生成する駒種の順。coreの `PieceType::HAND_KINDS` と同じ。
pub const DROP_KINDS: [u8; 7] = [GOLD, PAWN, LANCE, KNIGHT, SILVER, BISHOP, ROOK];

#[inline]
pub const fn make_piece(c: Color, pt: u8) -> u8 {
    pt | ((c as u8) << 4)
}

#[inline]
pub const fn piece_type(p: u8) -> u8 {
    p & 15
}

#[inline]
pub const fn piece_color(p: u8) -> Color {
    if p & WHITE_BIT != 0 {
        Color::White
    } else {
        Color::Black
    }
}

/// 成れる駒種か（歩・香・桂・銀・角・飛）。
#[inline]
pub const fn can_promote(pt: u8) -> bool {
    pt >= PAWN
}

/// 手駒の配列の添字（金9〜飛15 → 1〜7）。
#[inline]
pub const fn hand_index(pt: u8) -> usize {
    (pt - 8) as usize
}

/// 色cの駒が、中身vの升へ動けるか（空き升か相手の駒）。壁と自駒はfalse。
pub const CAN_MOVE_TO: [[bool; 32]; 2] = {
    let mut t = [[false; 32]; 2];
    let mut v = 0;
    while v < 32 {
        let is_piece = v >= 2 && (v & 15) >= 2;
        t[0][v] = v == 0 || (is_piece && v >= 16);
        t[1][v] = v == 0 || (is_piece && v < 16);
        v += 1;
    }
    t
};

/// 駒1種の利きの形。短い利き（1升の差分）と、飛び利きの方向ビット。
#[derive(Copy, Clone)]
pub struct PieceMoves {
    pub short: [i8; 8],
    pub n_short: u8,
    pub long: u8,
}

const fn pm(short: &[i8], long: u8) -> PieceMoves {
    let mut s = [0i8; 8];
    let mut i = 0;
    while i < short.len() {
        s[i] = short[i];
        i += 1;
    }
    PieceMoves {
        short: s,
        n_short: short.len() as u8,
        long,
    }
}

const fn bit(d: usize) -> u8 {
    1 << d
}

const GOLD_STEPS: [i8; 6] = [-1, 1, 11, -11, 10, -12];
const ORTH: u8 = bit(UP) | bit(DOWN) | bit(LEFT) | bit(RIGHT);
const DIAG: u8 = bit(UL) | bit(DR) | bit(UR) | bit(DL);

/// 先手の駒種ごとの利き。
const fn black_moves(pt: u8) -> PieceMoves {
    match pt {
        PAWN => pm(&[-1], 0),
        LANCE => pm(&[], bit(UP)),
        KNIGHT => pm(&[9, -13], 0),
        SILVER => pm(&[-1, 10, -12, 12, -10], 0),
        GOLD | 2..=5 => pm(&GOLD_STEPS, 0),
        KING => pm(&[-1, 1, 11, -11, 10, -10, -12, 12], 0),
        BISHOP => pm(&[], DIAG),
        ROOK => pm(&[], ORTH),
        HORSE => pm(&[-1, 1, 11, -11], DIAG),
        DRAGON => pm(&[10, -10, -12, 12], ORTH),
        _ => pm(&[], 0),
    }
}

/// 駒の符号（0〜31）ごとの利き。後手は先手の差分の符号を反転し、方向を逆にする。
pub const MOVES: [PieceMoves; 32] = {
    let mut t = [pm(&[], 0); 32];
    let mut p = 0;
    while p < 32 {
        let pt = (p & 15) as u8;
        if pt >= 2 {
            let b = black_moves(pt);
            if p < 16 {
                t[p] = b;
            } else {
                let mut w = b;
                let mut i = 0;
                while i < 8 {
                    w.short[i] = -b.short[i];
                    i += 1;
                }
                let mut long = 0u8;
                let mut d = 0;
                while d < 8 {
                    if b.long & bit(d) != 0 {
                        long |= bit(opp(d));
                    }
                    d += 1;
                }
                w.long = long;
                t[p] = w;
            }
        }
        p += 1;
    }
    t
};

/// SFENの駒文字（大文字）。
pub fn sfen_of_type(pt: u8) -> &'static str {
    match pt {
        PAWN => "P",
        LANCE => "L",
        KNIGHT => "N",
        SILVER => "S",
        GOLD => "G",
        BISHOP => "B",
        ROOK => "R",
        KING => "K",
        2 => "+P",
        3 => "+L",
        4 => "+N",
        5 => "+S",
        HORSE => "+B",
        DRAGON => "+R",
        _ => "?",
    }
}

pub fn type_of_sfen(c: char) -> Option<u8> {
    Some(match c {
        'P' => PAWN,
        'L' => LANCE,
        'N' => KNIGHT,
        'S' => SILVER,
        'G' => GOLD,
        'B' => BISHOP,
        'R' => ROOK,
        'K' => KING,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_roundtrip() {
        for s in 0..81u8 {
            let i = IDX81[s as usize];
            assert_eq!(SQ81[i as usize], s);
            assert_eq!(file_of(i), s / 9);
            assert_eq!(rank_of(i), s % 9);
        }
        assert_eq!(SQ81.iter().filter(|&&v| v != 0xFF).count(), 81);
    }

    #[test]
    fn every_step_from_board_stays_in_array() {
        // 盤上のどの升から、どの駒の短い利き・どの方向へ2升進んでも、配列の内側にある
        for &i in IDX81.iter() {
            let i = i as i32;
            for m in MOVES.iter() {
                for k in 0..m.n_short as usize {
                    let t = i + m.short[k] as i32;
                    assert!((0..LEN as i32).contains(&t));
                }
            }
            for d in DELTA {
                let t = i + d as i32;
                assert!((0..LEN as i32).contains(&t));
            }
        }
    }

    #[test]
    fn opposite_dirs() {
        for d in 0..8 {
            assert_eq!(DELTA[d], -DELTA[opp(d)]);
            assert_eq!(DIR_OF_DELTA[(DELTA[d] + 12) as usize] as usize, d);
        }
    }
}
