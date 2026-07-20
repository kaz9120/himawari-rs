//! PackedSfenValue互換の教師データ入出力（ADR-0038）。
//!
//! packed sfenは局面を256bit（32バイト）に収めるハフマン符号。
//! LSB-firstのビット列に、手番1bit・先手玉/後手玉の位置7bit×2・
//! 盤上駒（空1bit、歩4bit、香桂銀6bit、金6bit、角飛8bit）・
//! 手駒の順で並ぶ。符号表はやねうら王 extra/sfen_packer.cpp を
//! 正とする。対応範囲は平手由来の40駒全数の局面で、駒箱
//! （駒落ち）符号はエラーにする。

use crate::piece::{Piece, PieceType};
use crate::position::Position;
use crate::types::{Color, Square};

/// packed sfenのバイト数。
pub const PACKED_SFEN_BYTES: usize = 32;
/// PackedSfenValueレコードのバイト数。
pub const PSV_BYTES: usize = 40;

/// 教師データ1局面（やねうら王のPackedSfenValueと同一レイアウト）。
#[derive(Clone, Copy)]
pub struct PackedSfenValue {
    pub sfen: [u8; PACKED_SFEN_BYTES],
    /// 手番視点の評価値（cp）。
    pub score: i16,
    /// PVの初手（Move16形式）。指し手一致率の診断用。
    pub move16: u16,
    /// 初期局面からの手数。
    pub game_ply: u16,
    /// 手番側の最終勝敗（勝ち1、負け-1、引き分け0）。
    pub game_result: i8,
}

impl PackedSfenValue {
    pub fn from_bytes(b: &[u8; PSV_BYTES]) -> PackedSfenValue {
        let mut sfen = [0u8; PACKED_SFEN_BYTES];
        sfen.copy_from_slice(&b[..32]);
        PackedSfenValue {
            sfen,
            score: i16::from_le_bytes([b[32], b[33]]),
            move16: u16::from_le_bytes([b[34], b[35]]),
            game_ply: u16::from_le_bytes([b[36], b[37]]),
            game_result: b[38] as i8,
        }
    }

    pub fn to_bytes(&self) -> [u8; PSV_BYTES] {
        let mut b = [0u8; PSV_BYTES];
        b[..32].copy_from_slice(&self.sfen);
        b[32..34].copy_from_slice(&self.score.to_le_bytes());
        b[34..36].copy_from_slice(&self.move16.to_le_bytes());
        b[36..38].copy_from_slice(&self.game_ply.to_le_bytes());
        b[38] = self.game_result as u8;
        b
    }
}

struct BitReader<'a> {
    data: &'a [u8; PACKED_SFEN_BYTES],
    cursor: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8; PACKED_SFEN_BYTES]) -> Self {
        BitReader { data, cursor: 0 }
    }

    fn get(&mut self) -> Result<u32, String> {
        if self.cursor >= 256 {
            return Err("packed sfenが256bitを超えて読まれた".to_string());
        }
        let b = (self.data[self.cursor / 8] >> (self.cursor % 8)) & 1;
        self.cursor += 1;
        Ok(u32::from(b))
    }

    fn get_n(&mut self, n: usize) -> Result<u32, String> {
        let mut v = 0u32;
        for i in 0..n {
            v |= self.get()? << i;
        }
        Ok(v)
    }
}

struct BitWriter {
    data: [u8; PACKED_SFEN_BYTES],
    cursor: usize,
}

impl BitWriter {
    fn new() -> Self {
        BitWriter {
            data: [0; PACKED_SFEN_BYTES],
            cursor: 0,
        }
    }

    fn put(&mut self, b: u32) -> Result<(), String> {
        if self.cursor >= 256 {
            return Err("packed sfenが256bitに収まらない".to_string());
        }
        if b != 0 {
            self.data[self.cursor / 8] |= 1 << (self.cursor % 8);
        }
        self.cursor += 1;
        Ok(())
    }

    fn put_n(&mut self, v: u32, n: usize) -> Result<(), String> {
        for i in 0..n {
            self.put((v >> i) & 1)?;
        }
        Ok(())
    }
}

/// 盤上駒のハフマン符号（code, bits）。LSB-firstで書き出す。
fn board_code(raw: PieceType) -> (u32, usize) {
    match raw {
        PieceType::PAWN => (0x01, 2),
        PieceType::LANCE => (0x03, 4),
        PieceType::KNIGHT => (0x0b, 4),
        PieceType::SILVER => (0x07, 4),
        PieceType::GOLD => (0x0f, 5),
        PieceType::BISHOP => (0x1f, 6),
        PieceType::ROOK => (0x3f, 6),
        _ => unreachable!("盤上符号にできない駒種: {raw:?}"),
    }
}

