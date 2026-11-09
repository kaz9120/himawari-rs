//! 利き数の段階の集合と、升の語から特徴の添字を引く表。
//!
//! 差分計算（`nnue_acc`）は升ごとの語を祖先と比べ、変わった升の行だけを
//! 足し引きする。その語と添字の対応をここで持つ。

#[allow(clippy::wildcard_imports)]
use super::*;

/// 色ごとの利き数を、1枚以上・2枚以上・3枚以上の3枚の集合で持つ。
///
/// 特徴は利き数を3で打ち切るので、升ごとの数を持たなくても段階は決まる。
/// 駒ごとに利き先の集合を飽和カウンタへ足すので、升を1つずつ数えずに済む。
/// 親の段の値と排他的論理和を取れば、段階が変わった升の集合になる。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EffectLevels {
    /// `ge[色][k]` は、その色の利きが k+1 枚以上ある升
    pub ge: [[Bitboard; 3]; 2],
}

impl EffectLevels {
    pub const EMPTY: EffectLevels = EffectLevels {
        ge: [[Bitboard::EMPTY; 3]; 2],
    };

    /// 局面から作る。数え方は `effect_counts` と同じで、一致はテストで確かめる。
    ///
    /// 駒種ごとに集合から引く。駒ごとに `attacks` の分岐を
    /// 通らないので、駒種の分岐の外れが減る。
    pub fn build(pos: &Position) -> EffectLevels {
        let occ = pos.occupied();
        let mut l = EffectLevels::EMPTY;
        for c in [Color::Black, Color::White] {
            // 歩の利き先は駒ごとに別の升なので、集合のまま1段ずらして1回で足せる
            let pawns = pos.pieces(c, PieceType::PAWN);
            l.add(
                c,
                match c {
                    Color::Black => pawns.shift_up(),
                    Color::White => pawns.shift_down(),
                },
            );
            for sq in pos.pieces(c, PieceType::KNIGHT) {
                l.add(c, knight_attacks(c, sq));
            }
            for sq in pos.pieces(c, PieceType::SILVER) {
                l.add(c, silver_attacks(c, sq));
            }
            for sq in pos.golds(c) {
                l.add(c, gold_attacks(c, sq));
            }
            // 玉の無い局面（詰将棋など）もあるので、集合から引く
            for sq in pos.pieces(c, PieceType::KING) {
                l.add(c, king_attacks(sq));
            }
            for sq in pos.pieces(c, PieceType::LANCE) {
                l.add(c, lance_attacks(c, sq, occ));
            }
            for sq in pos.pieces(c, PieceType::BISHOP) {
                l.add(c, bishop_attacks(sq, occ));
            }
            for sq in pos.pieces(c, PieceType::ROOK) {
                l.add(c, rook_attacks(sq, occ));
            }
            for sq in pos.pieces(c, PieceType::HORSE) {
                l.add(c, bishop_attacks(sq, occ) | king_attacks(sq));
            }
            for sq in pos.pieces(c, PieceType::DRAGON) {
                l.add(c, rook_attacks(sq, occ) | king_attacks(sq));
            }
        }
        l
    }

    /// 色ごとの「k+1枚以上」の升の集合（81升の通し番号のビット）から作る。
    /// 配列盤が升ごとの利き数から作った集合を受け取る口である。
    #[inline]
    pub fn from_bits(bits: [[u128; 3]; 2]) -> EffectLevels {
        let mut l = EffectLevels::EMPTY;
        for (c, g) in bits.iter().enumerate() {
            for (k, &b) in g.iter().enumerate() {
                l.ge[c][k] = Bitboard::from_bits(b);
            }
        }
        l
    }

    #[inline(always)]
    fn add(&mut self, c: Color, x: Bitboard) {
        let g = &mut self.ge[c.index()];
        g[2] |= g[1] & x;
        g[1] |= g[0] & x;
        g[0] |= x;
    }

    /// 升sqへの色cの利き数を、3で打ち切って返す。
    #[inline]
    pub fn level(&self, c: Color, sq: Square) -> u8 {
        let g = &self.ge[c.index()];
        u8::from(g[0].test(sq)) + u8::from(g[1].test(sq)) + u8::from(g[2].test(sq))
    }

    /// 2つの局面で、どちらかの色の段階が違う升。
    #[inline]
    pub fn diff(&self, other: &EffectLevels) -> Bitboard {
        let mut d = Bitboard::EMPTY;
        for c in 0..2 {
            for k in 0..3 {
                d |= self.ge[c][k] ^ other.ge[c][k];
            }
        }
        d
    }
}

/// 升の中身と両色の利き数を、視点に依らない形で1語に詰める。
///
/// 上位が駒（0は空き、先手の14種を1〜14、後手を15〜28）、下位4ビットが
/// 先手と後手の利き数（各2ビット、3で打ち切り）である。視点ごとの特徴の
/// indexは `effect_code_index` で戻す。
#[inline]
pub fn effect_code<B: crate::nnue_acc::EvalBoard + ?Sized>(
    pos: &B,
    levels: &EffectLevels,
    sq: Square,
) -> u16 {
    let pc = pos.piece_on(sq);
    let p = if pc.is_empty() {
        0
    } else {
        let kind = u16::from(pc.piece_type().0) - 1;
        if pc.color() == Color::Black {
            kind
        } else {
            kind + 14
        }
    };
    p << 4
        | u16::from(levels.level(Color::Black, sq)) << 2
        | u16::from(levels.level(Color::White, sq))
}

