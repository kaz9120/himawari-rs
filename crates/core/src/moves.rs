//! 指し手のエンコーディング（ADR-0012）。
//!
//! 下位16bit: to(7) | from(7) | promote(1) | drop(1)。
//! 駒打ちのfromフィールドは駒種（OSL配列 9〜15）。
//! 上位: bit 16..=20 に移動後の駒（先後込み）。

use std::mem::MaybeUninit;

use crate::piece::{Piece, PieceType};
use crate::types::{Color, Square};

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Move(u32);

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Move16(pub u16);

const PROMOTE_BIT: u32 = 1 << 14;
const DROP_BIT: u32 = 1 << 15;

impl Move {
    pub const NONE: Move = Move(0);
    pub const NULL: Move = Move((1 << 7) | 1);
    pub const WIN: Move = Move((2 << 7) | 2);
    pub const RESIGN: Move = Move((3 << 7) | 3);

    /// 盤上の移動。pieceは移動後の駒（成りなら成駒）。
    #[inline]
    pub const fn new_move(from: Square, to: Square, promote: bool, piece_after: Piece) -> Move {
        let mut v = to.index() as u32 | ((from.index() as u32) << 7);
        if promote {
            v |= PROMOTE_BIT;
        }
        Move(v | ((piece_after.0 as u32) << 16))
    }

    /// 駒打ち。
    #[inline]
    pub const fn new_drop(pt: PieceType, to: Square, c: Color) -> Move {
        let v = to.index() as u32 | ((pt.0 as u32) << 7) | DROP_BIT;
        Move(v | ((Piece::new(c, pt).0 as u32) << 16))
    }

    #[inline]
    pub const fn to(self) -> Square {
        Square::from_index((self.0 & 0x7F) as u8)
    }

    #[inline]
    pub const fn from_sq(self) -> Square {
        debug_assert!(!self.is_drop());
        Square::from_index(((self.0 >> 7) & 0x7F) as u8)
    }

    #[inline]
    pub const fn is_drop(self) -> bool {
        self.0 & DROP_BIT != 0
    }

    #[inline]
    pub const fn is_promote(self) -> bool {
        self.0 & PROMOTE_BIT != 0
    }

    /// 打った駒の種類。
    #[inline]
    pub const fn drop_piece_type(self) -> PieceType {
        debug_assert!(self.is_drop());
        PieceType(((self.0 >> 7) & 0x7F) as u8)
    }

    /// 移動後の駒（成りなら成駒、打ちなら打った駒）。
    #[inline]
    pub const fn piece_after(self) -> Piece {
        Piece(((self.0 >> 16) & 0x1F) as u8)
    }

    /// 移動前の駒。
    #[inline]
    pub const fn piece_before(self) -> Piece {
        if self.is_promote() {
            self.piece_after().unpromote()
        } else {
            self.piece_after()
        }
    }

    /// 特殊値（NONE/NULL/WIN/RESIGN）か。
    #[inline]
    pub const fn is_special(self) -> bool {
        let v = self.0 & 0xFFFF;
        v & DROP_BIT == 0 && (v & 0x7F) == ((v >> 7) & 0x7F)
    }

    #[inline]
    pub const fn to_move16(self) -> Move16 {
        Move16((self.0 & 0xFFFF) as u16)
    }

    /// USI表記（例: "7g7f", "8h2b+", "P*5e"）。
    pub fn to_usi(self) -> String {
        if self.is_special() {
            return match self {
                Move::NULL => "0000".to_string(),
                Move::WIN => "win".to_string(),
                Move::RESIGN => "resign".to_string(),
                _ => "none".to_string(),
            };
        }
        if self.is_drop() {
            let pt = self.drop_piece_type();
            format!("{}*{}", pt.to_sfen().unwrap(), self.to().to_usi())
        } else {
            let mut s = format!("{}{}", self.from_sq().to_usi(), self.to().to_usi());
            if self.is_promote() {
                s.push('+');
            }
            s
        }
    }
}