/// 盤上1升の駒を読む。空はNone。
fn read_board_piece(r: &mut BitReader) -> Result<Option<Piece>, String> {
    if r.get()? == 0 {
        return Ok(None);
    }
    let raw = if r.get()? == 0 {
        PieceType::PAWN
    } else if r.get()? == 0 {
        if r.get()? == 0 {
            PieceType::LANCE
        } else {
            PieceType::KNIGHT
        }
    } else if r.get()? == 0 {
        PieceType::SILVER
    } else if r.get()? == 0 {
        PieceType::GOLD
    } else if r.get()? == 0 {
        PieceType::BISHOP
    } else {
        PieceType::ROOK
    };
    // 金は成りフラグを持たない
    let pt = if raw == PieceType::GOLD {
        raw
    } else if r.get()? != 0 {
        raw.promote()
    } else {
        raw
    };
    let c = if r.get()? == 0 {
        Color::Black
    } else {
        Color::White
    };
    Ok(Some(Piece::new(c, pt)))
}

/// 手駒1枚を読む。符号は盤上符号の先頭1bitを削ったもの+成りフラグ+先後。
/// 金だけは成りフラグを持たない（金の駒箱は後手の成銀で表現するため）。
/// 成りフラグ=1は駒箱（駒落ち）を意味するのでエラー。
fn read_hand_piece(r: &mut BitReader) -> Result<(Color, PieceType), String> {
    let raw = if r.get()? == 0 {
        PieceType::PAWN
    } else if r.get()? == 0 {
        if r.get()? == 0 {
            PieceType::LANCE
        } else {
            PieceType::KNIGHT
        }
    } else if r.get()? == 0 {
        PieceType::SILVER
    } else if r.get()? == 0 {
        PieceType::GOLD
    } else if r.get()? == 0 {
        PieceType::BISHOP
    } else {
        PieceType::ROOK
    };
    if raw != PieceType::GOLD && r.get()? != 0 {
        return Err("駒箱符号（駒落ち局面）は対象外".to_string());
    }
    let c = if r.get()? == 0 {
        Color::Black
    } else {
        Color::White
    };
    Ok((c, raw))
}

/// packed sfen（32バイト）をSFEN文字列に復元する。
pub fn unpack_sfen(data: &[u8; PACKED_SFEN_BYTES], ply: u16) -> Result<String, String> {
    let mut r = BitReader::new(data);
    let turn = if r.get()? == 0 { "b" } else { "w" };

    let mut board = [Piece::EMPTY; 81];
    for c in [Color::Black, Color::White] {
        let k = r.get_n(7)? as usize;
        if k >= 81 {
            return Err(format!("玉の位置が盤外: {k}"));
        }
        if !board[k].is_empty() {
            return Err("両玉が同じ升にいる".to_string());
        }
        board[k] = Piece::new(c, PieceType::KING);
    }

    for (sq, cell) in board.iter_mut().enumerate() {
        if cell.piece_type() == PieceType::KING {
            continue;
        }
        if let Some(pc) = read_board_piece(&mut r)? {
            *cell = pc;
        }
        let _ = sq;
    }

    // 手駒はちょうど256bitまで詰まっている
    let mut hands = [[0u32; PieceType::NB]; 2];
    while r.cursor != 256 {
        let (c, pt) = read_hand_piece(&mut r)?;
        hands[c.index()][pt.index()] += 1;
    }

    // SFEN組み立て。段は一段目から、筋は9筋から
    let mut out = String::with_capacity(96);
    for rank in 0..9 {
        let mut empties = 0;
        for file in (0..9).rev() {
            let pc = board[file * 9 + rank];
            if pc.is_empty() {
                empties += 1;
                continue;
            }
            if empties > 0 {
                out.push_str(&empties.to_string());
                empties = 0;
            }
            let s = pc.piece_type().to_sfen().expect("盤上駒のSFEN文字");
            if pc.color() == Color::Black {
                out.push_str(s);
            } else {
                out.push_str(&s.to_lowercase());
            }
        }
        if empties > 0 {
            out.push_str(&empties.to_string());
        }
        if rank < 8 {
            out.push('/');
        }
    }

    out.push(' ');
    out.push_str(turn);
    out.push(' ');
    // 手駒は慣例順（飛角金銀桂香歩、先手が先）
    const HAND_ORDER: [PieceType; 7] = [
        PieceType::ROOK,
        PieceType::BISHOP,
        PieceType::GOLD,
        PieceType::SILVER,
        PieceType::KNIGHT,
        PieceType::LANCE,
        PieceType::PAWN,
    ];
    let mut any = false;
    for c in [Color::Black, Color::White] {
        for pt in HAND_ORDER {
            let n = hands[c.index()][pt.index()];
            if n == 0 {
                continue;
            }
            any = true;
            if n > 1 {
                out.push_str(&n.to_string());
            }
            let s = pt.to_sfen().expect("手駒のSFEN文字");
            if c == Color::Black {
                out.push_str(s);
            } else {
                out.push_str(&s.to_lowercase());
            }
        }
    }
    if !any {
        out.push('-');
    }
    out.push(' ');
    out.push_str(&ply.max(1).to_string());
    Ok(out)
}

