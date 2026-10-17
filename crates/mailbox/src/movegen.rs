//! 合法手の生成。盤上の手は駒番号の一覧から引き、合法性は利きの配列で決める。
//!
//! - 玉の手: 行き先に相手の利きが無く、かつ玉の升に届く相手の飛び利きの延長でない
//! - 玉以外の盤上の手: ピンされた駒はピンの線の上だけを動く。王手中は王手駒を取るか合駒の升へ
//! - 駒打ち: 王手中は合駒の升だけ。王手になる歩打ちは、打って相手に合法手が無ければ打ち歩詰め
//!
//! 生成の順はcoreと違う。照合は集合で行う。

use crate::board::*;
use crate::moves::Move;
use crate::position::Position;

/// 合法手は最大593手。
pub const MAX_MOVES: usize = 600;

pub struct MoveList {
    moves: [Move; MAX_MOVES],
    len: usize,
}

impl Default for MoveList {
    fn default() -> Self {
        MoveList {
            moves: [Move::NONE; MAX_MOVES],
            len: 0,
        }
    }
}

impl MoveList {
    #[inline]
    pub fn push(&mut self, m: Move) {
        self.moves[self.len] = m;
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
    pub fn as_slice(&self) -> &[Move] {
        &self.moves[..self.len]
    }

    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
    }
}

/// 升の集合（配列盤の添字143升を2語で持つ）。
#[derive(Copy, Clone, Default)]
struct SqSet([u128; 2]);

impl SqSet {
    #[inline]
    fn set(&mut self, i: u8) {
        self.0[(i >> 7) as usize] |= 1u128 << (i & 127);
    }

    #[inline]
    fn has(&self, i: u8) -> bool {
        self.0[(i >> 7) as usize] & (1u128 << (i & 127)) != 0
    }
}

/// 生成の前に1回だけ求める、王手とピンの情報。
struct Ctx {
    us: Color,
    /// 王手中なら、玉以外の駒が行ってよい升（王手駒の升と合駒の升）。
    evasion: Option<SqSet>,
    /// 合駒の升（駒打ちの行き先）。
    between: SqSet,
    /// ピンされた駒の升と、ピンの線の方向（玉から駒への向き）。
    pins: [(u8, u8); 8],
    n_pins: usize,
}

impl Ctx {
    #[inline]
    fn pin_dir(&self, s: u8) -> Option<usize> {
        for k in 0..self.n_pins {
            if self.pins[k].0 == s {
                return Some(self.pins[k].1 as usize);
            }
        }
        None
    }
}

impl Position {
    /// 玉の升kから8方向を歩き、ピンされた自駒を集める。
    /// 方向dで最初に当たった自駒の升に、相手の飛び利きが方向opp(d)（玉へ向かう向き）で
    /// 届いていればピンである。
    fn pins(&self, us: Color, ctx: &mut Ctx) {
        let k = self.king[us.index()];
        let them = us.flip().index();
        for (d, &delta) in DELTA.iter().enumerate() {
            let delta = delta as i32;
            let mut t = (k as i32 + delta) as usize;
            loop {
                let v = self.board[t];
                if v == EMPTY {
                    t = (t as i32 + delta) as usize;
                    continue;
                }
                if v != WALL && piece_color(v) == us && self.longdir[them][t] & (1 << opp(d)) != 0 {
                    ctx.pins[ctx.n_pins] = (t as u8, d as u8);
                    ctx.n_pins += 1;
                }
                break;
            }
        }
    }

