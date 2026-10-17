//! 合法手の生成。合法性は利きの配列で決め、生成した手をあとで濾さない。
//!
//! - 玉の手: 行き先に相手の利きが無く、かつ玉の升に届く相手の飛び利きの延長でない
//! - 玉以外の盤上の手: ピンされた駒はピンの線の上だけを動く。王手中は王手駒を取るか合駒の升へ
//! - 駒打ち: 王手中は合駒の升だけ。王手になる歩打ちは、打ってから詰みかを専用の判定で見る
//!
//! 生成の順はcoreに揃える（ADR-0241の段階6で、固定深さのノード数を比べるため）。
//! coreの `generate(GenType, all)` の擬似合法手を `is_legal` で濾した列と、
//! 同じ手が同じ順に並ぶ。`generate_legal` はcoreの同名の関数と同じ列を返す。
//!
//! 順を揃えるため、1つの駒の行き先は81升の集合（u128）へ集めてから小さい升の順に回す。
//! 動かす駒も、駒のいる升の集合を小さい升の順に回す。

use std::mem::MaybeUninit;

use crate::board::*;
use crate::moves::Move;
use crate::position::Position;

/// 合法手は最大593手。
pub const MAX_MOVES: usize = 600;

pub struct MoveList {
    moves: [MaybeUninit<Move>; MAX_MOVES],
    len: usize,
}

impl Default for MoveList {
    #[inline]
    fn default() -> Self {
        MoveList {
            moves: [const { MaybeUninit::uninit() }; MAX_MOVES],
            len: 0,
        }
    }
}

impl MoveList {
    #[inline]
    pub fn push(&mut self, m: Move) {
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
    pub fn as_slice(&self) -> &[Move] {
        // SAFETY: 先頭からlen個はpushで書き込み済みで、MoveはCopyなので初期化済みとして読める
        unsafe { std::slice::from_raw_parts(self.moves.as_ptr().cast::<Move>(), self.len) }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
    }
}

/// 生成の分類。coreの `GenType` と同じ意味で、どれも合法手だけを返す。
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum GenType {
    /// 駒取りと、空き升への歩の成り
    Captures,
    /// 駒を取らない手（歩の成りを除く）と駒打ち
    Quiets,
    /// 王手の回避（王手中だけ）
    Evasions,
    /// 王手でないときの全部
    NonEvasions,
}

/// 生成の前に1回だけ求める、王手とピンの情報。
struct Ctx {
    us: Color,
    /// 玉以外の駒が行ってよい升。王手中は王手駒の升と合駒の升、それ以外は全部。
    evasion: u128,
    /// ピンされた駒の升と、その駒が動ける線（玉からピンの方向へ盤の端まで）。
    pins: [(u8, u128); 8],
    n_pins: usize,
}

impl Ctx {
    #[inline]
    fn pin_mask(&self, s: u8) -> u128 {
        for k in 0..self.n_pins {
            if self.pins[k].0 == s {
                return self.pins[k].1;
            }
        }
        ALL81
    }
}

/// 王手駒を探す短い利きの差分（玉の8近傍と、両者の桂の位置）。
const CHECK_OFFSETS: [i8; 12] = [-1, 1, 11, -11, 10, -10, -12, 12, 9, -13, -9, 13];

impl Position {
    /// 玉の升kから方向dへ盤の端まで歩いた升の集合（kを含まない）。
    fn line_from(&self, k: u8, d: usize) -> u128 {
        let mut m = 0u128;
        let delta = DELTA[d] as i32;
        let mut t = (k as i32 + delta) as u8;
        while self.board[t as usize] != WALL {
            m |= bit81(t);
            t = (t as i32 + delta) as u8;
        }
        m
    }

    /// 手番cの玉から方向dへ歩いて最初の駒の升（無ければNone）。
    #[inline]
    fn first_piece(&self, k: u8, d: usize) -> Option<u8> {
        let delta = DELTA[d] as i32;
        let mut t = (k as i32 + delta) as usize;
        loop {
            match self.board[t] {
                EMPTY => t = (t as i32 + delta) as usize,
                WALL => return None,
                _ => return Some(t as u8),
            }
        }
    }

