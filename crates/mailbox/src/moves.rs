//! 指し手。升は配列盤の添字で持つ。
//!
//! bit 0..8: 行き先、bit 8..16: 移動元（駒打ちは打つ駒種）、bit 16: 成り、bit 17: 駒打ち、
//! bit 24..29: 移動後の駒（先後込み）。移動後の駒はcoreの `Move` と同じく、置換表から来た手を
//! `pseudo_legal` で確かめるときに、移動元の駒と突き合わせるために持つ。

use crate::board::{Color, SQ81, make_piece, sfen_of_type, usi_square};

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct Move(u32);

const PROMOTE: u32 = 1 << 16;
const DROP: u32 = 1 << 17;

impl Move {
    pub const NONE: Move = Move(0);

    #[inline]
    pub const fn normal(from: u8, to: u8, promote: bool, piece_after: u8) -> Move {
        let v = to as u32 | ((from as u32) << 8) | ((piece_after as u32) << 24);
        Move(if promote { v | PROMOTE } else { v })
    }

    #[inline]
    pub const fn drop(pt: u8, to: u8, c: Color) -> Move {
        Move(to as u32 | ((pt as u32) << 8) | DROP | ((make_piece(c, pt) as u32) << 24))
    }

    /// 移動後の駒（成りなら成駒、打ちなら打った駒）。
    #[inline]
    pub const fn piece_after(self) -> u8 {
        ((self.0 >> 24) & 0x1F) as u8
    }

    /// 移動前の駒。
    #[inline]
    pub const fn piece_before(self) -> u8 {
        if self.is_promote() {
            self.piece_after() | 8
        } else {
            self.piece_after()
        }
    }

    #[inline]
    pub const fn to(self) -> u8 {
        self.0 as u8
    }

    #[inline]
    pub const fn from(self) -> u8 {
        (self.0 >> 8) as u8
    }

    /// 打つ駒種（駒打ちのとき）。
    #[inline]
    pub const fn drop_type(self) -> u8 {
        (self.0 >> 8) as u8
    }

    #[inline]
    pub const fn is_drop(self) -> bool {
        self.0 & DROP != 0
    }

    #[inline]
    pub const fn is_promote(self) -> bool {
        self.0 & PROMOTE != 0
    }

    /// 置換表に入れるcoreの `Move16` のビット列。升を81升の通し番号に直し、coreと同じ詰め方にする。
    ///
    /// 下位から行き先7bit・移動元7bit（駒打ちは駒種）・成り・駒打ちである。
    #[inline]
    pub const fn to_move16(self) -> u16 {
        let to = SQ81[self.to() as usize] as u16;
        if self.is_drop() {
            to | ((self.drop_type() as u16) << 7) | 0x8000
        } else {
            let from = SQ81[self.from() as usize] as u16;
            let v = to | (from << 7);
            if self.is_promote() { v | 0x4000 } else { v }
        }
    }

    /// USI表記（例: "7g7f"、"8h2b+"、"P*5e"）。
    pub fn to_usi(self) -> String {
        if self.is_drop() {
            format!(
                "{}*{}",
                sfen_of_type(self.drop_type()),
                usi_square(self.to())
            )
        } else {
            let mut s = format!("{}{}", usi_square(self.from()), usi_square(self.to()));
            if self.is_promote() {
                s.push('+');
            }
            s
        }
    }
}