    /// 1つの王手駒を探す。王手が1つのときに呼ぶ。合駒の升も返す。
    fn checker(&self, us: Color) -> (u8, SqSet) {
        let k = self.king[us.index()];
        let them = us.flip();
        let mut between = SqSet::default();
        // 飛び利きの王手。玉へ向かう方向dの飛び利きをたどって駒を探す
        let dirs = self.longdir[them.index()][k as usize];
        if dirs != 0 {
            let d = dirs.trailing_zeros() as usize;
            let back = DELTA[opp(d)] as i32;
            let mut t = (k as i32 + back) as u8;
            while self.board[t as usize] == EMPTY {
                between.set(t);
                t = (t as i32 + back) as u8;
            }
            return (t, between);
        }
        // 短い利きの王手。玉の近傍と桂の升を見る
        for o in [-1i8, 1, 11, -11, 10, -10, -12, 12, 9, -13, -9, 13] {
            let x = (k as i32 - o as i32) as usize;
            let v = self.board[x];
            if v == EMPTY || v == WALL || piece_color(v) != them {
                continue;
            }
            let m = &MOVES[v as usize];
            if m.short[..m.n_short as usize].contains(&o) {
                return (x as u8, between);
            }
        }
        unreachable!("王手駒が見つからない: {}", self.to_sfen());
    }

    /// 合法手を全部生成する（不成も、行き所のある限りすべて出す）。
    pub fn generate_legal(&mut self, list: &mut MoveList) {
        let us = self.side;
        let ui = us.index();
        let them = us.flip().index();
        let k = self.king[ui];
        let n_checks = self.count[them][k as usize];
        let mut ctx = Ctx {
            us,
            evasion: None,
            between: SqSet::default(),
            pins: [(0, 0); 8],
            n_pins: 0,
        };

        self.gen_king(&ctx, list);
        if n_checks >= 2 {
            return;
        }
        self.pins(us, &mut ctx);
        if n_checks == 1 {
            let (csq, between) = self.checker(us);
            let mut ev = between;
            ev.set(csq);
            ctx.evasion = Some(ev);
            ctx.between = between;
        }
        self.gen_board(&ctx, list);
        self.gen_drops(&ctx, n_checks == 1, list);
    }

    fn gen_king(&self, ctx: &Ctx, list: &mut MoveList) {
        let us = ctx.us;
        let ui = us.index();
        let them = us.flip().index();
        let k = self.king[ui];
        let xray = self.longdir[them][k as usize];
        for (d, &delta) in DELTA.iter().enumerate() {
            let t = (k as i32 + delta as i32) as usize;
            if !CAN_MOVE_TO[ui][self.board[t] as usize] {
                continue;
            }
            if self.count[them][t] != 0 || xray & (1 << d) != 0 {
                continue;
            }
            list.push(Move::normal(k, t as u8, false));
        }
    }

    fn gen_board(&self, ctx: &Ctx, list: &mut MoveList) {
        let us = ctx.us;
        let ui = us.index();
        let mut ids = self.on_board[ui];
        while ids != 0 {
            let id = ids.trailing_zeros() as usize;
            ids &= ids - 1;
            let s = self.sq_of[id];
            let p = self.board[s as usize];
            let pt = piece_type(p);
            if pt == KING {
                continue;
            }
            let pin = ctx.pin_dir(s);
            let m = &MOVES[p as usize];
            for k in 0..m.n_short as usize {
                let o = m.short[k];
                if let Some(pd) = pin
                    && o != DELTA[pd]
                    && o != -DELTA[pd]
                {
                    continue;
                }
                let t = (s as i32 + o as i32) as u8;
                if !CAN_MOVE_TO[ui][self.board[t as usize] as usize] {
                    continue;
                }
                if let Some(ev) = &ctx.evasion
                    && !ev.has(t)
                {
                    continue;
                }
                push_moves(us, pt, s, t, list);
            }
            let mut dirs = m.long;
            while dirs != 0 {
                let d = dirs.trailing_zeros() as usize;
                dirs &= dirs - 1;
                if let Some(pd) = pin
                    && d != pd
                    && d != opp(pd)
                {
                    continue;
                }
                let delta = DELTA[d] as i32;
                let mut t = (s as i32 + delta) as u8;
                loop {
                    let v = self.board[t as usize];
                    if !CAN_MOVE_TO[ui][v as usize] {
                        break;
                    }
                    if ctx.evasion.as_ref().is_none_or(|ev| ev.has(t)) {
                        push_moves(us, pt, s, t, list);
                    }
                    if v != EMPTY {
                        break;
                    }
                    t = (t as i32 + delta) as u8;
                }
            }
        }
    }