    /// 色usの駒xが、自玉に対してピンされているか。
    #[inline]
    fn is_pinned(&self, us: Color, x: u8) -> bool {
        let k = self.king[us.index()];
        let Some(d) = dir_between(k, x) else {
            return false;
        };
        self.first_piece(k, d) == Some(x)
            && self.longdir[us.flip().index()][x as usize] & (1 << opp(d)) != 0
    }

    /// 玉から8方向を歩き、ピンされた自駒を集める。
    /// 方向dで最初に当たった自駒の升に、相手の飛び利きが方向opp(d)（玉へ向かう向き）で
    /// 届いていればピンである。
    fn collect_pins(&self, ctx: &mut Ctx) {
        let us = ctx.us;
        let k = self.king[us.index()];
        let them = us.flip().index();
        for d in 0..8 {
            if let Some(t) = self.first_piece(k, d) {
                let v = self.board[t as usize];
                if piece_color(v) == us && self.longdir[them][t as usize] & (1 << opp(d)) != 0 {
                    ctx.pins[ctx.n_pins] = (t, self.line_from(k, d));
                    ctx.n_pins += 1;
                }
            }
        }
    }

    /// 1つの王手駒の升と、合駒の升の集合。王手が1つのときに呼ぶ。
    fn checker(&self, us: Color) -> (u8, u128) {
        let k = self.king[us.index()];
        let them = us.flip();
        let mut between = 0u128;
        // 飛び利きの王手。玉へ向かう方向dの飛び利きを逆にたどって駒を探す
        let dirs = self.longdir[them.index()][k as usize];
        if dirs != 0 {
            let d = dirs.trailing_zeros() as usize;
            let back = DELTA[opp(d)] as i32;
            let mut t = (k as i32 + back) as u8;
            while self.board[t as usize] == EMPTY {
                between |= bit81(t);
                t = (t as i32 + back) as u8;
            }
            return (t, between);
        }
        // 短い利きの王手。玉の近傍と桂の升を見る
        for o in CHECK_OFFSETS {
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

    /// 升sの駒pが動ける升の集合（空き升と相手の駒）。
    #[inline]
    fn targets_of(&self, s: u8, p: u8, ui: usize) -> u128 {
        let m = &MOVES[p as usize];
        let mut out = 0u128;
        for k in 0..m.n_short as usize {
            let t = (s as i32 + m.short[k] as i32) as u8;
            if CAN_MOVE_TO[ui][self.board[t as usize] as usize] {
                out |= bit81(t);
            }
        }
        let mut dirs = m.long;
        while dirs != 0 {
            let d = dirs.trailing_zeros() as usize;
            dirs &= dirs - 1;
            let delta = DELTA[d] as i32;
            let mut t = (s as i32 + delta) as u8;
            loop {
                let v = self.board[t as usize];
                if !CAN_MOVE_TO[ui][v as usize] {
                    break;
                }
                out |= bit81(t);
                if v != EMPTY {
                    break;
                }
                t = (t as i32 + delta) as u8;
            }
        }
        out
    }

    /// 合法手を全部生成する。順はcoreの `generate_legal(pos, all, list)` と同じ。
    pub fn generate_legal(&mut self, all: bool, list: &mut MoveList) {
        if self.in_check() {
            self.generate(GenType::Evasions, all, list);
        } else {
            self.generate(GenType::NonEvasions, all, list);
        }
    }

    /// 分類ごとの合法手を生成する。Evasionsは王手中だけ、ほかは王手でないときだけ呼ぶ。
    pub fn generate(&mut self, gt: GenType, all: bool, list: &mut MoveList) {
        let us = self.side;
        let ui = us.index();
        let them = us.flip().index();
        let k = self.king[ui];
        let n_checks = self.count[them][k as usize];
        debug_assert_eq!(gt == GenType::Evasions, n_checks != 0);
        let mut ctx = Ctx {
            us,
            evasion: ALL81,
            pins: [(0, 0); 8],
            n_pins: 0,
        };
        let own = self.occ[ui];
        let enemy = self.occ[them];
        let empty = ALL81 & !(own | enemy);
        match gt {
            GenType::Evasions => {
                self.gen_king(&ctx, ALL81 & !own, list);
                if n_checks >= 2 {
                    return;
                }
                let (csq, between) = self.checker(us);
                ctx.evasion = between | bit81(csq);
                self.collect_pins(&mut ctx);
                self.gen_board(&ctx, ALL81 & !own, all, false, list);
                self.gen_drops(&ctx, between, list);
            }
            GenType::NonEvasions => {
                self.collect_pins(&mut ctx);
                self.gen_board(&ctx, ALL81 & !own, all, false, list);
                self.gen_king(&ctx, ALL81 & !own, list);
                self.gen_drops(&ctx, empty, list);
            }
            GenType::Captures => {
                self.collect_pins(&mut ctx);
                self.gen_board(&ctx, enemy, all, false, list);
                self.gen_king(&ctx, enemy, list);
                // 空き升への歩の成り
                let fwd = pawn_delta(us);
                for_each81(own, |s| {
                    if self.board[s as usize] != make_piece(us, PAWN) {
                        return;
                    }
                    let t = (s as i32 + fwd) as u8;
                    if self.board[t as usize] == EMPTY
                        && rel_rank(us, t) <= 2
                        && ctx.pin_mask(s) & bit81(t) != 0
                    {
                        list.push(Move::normal(s, t, true));
                    }
                });
            }
            GenType::Quiets => {
                self.collect_pins(&mut ctx);
                self.gen_board(&ctx, empty, all, true, list);
                // 歩の静かな手。成りはCaptures側で、不成だけをここで積む
                let fwd = pawn_delta(us);
                for_each81(own, |s| {
                    if self.board[s as usize] != make_piece(us, PAWN) {
                        return;
                    }
                    let t = (s as i32 + fwd) as u8;
                    if self.board[t as usize] != EMPTY || ctx.pin_mask(s) & bit81(t) == 0 {
                        return;
                    }
                    let rr = rel_rank(us, t);
                    if rr > 2 || (all && rr >= 1) {
                        list.push(Move::normal(s, t, false));
                    }
                });
                self.gen_king(&ctx, empty, list);
                self.gen_drops(&ctx, empty, list);
            }
        }
    }

    fn gen_king(&self, ctx: &Ctx, target: u128, list: &mut MoveList) {
        let ui = ctx.us.index();
        let them = ctx.us.flip().index();
        let k = self.king[ui];
        let xray = self.longdir[them][k as usize];
        let mut safe = 0u128;
        for (d, &delta) in DELTA.iter().enumerate() {
            let t = (k as i32 + delta as i32) as u8;
            if CAN_MOVE_TO[ui][self.board[t as usize] as usize]
                && self.count[them][t as usize] == 0
                && xray & (1 << d) == 0
            {
                safe |= bit81(t);
            }
        }
        for_each81(safe & target, |t| list.push(Move::normal(k, t, false)));
    }

    /// 玉以外の盤上の駒の手。skip_pawnsなら歩を飛ばす（Quietsは歩を別に積む）。
    fn gen_board(&self, ctx: &Ctx, target: u128, all: bool, skip_pawns: bool, list: &mut MoveList) {
        let us = ctx.us;
        let ui = us.index();
        let movers = self.occ[ui] & !bit81(self.king[ui]);
        let target = target & ctx.evasion;
        for_each81(movers, |s| {
            let p = self.board[s as usize];
            let pt = piece_type(p);
            if skip_pawns && pt == PAWN {
                return;
            }
            let to = self.targets_of(s, p, ui) & target & ctx.pin_mask(s);
            for_each81(to, |t| push_variants(us, pt, s, t, all, list));
        });
    }

    fn gen_drops(&mut self, ctx: &Ctx, target: u128, list: &mut MoveList) {
        let us = ctx.us;
        let ui = us.index();
        if self.hand_ids[ui].iter().all(|&h| h == 0) {
            return;
        }
        let r1 = rank_mask(if us == Color::Black { 0 } else { 8 });
        let r2 = rank_mask(if us == Color::Black { 1 } else { 7 });
        for pt in DROP_KINDS {
            if self.hand_ids[ui][hand_index(pt)] == 0 {
                continue;
            }
            let mut mask = match pt {
                PAWN => {
                    let mut nifu = 0u128;
                    let mut files = self.pawn_files[ui];
                    while files != 0 {
                        nifu |= file_mask(files.trailing_zeros() as u8);
                        files &= files - 1;
                    }
                    target & !r1 & !nifu
                }
                LANCE => target & !r1,
                KNIGHT => target & !(r1 | r2),
                _ => target,
            };
            if pt == PAWN {
                // 王手になる歩打ちの升（相手の玉の1升前）。打ち歩詰めなら除く
                let ek = self.king[us.flip().index()];
                let sq = (ek as i32 - pawn_delta(us)) as u8;
                if self.board[sq as usize] != WALL
                    && mask & bit81(sq) != 0
                    && self.pawn_drop_mates(sq)
                {
                    mask &= !bit81(sq);
                }
            }
            for_each81(mask, |t| list.push(Move::drop(pt, t)));
        }
    }

    /// 升sへの歩打ちが打ち歩詰めか。打って進め、相手に王手を外す手があるかを見て戻す。
    fn pawn_drop_mates(&mut self, s: u8) -> bool {
        let m = Move::drop(PAWN, s);
        self.do_move(m);
        let escapes = self.has_reply_to_pawn_check(s);
        self.undo_move(m);
        !escapes
    }

    /// 升sの歩で王手された側（手番側）に、合法手があるか。
    /// 王手は歩の1つだけなので、合駒は無い。玉の移動と、歩を取る手だけを見る。
    fn has_reply_to_pawn_check(&self, s: u8) -> bool {
        let us = self.side;
        let ui = us.index();
        let att = us.flip().index();
        let k = self.king[ui];
        // 玉の移動（歩を取る手を含む）。王手は短い利きだけなので、背後の利きは無い
        for &delta in DELTA.iter() {
            let t = (k as i32 + delta as i32) as usize;
            if CAN_MOVE_TO[ui][self.board[t] as usize] && self.count[att][t] == 0 {
                return true;
            }
        }
        // 玉以外で歩を取る手。ピンされた駒は歩の升へ動けない（歩の升は玉の隣で、
        // ピンの線の上なら歩が玉との間にいることになり、ピンにならない）
        for o in CHECK_OFFSETS {
            let x = (s as i32 - o as i32) as u8;
            let v = self.board[x as usize];
            if v == EMPTY || v == WALL || piece_color(v) != us || piece_type(v) == KING {
                continue;
            }
            let m = &MOVES[v as usize];
            if m.short[..m.n_short as usize].contains(&o) && !self.is_pinned(us, x) {
                return true;
            }
        }
        let mut dirs = self.longdir[ui][s as usize];
        while dirs != 0 {
            let d = dirs.trailing_zeros() as usize;
            dirs &= dirs - 1;
            let back = DELTA[opp(d)] as i32;
            let mut x = (s as i32 + back) as u8;
            while self.board[x as usize] == EMPTY {
                x = (x as i32 + back) as u8;
            }
            if !self.is_pinned(us, x) {
                return true;
            }
        }
        false
    }
}

/// 歩が進む向きの差分。
#[inline]
const fn pawn_delta(us: Color) -> i32 {
    match us {
        Color::Black => -1,
        Color::White => 1,
    }
}

/// 盤上の駒の手を、成り・不成を分けて積む。規約はcoreの `push_variants` と同じ。
#[inline]
fn push_variants(us: Color, pt: u8, from: u8, to: u8, all: bool, list: &mut MoveList) {
    let rel = rel_rank(us, to);
    if can_promote(pt) && (rel_rank(us, from) <= 2 || rel <= 2) {
        list.push(Move::normal(from, to, true));
        let non_promote = match pt {
            PAWN => all && rel >= 1,
            LANCE => {
                if all {
                    rel >= 1
                } else {
                    rel >= 2
                }
            }
            KNIGHT => rel >= 2,
            SILVER => true,
            _ => all,
        };
        if non_promote {
            list.push(Move::normal(from, to, false));
        }
    } else {
        let ok = match pt {
            PAWN | LANCE => rel >= 1,
            KNIGHT => rel >= 2,
            _ => true,
        };
        if ok {
            list.push(Move::normal(from, to, false));
        }
    }
}

/// perft。深さ1は合法手の数を数えるだけにする。
pub fn perft(pos: &mut Position, depth: u32) -> u64 {
    let mut list = MoveList::default();
    pos.generate_legal(true, &mut list);
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
