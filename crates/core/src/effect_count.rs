//! 色ごと・升ごとの利き数と飛び利きの方向を持つバイト表（ADR-0239の案B）。
//!
//! 技巧の `ExtendedBoard` に倣い、1升あたり利き数1バイトと、その升へ
//! 届いている飛び利きの方向1バイトを持つ。探索では手ごとに前の表を複写し、
//! 戻しは複写した段を捨てるだけにする（`SearchBoard` が持つ）。
//!
//! 飛び利きの方向は `attacks::ray_attacks` の並び（0〜7）で持つ。ビットdが
//! 立っていれば、方向dへ進む飛び利きがその升に届いている。同じ色で
//! 同じ方向から届く飛び利きは、手前の駒が遮るので高々1本である。
//!
//! 差分は次の順で当てる。
//!
//! 1. 動かした駒の利きを移動元から外す（指す前の占有）
//! 2. 取られた駒の利きを移動先から外す（指す前の占有）
//! 3. 移動先が空いていたなら、移動先へ届いていた飛び利きの先を切る
//!    （指す前の占有で、移動先の奥から最初の駒まで）
//! 4. 移動元へ届いていた飛び利きを、移動元の奥へ延ばす（指した後の占有）
//! 5. 動かした駒の利きを移動先に足す（指した後の占有）
//!
//! 3を4より先に行うのは、移動先・移動元の順に並ぶ線の上で、移動元へ
//! 届いていた飛び利きが移動先で止まるからである。3がその升の方向ビットを
//! 消すので、4で誤って延ばさない。

use crate::attacks::ray_attacks;
#[cfg(test)]
use crate::attacks::{
    bishop_attacks, gold_attacks, king_attacks, knight_attacks, pawn_attacks, rook_attacks,
    silver_attacks,
};
use crate::bitboard::Bitboard;
use crate::moves::Move;
use crate::piece::{Piece, PieceType};
use crate::position::Position;
use crate::types::{Color, Square};

/// 1色ぶんの升の数。81升をSIMDの幅（16・32）の倍数へ切り上げる
const LANES: usize = 96;

/// 利き数の行の前に置く詰め物。短い利きのパターンを升の前後11升の窓で
/// まとめて足すので（ADR-0240の2回目）、盤の端の升でも窓が行からはみ出さない
const PAD: usize = 16;
/// 利き数の1色ぶんの長さ。詰め物16＋81升＋窓のはみ出し20を16の倍数へ切り上げる
const COUNT_LANES: usize = 128;
/// 短い利きのパターンの窓。升sqの窓は行の `PAD + sq - WINDOW_BACK` から32バイト
const WINDOW: usize = 32;
const WINDOW_BACK: usize = 11;

/// 短い利きの駒の種類の数。歩・桂・銀・金は先後で2つずつ、玉・馬の縦横・竜の斜めは1つずつ
const SHORT_KINDS: usize = 11;

/// 短い利きの升の差（筋の差, 段の差）。先手の前は段の差−1である。
/// 後手は段の差の符号を反転して使う
const fn short_deltas(kind: usize) -> &'static [(i32, i32)] {
    match kind / 2 {
        0 => &[(0, -1)],
        1 => &[(-1, -2), (1, -2)],
        2 => &[(-1, -1), (0, -1), (1, -1), (-1, 1), (1, 1)],
        3 => &[(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (0, 1)],
        _ => match kind {
            8 => &[(0, -1), (0, 1), (-1, 0), (1, 0)],
            9 => &[(-1, -1), (1, -1), (-1, 1), (1, 1)],
            _ => &[
                (-1, -1),
                (0, -1),
                (1, -1),
                (-1, 0),
                (1, 0),
                (-1, 1),
                (0, 1),
                (1, 1),
            ],
        },
    }
}