    fn gen_drops(&mut self, ctx: &Ctx, in_check: bool, list: &mut MoveList) {
        let us = ctx.us;
        let ui = us.index();
        let mut kinds = [0u8; 7];
        let mut nk = 0;
        for pt in HAND_KINDS {
            if self.hand_ids[ui][hand_index(pt)] != 0 {
                kinds[nk] = pt;
                nk += 1;
            }
        }
        if nk == 0 {
            return;
        }
        // 王手になる歩打ちの升（相手の玉の1升前）。打ち歩詰めならそこへ歩を打てない
        let ek = self.king[us.flip().index()];
        let pawn_check_sq = match us {
            Color::Black => ek + 1,
            Color::White => ek - 1,
        };
        let mut pawn_mate_sq = 0u8;
        if self.hand_ids[ui][hand_index(PAWN)] != 0
            && self.board[pawn_check_sq as usize] == EMPTY
            && self.pawn_files[ui] & (1 << file_of(pawn_check_sq)) == 0
            && (!in_check || ctx.between.has(pawn_check_sq))
        {
            let m = Move::drop(PAWN, pawn_check_sq);
            self.do_move(m);
            let mut replies = MoveList::default();
            self.generate_legal(&mut replies);
            self.undo_move(m);
            if replies.is_empty() {
                pawn_mate_sq = pawn_check_sq;
            }
        }
        let emit = |t: u8, list: &mut MoveList| {
            let rr = rel_rank(us, t);
            for &pt in &kinds[..nk] {
                match pt {
                    PAWN => {
                        if rr == 0
                            || self.pawn_files[ui] & (1 << file_of(t)) != 0
                            || t == pawn_mate_sq
                        {
                            continue;
                        }
                    }
                    LANCE if rr == 0 => continue,
                    KNIGHT if rr <= 1 => continue,
                    _ => {}
                }
                list.push(Move::drop(pt, t));
            }
        };
        if in_check {
            for &t in IDX81.iter() {
                if ctx.between.has(t) {
                    emit(t, list);
                }
            }
        } else {
            for &t in IDX81.iter() {
                if self.board[t as usize] == EMPTY {
                    emit(t, list);
                }
            }
        }
    }
}

/// 盤上の駒の手を、成り・不成を分けて積む。行き所のない不成は出さない。
#[inline]
fn push_moves(us: Color, pt: u8, from: u8, to: u8, list: &mut MoveList) {
    if can_promote(pt) && (rel_rank(us, from) <= 2 || rel_rank(us, to) <= 2) {
        list.push(Move::normal(from, to, true));
        let rr = rel_rank(us, to);
        let dead = match pt {
            PAWN | LANCE => rr == 0,
            KNIGHT => rr <= 1,
            _ => false,
        };
        if !dead {
            list.push(Move::normal(from, to, false));
        }
    } else {
        list.push(Move::normal(from, to, false));
    }
}

/// perft。深さ1は合法手の数を数えるだけにする。
pub fn perft(pos: &mut Position, depth: u32) -> u64 {
    let mut list = MoveList::default();
    pos.generate_legal(&mut list);
    if depth <= 1 {
        return list.len() as u64;
    }
    let mut n = 0;
    for &m in list.as_slice() {
        pos.do_move(m);
        n += perft(pos, depth - 1);
        pos.undo_move(m);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startpos_perft_shallow() {
        let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
        assert_eq!(perft(&mut pos, 1), 30);
        assert_eq!(perft(&mut pos, 2), 900);
        assert_eq!(perft(&mut pos, 3), 25_470);
    }
}
