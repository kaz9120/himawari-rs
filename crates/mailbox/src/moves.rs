//! 指し手。升は配列盤の添字で持つ。
//!
//! bit 0..8: 行き先、bit 8..16: 移動元（駒打ちは打つ駒種）、bit 16: 成り、bit 17: 駒打ち。

use crate::board::{sfen_of_type, usi_square};

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct Move(u32);

const PROMOTE: u32 = 1 << 16;
const DROP: u32 = 1 << 17;

impl Move {
    pub const NONE: Move = Move(0);

    #[inline]
    pub const fn normal(from: u8, to: u8, promote: bool) -> Move {
        let v = to as u32 | ((from as u32) << 8);
        Move(if promote { v | PROMOTE } else { v })
    }

    #[inline]
    pub const fn drop(pt: u8, to: u8) -> Move {
        Move(to as u32 | ((pt as u32) << 8) | DROP)
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