/// 種類と段ごとの短い利きのパターン。窓の中で利く升に1が立つ。
/// 段をはみ出す升は落とす。筋をはみ出す升は行の詰め物へ落ちるので、
/// 足し引きが釣り合う限り盤の升の値を汚さない。
const fn build_short_patterns() -> [[[u8; WINDOW]; 9]; SHORT_KINDS] {
    let mut t = [[[0u8; WINDOW]; 9]; SHORT_KINDS];
    let mut kind = 0;
    while kind < SHORT_KINDS {
        let deltas = short_deltas(kind);
        // 歩・桂・銀・金の後手（奇数）は段の差を反転する
        let flip = kind < 8 && kind % 2 == 1;
        let mut rank = 0;
        while rank < 9 {
            let mut i = 0;
            while i < deltas.len() {
                let (df, dr0) = deltas[i];
                let dr = if flip { -dr0 } else { dr0 };
                let r = rank as i32 + dr;
                if r >= 0 && r < 9 {
                    let off = WINDOW_BACK as i32 + df * 9 + dr;
                    t[kind][rank][off as usize] = 1;
                }
                i += 1;
            }
            rank += 1;
        }
        kind += 1;
    }
    t
}

static SHORT_PATTERNS: [[[u8; WINDOW]; 9]; SHORT_KINDS] = build_short_patterns();

/// 短い利きのパターンの種類。飛び利きしか持たない駒（香・角・飛）は `None`。
#[inline(always)]
fn short_kind(pc: Piece) -> Option<usize> {
    let c = pc.color().index();
    match pc.piece_type() {
        PieceType::LANCE | PieceType::BISHOP | PieceType::ROOK => None,
        PieceType::PAWN => Some(c),
        PieceType::KNIGHT => Some(2 + c),
        PieceType::SILVER => Some(4 + c),
        PieceType::HORSE => Some(8),
        PieceType::DRAGON => Some(9),
        PieceType::KING => Some(10),
        // 金と、金の動きをする成駒
        _ => Some(6 + c),
    }
}

/// 利き数と飛び利きの方向のバイト表。
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(C, align(64))]
pub struct EffectCounts {
    /// 色cの駒が升へ利いている数。玉の利きも数える
    /// 升sqの値は `count[c][PAD + sq]` にある
    count: [[u8; COUNT_LANES]; 2],
    /// 色cの飛び利きが升へ届いている方向（ビットdが方向d）
    dirs: [[u8; LANES]; 2],
}

impl std::fmt::Debug for EffectCounts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectCounts")
            .field(
                "count",
                &[&self.count[0][PAD..PAD + 81], &self.count[1][PAD..PAD + 81]],
            )
            .field("dirs", &[&self.dirs[0][..81], &self.dirs[1][..81]])
            .finish()
    }
}

impl Default for EffectCounts {
    fn default() -> Self {
        EffectCounts {
            count: [[0; COUNT_LANES]; 2],
            dirs: [[0; LANES]; 2],
        }
    }
}

/// 駒の飛び利きの方向。香は前の1方向、角・馬は斜め4方向、飛・竜は縦横4方向。
#[inline(always)]
fn ray_dirs(pc: Piece) -> u8 {
    match pc.piece_type() {
        PieceType::LANCE => {
            if pc.color() == Color::Black {
                1 << 4
            } else {
                1 << 0
            }
        }
        PieceType::BISHOP | PieceType::HORSE => 0b1010_1010,
        PieceType::ROOK | PieceType::DRAGON => 0b0101_0101,
        _ => 0,
    }
}

/// 駒の飛び利きでない利き（隣への1升など）。差分はパターンで当てるので、
/// テストでパターンとの一致を確かめるときだけ使う。
#[cfg(test)]
fn short_attacks(pc: Piece, sq: Square) -> Bitboard {
    match pc.piece_type() {
        PieceType::LANCE | PieceType::BISHOP | PieceType::ROOK => Bitboard::EMPTY,
        // 馬の縦横の1升、竜の斜めの1升。全升を占有にした飛び利きは隣の升だけになる
        PieceType::HORSE => king_attacks(sq) & !bishop_attacks(sq, Bitboard::ALL),
        PieceType::DRAGON => king_attacks(sq) & !rook_attacks(sq, Bitboard::ALL),
        PieceType::PAWN => pawn_attacks(pc.color(), sq),
        PieceType::KNIGHT => knight_attacks(pc.color(), sq),
        PieceType::SILVER => silver_attacks(pc.color(), sq),
        PieceType::KING => king_attacks(sq),
        // 金と、金の動きをする成駒
        _ => gold_attacks(pc.color(), sq),
    }
}