/// `effect_code` の語を、視点cの特徴のindexへ戻す。`effect_active` と同じ値になる。
///
/// 語は `駒 << 4 | 先手の段階 << 2 | 後手の段階` で、`effect_index` の升より下の桁と
/// 同じ並びになる。先手の視点は升の起点に語を足すだけで済む。後手の視点は、
/// 自他を入れ替えた語を表で引く。
#[inline(always)]
pub fn effect_code_index(c: Color, sq: Square, code: u16) -> u32 {
    const PER_SQ: u32 = (EFF_PIECES * EFF_LEVELS * EFF_LEVELS) as u32;
    match c {
        Color::Black => sq.index() as u32 * PER_SQ + u32::from(code),
        Color::White => {
            sq.inv().index() as u32 * PER_SQ + u32::from(WHITE_CODE[usize::from(code) & 511])
        }
    }
}

/// 後手の視点の語。駒の先後と、両色の段階を入れ替える。
static WHITE_CODE: [u16; 512] = {
    let mut t = [0u16; 512];
    let mut code = 0;
    while code < 512 {
        let p = code >> 4;
        let (b, w) = ((code >> 2) & 3, code & 3);
        let p = if p == 0 {
            0
        } else if p <= 14 {
            p + 14
        } else {
            p - 14
        };
        t[code] = ((p << 4) | (w << 2) | b) as u16;
        code += 1;
    }
    t
};

/// 側の入力がF2と最小の攻め駒の組（`-f2_minatt`）で、両方を升の語の差分で持つ構成か。
///
/// 升の語は `nnue_acc::EvalBoard::side_codes` が作る16ビットで、下位9ビットがF2の語
/// （`effect_code`）、上位7ビットが `先手の階級 * 11 + 後手の階級` である。
pub const SIDE_MINATT_INC: bool = EFF_GENERIC;

/// F2の表の行数。最小の攻め駒の行はこの後ろに続く（`side_feat::ROWS` の順）。
const F2_ROWS: u32 = (81 * EFF_PIECES * EFF_LEVELS * EFF_LEVELS) as u32;
/// 最小の攻め駒の階級の数（`side_feat` と同じ）。
const MINATT_CLASSES: u32 = 11;
/// 最小の攻め駒の表で、1升が持つ行数。
const MINATT_PER_SQ: u32 = EFF_PIECES as u32 * MINATT_CLASSES * MINATT_CLASSES;

/// `side_codes` の語のうちF2の部分を、視点cの特徴のindexへ戻す。
#[inline(always)]
pub fn side_f2_index(c: Color, sq: Square, code: u16) -> u32 {
    effect_code_index(c, sq, code & 0x1FF)
}

/// `side_codes` の語のうち最小の攻め駒の部分を、視点cの特徴のindexへ戻す。
/// `side_feat` の `KIND_MINATT` と同じ値になる。
#[inline(always)]
pub fn side_minatt_index(c: Color, sq: Square, code: u16) -> u32 {
    let k = usize::from(code >> 4);
    match c {
        Color::Black => {
            let (p, m) = (k as u32 & 31, k as u32 >> 5);
            F2_ROWS + sq.index() as u32 * MINATT_PER_SQ + p * MINATT_CLASSES * MINATT_CLASSES + m
        }
        Color::White => {
            F2_ROWS + sq.inv().index() as u32 * MINATT_PER_SQ + u32::from(WHITE_MINATT[k & 4095])
        }
    }
}

/// 後手の視点の、升の中で閉じた最小の攻め駒の行。添字は `語 >> 4`（駒5ビット＋階級の組7ビット）で、
/// 駒の先後と、両者の階級を入れ替える。
static WHITE_MINATT: [u16; 4096] = {
    let mut t = [0u16; 4096];
    let mut k = 0;
    while k < 4096 {
        let (p, m) = (k & 31, k >> 5);
        if p <= 28 && m < 121 {
            let p = if p == 0 {
                0
            } else if p <= 14 {
                p + 14
            } else {
                p - 14
            };
            let (b, w) = (m / 11, m % 11);
            t[k] = (p * 121 + w * 11 + b) as u16;
        }
        k += 1;
    }
    t
};

/// `SIDE_MINATT_INC` の構成で、側のaccumulatorが取りうる値の上限（絶対値）を重みから見積もる。
///
/// 1升に立つのはF2と最小の攻め駒の1つずつなので、升ごとに各表の最大の絶対値を足す。
/// i16に収まれば、差分の経路をi16の巡回加算で持っても、i32で作り直す値と一致する。
/// 読み込みでこの値を検査する。
pub fn side_minatt_acc_bound(net: &NnueNetwork) -> i64 {
    if !SIDE_MINATT_INC {
        unreachable!("f2_minatt の構成でだけ呼ぶ");
    }
    const WIDTH: usize = if EFF_OUT == 0 { 1 } else { EFF_OUT };
    let mut sum: Vec<i64> = net.eff_b.iter().map(|&b| i64::from(b).abs()).collect();
    let f2 = F2_ROWS as usize * WIDTH;
    let blocks = net.eff_w[..f2]
        .chunks(EFF_PIECES * EFF_LEVELS * EFF_LEVELS * WIDTH)
        .chain(net.eff_w[f2..].chunks(MINATT_PER_SQ as usize * WIDTH));
    for rows in blocks {
        let mut most = vec![0i64; EFF_OUT];
        for row in rows.chunks(WIDTH) {
            for (m, &w) in most.iter_mut().zip(row) {
                *m = (*m).max(i64::from(w).abs());
            }
        }
        for (s, m) in sum.iter_mut().zip(&most) {
            *s += m;
        }
    }
    sum.into_iter().max().unwrap_or(0)
}