impl Move16 {
    pub const NONE: Move16 = Move16(0);

    /// USI表記からのパース。盤面情報がないため駒情報なしのMove16を返す。
    /// 完全なMoveへの復元はPosition側で行う。
    pub fn from_usi(s: &str) -> Option<Move16> {
        let b = s.as_bytes();
        if b.len() < 4 {
            return None;
        }
        if b[1] == b'*' {
            // 駒打ち
            let pt = PieceType::from_sfen_char(b[0] as char)?;
            if !pt.is_piece() || pt == PieceType::KING {
                return None;
            }
            let to = Square::from_usi(std::str::from_utf8(&b[2..4]).ok()?)?;
            Some(Move16(
                (to.index() as u16) | ((pt.0 as u16) << 7) | (DROP_BIT as u16),
            ))
        } else {
            let from = Square::from_usi(std::str::from_utf8(&b[0..2]).ok()?)?;
            let to = Square::from_usi(std::str::from_utf8(&b[2..4]).ok()?)?;
            let promote = b.len() == 5 && b[4] == b'+';
            if from == to {
                return None;
            }
            let mut v = (to.index() as u16) | ((from.index() as u16) << 7);
            if promote {
                v |= PROMOTE_BIT as u16;
            }
            Some(Move16(v))
        }
    }
}

/// 指し手バッファの容量。将棋の最大合法手数593を上回る。
const MOVE_LIST_CAP: usize = 608;

/// 指し手バッファ。
///
/// 生成のたびに全要素を埋めると1回あたり2.4KBの書き込みになり、
/// 書いた値は一度も読まれない（ADR-0101）。未初期化のまま確保し、
/// `push` した先頭 `len` 要素だけを有効として扱う。
///
/// 不変条件: `moves[..len]` は初期化済みである。`len` を増やすのは
/// `push` だけで、`clear` は0へ戻すだけである。
pub struct MoveList {
    moves: [MaybeUninit<Move>; MOVE_LIST_CAP],
    len: usize,
}

impl Default for MoveList {
    /// `len` だけを書き、`moves` は未初期化のままにする。
    ///
    /// `MoveList { moves: [MaybeUninit::uninit(); N], len: 0 }` と
    /// 構造体を値で組み立てると、LLVMが未初期化の配列を含む
    /// アグリゲートを定数へ畳み、2,440バイトのゼロクリアを生成する
    /// （ADR-0101）。フィールド単位で書けばこれが起きない。
    #[inline]
    fn default() -> Self {
        let mut this = MaybeUninit::<MoveList>::uninit();
        // SAFETY: lenを0にすれば型の不変条件（moves[..len]が初期化済み）
        // を満たす。movesは未初期化のままでよく、as_sliceは空を返す
        unsafe {
            (&raw mut (*this.as_mut_ptr()).len).write(0);
            this.assume_init()
        }
    }
}

impl MoveList {
    #[inline]
    pub fn push(&mut self, m: Move) {
        debug_assert!(self.len < MOVE_LIST_CAP);
        self.moves[self.len].write(m);
        self.len += 1;
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
    }

    #[inline]
    pub fn as_slice(&self) -> &[Move] {
        // SAFETY: 先頭len要素はpushで初期化済み（型の不変条件）。
        // MaybeUninit<Move>はMoveと同じレイアウトを持つ
        unsafe { std::slice::from_raw_parts(self.moves.as_ptr().cast::<Move>(), self.len) }
    }
}

impl<'a> IntoIterator for &'a MoveList {
    type Item = &'a Move;
    type IntoIter = std::slice::Iter<'a, Move>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{File, Rank};