impl EffectCounts {
    /// 盤面から作り直す。差分更新の正しさは、これとの一致で確かめる。
    pub fn build(pos: &Position) -> EffectCounts {
        let mut t = EffectCounts::default();
        let occ = pos.occupied();
        for sq in occ {
            t.add_piece(pos.piece_on(sq), sq, occ);
        }
        t
    }

    /// 1段下の表を複写する。探索の進めごとに呼ぶので、`memcpy` の呼び出しを
    /// 避けて16バイトずつ読み書きする。
    #[inline(always)]
    pub fn copy_from(&mut self, src: &EffectCounts) {
        const WORDS: usize = std::mem::size_of::<EffectCounts>() / 16;
        let s = (src as *const EffectCounts).cast::<u128>();
        let d = (self as *mut EffectCounts).cast::<u128>();
        for i in 0..WORDS {
            // SAFETY: どちらも同じ型の値で、64バイト境界に置かれ、大きさは16の倍数。
            // 1語ずつの読み書きに固定し、`memcpy` の呼び出しへ畳ませない
            unsafe { d.add(i).write_volatile(s.add(i).read_volatile()) };
        }
    }

    /// 色cの駒が升sqへ利いている数。
    #[inline]
    pub fn count(&self, c: Color, sq: Square) -> u8 {
        self.count[c.index()][PAD + sq.index()]
    }

    /// 色cの飛び利きが升sqへ届いている方向のビット。
    #[inline]
    pub fn dirs(&self, c: Color, sq: Square) -> u8 {
        self.dirs[c.index()][sq.index()]
    }

    /// 差分の内側のループで引く升。升の番号は81未満、色は0か1なので、
    /// 境界検査を外す。
    #[inline(always)]
    fn count_mut(&mut self, c: usize, s: Square) -> &mut u8 {
        debug_assert!(c < 2 && s.index() < LANES);
        // SAFETY: c < 2 かつ PAD + s.index() < PAD + 81 <= COUNT_LANES
        unsafe {
            self.count
                .get_unchecked_mut(c)
                .get_unchecked_mut(PAD + s.index())
        }
    }

    #[inline(always)]
    fn dirs_mut(&mut self, c: usize, s: Square) -> &mut u8 {
        debug_assert!(c < 2 && s.index() < LANES);
        // SAFETY: c < 2 かつ s.index() < 81 <= LANES
        unsafe { self.dirs.get_unchecked_mut(c).get_unchecked_mut(s.index()) }
    }

    /// 駒の短い利きを、升の前後の32バイトの窓へパターンでまとめて足し引きする
    /// （ADR-0240の2回目）。升を1つずつ辿るループと分岐を消す。
    #[inline(always)]
    fn apply_short<const ADD: bool>(&mut self, c: usize, pc: Piece, sq: Square) {
        let Some(kind) = short_kind(pc) else {
            return;
        };
        let rank = sq.index() % 9;
        debug_assert!(c < 2 && kind < SHORT_KINDS);
        // SAFETY: kind < SHORT_KINDS、rank < 9。窓の先頭は PAD + sq - 11 >= 5、
        // 末尾は PAD + 80 - 11 + 32 = 117 <= COUNT_LANES なので行に収まる
        unsafe {
            let pat = SHORT_PATTERNS.get_unchecked(kind).get_unchecked(rank);
            let row = self
                .count
                .get_unchecked_mut(c)
                .as_mut_ptr()
                .add(PAD + sq.index() - WINDOW_BACK);
            let w = &mut *row.cast::<[u8; WINDOW]>();
            for i in 0..WINDOW {
                w[i] = if ADD {
                    w[i].wrapping_add(pat[i])
                } else {
                    w[i].wrapping_sub(pat[i])
                };
            }
        }
    }