/// packed sfen（32バイト）からPositionを構築する。
pub fn unpack(data: &[u8; PACKED_SFEN_BYTES], ply: u16) -> Result<Position, String> {
    let sfen = unpack_sfen(data, ply)?;
    Position::from_sfen(&sfen).map_err(|e| format!("復元SFENが不正: {sfen} ({e:?})"))
}

/// Positionをpacked sfen（32バイト）に圧縮する。検証と将来の
/// gensfen用。40駒全数の局面のみ対応（それ以外はエラー）。
pub fn pack(pos: &Position) -> Result<[u8; PACKED_SFEN_BYTES], String> {
    let mut w = BitWriter::new();
    w.put(u32::from(pos.side_to_move() == Color::White))?;
    for c in [Color::Black, Color::White] {
        w.put_n(pos.king(c).index() as u32, 7)?;
    }

    for sq_i in 0..81u8 {
        let pc = pos.piece_on(Square::from_index(sq_i));
        if pc.is_empty() {
            w.put(0)?;
            continue;
        }
        let pt = pc.piece_type();
        if pt == PieceType::KING {
            continue;
        }
        let raw = pt.unpromote();
        let (code, bits) = board_code(raw);
        w.put_n(code, bits)?;
        if raw != PieceType::GOLD {
            w.put(u32::from(pt.is_promoted()))?;
        }
        w.put(u32::from(pc.color() == Color::White))?;
    }

    // 手駒（やねうら王のApery順: 歩香桂銀金角飛、先手が先）
    const APERY_ORDER: [PieceType; 7] = [
        PieceType::PAWN,
        PieceType::LANCE,
        PieceType::KNIGHT,
        PieceType::SILVER,
        PieceType::GOLD,
        PieceType::BISHOP,
        PieceType::ROOK,
    ];
    for c in [Color::Black, Color::White] {
        let hand = pos.hand(c);
        for pt in APERY_ORDER {
            let (code, bits) = board_code(pt);
            for _ in 0..hand.count(pt) {
                // 盤上符号の先頭1bitを削る。金は成りフラグを持たない
                w.put_n(code >> 1, bits - 1)?;
                if pt != PieceType::GOLD {
                    w.put(0)?; // 成りフラグ（手駒は常に0）
                }
                w.put(u32::from(c == Color::White))?;
            }
        }
    }

    if w.cursor != 256 {
        return Err(format!(
            "駒が40枚に満たない局面は対象外（{}bitで終了）",
            w.cursor
        ));
    }
    Ok(w.data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movegen::generate_legal;
    use crate::moves::MoveList;
    use crate::position::SFEN_STARTPOS;

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
    }

    /// 乱数プレイアウト局面のpack→unpackでSFENが一致する。
    #[test]
    fn roundtrip_random_positions() {
        for seed in 1..=16u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
            for _ in 0..100 {
                let packed = pack(&pos).unwrap();
                let restored = unpack(&packed, pos.game_ply()).unwrap();
                assert_eq!(pos.to_sfen(), restored.to_sfen());
                let mut list = MoveList::default();
                generate_legal(&pos, true, &mut list);
                if list.is_empty() {
                    break;
                }
                let m = list.as_slice()[(rng.next() % list.len() as u64) as usize];
                pos.do_move(m);
            }
        }
    }

    /// 平手初期局面はちょうど256bitに収まり、復元も一致する。
    #[test]
    fn roundtrip_startpos() {
        let pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
        let packed = pack(&pos).unwrap();
        let restored = unpack_sfen(&packed, 1).unwrap();
        assert_eq!(restored, SFEN_STARTPOS);
    }

    /// PSVレコードのバイト列roundtrip。
    #[test]
    fn psv_record_roundtrip() {
        let pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
        let rec = PackedSfenValue {
            sfen: pack(&pos).unwrap(),
            score: -1234,
            move16: 0xBEEF,
            game_ply: 42,
            game_result: -1,
        };
        let restored = PackedSfenValue::from_bytes(&rec.to_bytes());
        assert_eq!(restored.score, -1234);
        assert_eq!(restored.move16, 0xBEEF);
        assert_eq!(restored.game_ply, 42);
        assert_eq!(restored.game_result, -1);
        assert_eq!(restored.sfen, rec.sfen);
    }

    /// 壊れた入力（玉位置が盤外）はエラーになる。
    #[test]
    fn rejects_invalid_king_square() {
        let mut data = [0u8; PACKED_SFEN_BYTES];
        // 手番0 + 玉位置127（7bit全部1）
        data[0] = 0b1111_1110;
        assert!(unpack_sfen(&data, 1).is_err());
    }
}