    #[test]
    fn move_fields_roundtrip() {
        let from = Square::new(File(6), Rank(6)); // 7g
        let to = Square::new(File(6), Rank(5)); // 7f
        let pc = Piece::new(Color::Black, PieceType::PAWN);
        let m = Move::new_move(from, to, false, pc);
        assert_eq!(m.from_sq(), from);
        assert_eq!(m.to(), to);
        assert!(!m.is_drop() && !m.is_promote() && !m.is_special());
        assert_eq!(m.piece_after(), pc);
        assert_eq!(m.piece_before(), pc);
        assert_eq!(m.to_usi(), "7g7f");
    }

    #[test]
    fn promote_move() {
        let from = Square::from_usi("8h").unwrap();
        let to = Square::from_usi("2b").unwrap();
        let horse = Piece::new(Color::Black, PieceType::HORSE);
        let m = Move::new_move(from, to, true, horse);
        assert!(m.is_promote());
        assert_eq!(m.piece_after(), horse);
        assert_eq!(
            m.piece_before(),
            Piece::new(Color::Black, PieceType::BISHOP)
        );
        assert_eq!(m.to_usi(), "8h2b+");
    }

    #[test]
    fn drop_move() {
        let to = Square::from_usi("5e").unwrap();
        let m = Move::new_drop(PieceType::PAWN, to, Color::White);
        assert!(m.is_drop());
        assert_eq!(m.drop_piece_type(), PieceType::PAWN);
        assert_eq!(m.piece_after(), Piece::new(Color::White, PieceType::PAWN));
        assert_eq!(m.to_usi(), "P*5e");
    }

    #[test]
    fn special_values_do_not_collide() {
        for m in [Move::NONE, Move::NULL, Move::WIN, Move::RESIGN] {
            assert!(m.is_special());
        }
        let normal = Move::new_move(
            Square::from_index(10),
            Square::from_index(11),
            false,
            Piece::new(Color::Black, PieceType::GOLD),
        );
        assert!(!normal.is_special());
        let drop = Move::new_drop(PieceType::GOLD, Square::from_index(0), Color::Black);
        assert!(!drop.is_special());
    }

    /// 未初期化バッファでもpushした範囲だけが見えること（ADR-0101）。
    #[test]
    fn move_list_exposes_only_pushed_moves() {
        let mut list = MoveList::default();
        assert!(list.is_empty());
        assert_eq!(list.len(), 0);
        assert_eq!(list.as_slice(), &[]);

        let a = Move::new_drop(
            PieceType::PAWN,
            Square::from_usi("5e").unwrap(),
            Color::Black,
        );
        let b = Move::new_drop(
            PieceType::GOLD,
            Square::from_usi("1a").unwrap(),
            Color::White,
        );
        list.push(a);
        list.push(b);
        assert_eq!(list.len(), 2);
        assert_eq!(list.as_slice(), &[a, b]);
        assert_eq!(list.into_iter().copied().collect::<Vec<_>>(), vec![a, b]);

        // clear後は再び空。前に書いた値は見えない
        list.clear();
        assert!(list.is_empty());
        assert_eq!(list.as_slice(), &[]);
        list.push(b);
        assert_eq!(list.as_slice(), &[b]);

        // 容量いっぱいまで積んでも先頭から順に読める
        let mut full = MoveList::default();
        for i in 0..MOVE_LIST_CAP {
            full.push(Move::new_drop(
                PieceType::PAWN,
                Square::from_index((i % 81) as u8),
                Color::Black,
            ));
        }
        assert_eq!(full.len(), MOVE_LIST_CAP);
        assert_eq!(full.as_slice()[0], full.as_slice()[81]);
    }

    #[test]
    fn move16_usi_parse() {
        let m = Move16::from_usi("7g7f").unwrap();
        assert_eq!(m.0 & 0x7F, Square::from_usi("7f").unwrap().index() as u16);
        assert!(Move16::from_usi("P*5e").is_some());
        assert!(Move16::from_usi("8h2b+").is_some());
        assert!(Move16::from_usi("K*5e").is_none());
        assert!(Move16::from_usi("7g7g").is_none());
        assert!(Move16::from_usi("xx").is_none());
    }
}