    #[inline(always)]
    fn add_piece(&mut self, pc: Piece, sq: Square, occ: Bitboard) {
        let c = pc.color().index();
        self.apply_short::<true>(c, pc, sq);
        let mut dirs = ray_dirs(pc);
        while dirs != 0 {
            let d = dirs.trailing_zeros() as usize;
            dirs &= dirs - 1;
            self.extend(c, d, sq, occ);
        }
    }

    #[inline(always)]
    fn remove_piece(&mut self, pc: Piece, sq: Square, occ: Bitboard) {
        let c = pc.color().index();
        self.apply_short::<false>(c, pc, sq);
        let mut dirs = ray_dirs(pc);
        while dirs != 0 {
            let d = dirs.trailing_zeros() as usize;
            dirs &= dirs - 1;
            self.cut(c, d, sq, occ);
        }
    }

    /// 色cの方向dの飛び利きを、sqの奥から最初の駒まで足す。
    #[inline(always)]
    fn extend(&mut self, c: usize, d: usize, sq: Square, occ: Bitboard) {
        let bit = 1u8 << d;
        for s in ray_attacks(d, sq, occ) {
            *self.count_mut(c, s) += 1;
            *self.dirs_mut(c, s) |= bit;
        }
    }

    /// 色cの方向dの飛び利きを、sqの奥から最初の駒まで外す。
    #[inline(always)]
    fn cut(&mut self, c: usize, d: usize, sq: Square, occ: Bitboard) {
        let bit = 1u8 << d;
        for s in ray_attacks(d, sq, occ) {
            *self.count_mut(c, s) -= 1;
            *self.dirs_mut(c, s) &= !bit;
        }
    }

    /// 1手ぶん進めた盤面へ追従する。`captured` は取られた駒（なければ空）、
    /// `occ_before`・`occ_after` は指す前と指した後の占有である。局面そのものは
    /// 読まないので、手と占有を積んでおけば後から当てられる（ADR-0240の遅延更新）。
    pub fn update(&mut self, m: Move, captured: Piece, occ_before: Bitboard, occ_after: Bitboard) {
        let to = m.to();
        if !m.is_drop() {
            self.remove_piece(m.piece_before(), m.from_sq(), occ_before);
        }
        if captured != Piece::EMPTY {
            self.remove_piece(captured, to, occ_before);
        } else {
            // 空いていた移動先へ届いていた飛び利きは、移動先で止まる
            for c in 0..2 {
                let mut dirs = self.dirs[c][to.index()];
                while dirs != 0 {
                    let d = dirs.trailing_zeros() as usize;
                    dirs &= dirs - 1;
                    self.cut(c, d, to, occ_before);
                }
            }
        }
        if !m.is_drop() {
            // 移動元へ届いていた飛び利きは、空いた移動元の奥へ伸びる
            let from = m.from_sq();
            for c in 0..2 {
                let mut dirs = self.dirs[c][from.index()];
                while dirs != 0 {
                    let d = dirs.trailing_zeros() as usize;
                    dirs &= dirs - 1;
                    self.extend(c, d, from, occ_after);
                }
            }
        }
        self.add_piece(m.piece_after(), to, occ_after);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SFEN_STARTPOS;

    fn assert_counts_match(pos: &Position, t: &EffectCounts) {
        let occ = pos.occupied();
        for c in [Color::Black, Color::White] {
            for i in 0..Square::NB {
                let sq = Square::from_index(i as u8);
                let n = pos.attackers_to(c, sq, occ).count();
                assert_eq!(
                    u32::from(t.count(c, sq)),
                    n,
                    "利き数が食い違う: {sq:?}\n{}",
                    pos.to_sfen()
                );
            }
        }
    }

    /// 全合法手を深さdまで辿り、差分更新と作り直しを突き合わせる。
    fn walk(pos: &mut Position, t: &EffectCounts, depth: u32) -> u64 {
        assert_counts_match(pos, t);
        if depth == 0 {
            return 1;
        }
        let mut list = crate::moves::MoveList::default();
        crate::movegen::generate_legal(pos, true, &mut list);
        let mut n = 0;
        for &m in &list {
            let occ = pos.occupied();
            let captured = if m.is_drop() {
                Piece::EMPTY
            } else {
                pos.piece_on(m.to())
            };
            pos.do_move(m);
            let mut next = *t;
            next.update(m, captured, occ, pos.occupied());
            assert_eq!(
                next,
                EffectCounts::build(pos),
                "差分更新が作り直しと食い違う: 手={}\n{}",
                m.to_usi(),
                pos.to_sfen()
            );
            n += walk(pos, &next, depth - 1);
            pos.undo_move(m);
        }
        n
    }

    #[test]
    fn incremental_update_matches_rebuild() {
        for sfen in [
            SFEN_STARTPOS,
            "l6nl/5+P1gk/2np1S3/p1p4Pp/3P2Sp1/1PPb2P1P/P5GS1/R8/LN4bKL w GR5pnsg 1",
            "+R1G4nl/1g4+Ss1/1kspp2p1/ppp2pS1p/4n4/P4Gp1P/1P1PP1P2/1+n2K2R1/7NL w G2P2b2lp 24",
        ] {
            let mut pos = Position::from_sfen(sfen).expect("sfen");
            let t = EffectCounts::build(&pos);
            let n = walk(&mut pos, &t, 2);
            assert!(n > 500, "局面数が少なすぎる: {n}");
        }
    }

    /// 短い利きのパターンが、駒・升ごとの利きのBitboardと一致する。
    /// 盤の外へ出る分は行の詰め物へ落ちるので、盤の升だけを比べる。
    #[test]
    fn short_patterns_match_attacks() {
        use crate::piece::PieceType as P;
        for color in [Color::Black, Color::White] {
            for pt in [
                P::PAWN,
                P::KNIGHT,
                P::SILVER,
                P::GOLD,
                P::KING,
                P::HORSE,
                P::DRAGON,
                P::PRO_PAWN,
            ] {
                let pc = Piece::new(color, pt);
                for i in 0..Square::NB {
                    let sq = Square::from_index(i as u8);
                    let mut t = EffectCounts::default();
                    t.apply_short::<true>(color.index(), pc, sq);
                    let want = short_attacks(pc, sq);
                    for j in 0..Square::NB {
                        let s = Square::from_index(j as u8);
                        assert_eq!(
                            t.count(color, s),
                            u8::from(want.test(s)),
                            "{pc:?} {sq:?} -> {s:?}"
                        );
                    }
                }
            }
        }
    }

    /// 飛び利きの方向ビットは、その方向の飛び駒から届いている升だけに立つ。
    #[test]
    fn ray_directions_point_back_to_a_slider() {
        let sfen = "l6nl/5+P1gk/2np1S3/p1p4Pp/3P2Sp1/1PPb2P1P/P5GS1/R8/LN4bKL w GR5pnsg 1";
        let pos = Position::from_sfen(sfen).expect("sfen");
        let t = EffectCounts::build(&pos);
        let occ = pos.occupied();
        for c in [Color::Black, Color::White] {
            for i in 0..Square::NB {
                let sq = Square::from_index(i as u8);
                for d in 0..8 {
                    if t.dirs(c, sq) & (1 << d) == 0 {
                        continue;
                    }
                    // 逆向きに辿った最初の駒が、方向dの飛び利きを持つ色cの駒
                    // 逆向きの飛び利きは最初の駒で止まるので、占有との積は1升になる
                    let src = (ray_attacks(d ^ 4, sq, occ) & occ).lsb();
                    let pc = pos.piece_on(src);
                    assert_eq!(pc.color(), c);
                    assert!(ray_dirs(pc) & (1 << d) != 0, "{sq:?} 方向{d}");
                }
            }
        }
    }
}
