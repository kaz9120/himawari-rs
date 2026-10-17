//! 段階5の盤面操作。合法性・擬似合法性・王手の判定・SEE・1手詰め（ADR-0241の第6回）。
//!
//! どれもcoreの同名の関数と同じ値を返す（照合のテストで確かめる）。
//! 利きの配列を引いて決め、Bitboardの利きの計算は使わない。
//!
//! - `is_legal`: 擬似合法手の合法性。玉の手は利き数と飛び利きの延長、ほかはピンで決める
//! - `pseudo_legal`: 置換表から来た手が、この局面の擬似合法手か
//! - `gives_check`: 直接王手は駒の利きの形と線の空き、開き王手は自分の飛び利きの方向ビットで決める
//! - `see_ge`: coreと同じswapの手順。攻め駒は升から8方向と近傍を見て集める
//! - `mate_1ply`: coreと同じ候補を同じ順に調べ、最初に詰みと判定した手を返す。
//!   判定は手を進めて、王手を外す手があるかを利きの配列で見る。coreの判定は不完全
//!   （持ち駒があれば合駒できるとみなす）なので、その取り決めも同じにする

use crate::board::*;
use crate::movegen::pawn_delta;
use crate::moves::Move;
use crate::position::{CHECK_OFFSETS, Position};
use crate::zobrist::PIECE_VALUE;

/// 81升の集合を小さい升の順に、升の添字で返す。
#[inline]
fn iter81(mut m: u128) -> impl Iterator<Item = u8> {
    std::iter::from_fn(move || {
        if m == 0 {
            return None;
        }
        let i = m.trailing_zeros() as usize;
        m &= m - 1;
        Some(IDX81[i])
    })
}

/// SEEで攻め駒を選ぶ順（安い順）。coreの `SEE_ORDER` と同じで、金と成金4種は同じ組。
#[inline]
const fn see_rank(pt: u8) -> u8 {
    match pt {
        PAWN => 0,
        LANCE => 1,
        KNIGHT => 2,
        SILVER => 3,
        GOLD | 2..=5 => 4,
        BISHOP => 5,
        HORSE => 6,
        ROOK => 7,
        DRAGON => 8,
        _ => 9,
    }
}

const SEE_VALUE: [i32; 10] = [90, 315, 405, 495, 540, 855, 945, 990, 1395, 15000];

/// 行き所のない駒になるので不成では入れない升か。
#[inline]
fn dead_square(us: Color, pt: u8, to: u8) -> bool {
    let rr = rel_rank(us, to);
    match pt {
        PAWN | LANCE => rr == 0,
        KNIGHT => rr <= 1,
        _ => false,
    }
}

impl Position {
    /// 色cの駒のうち、升sqへ利いている駒の升の集合（81升）。
    /// removedの升の駒は無いものとし、飛び利きもそこを通す（SEEの取り合いで使う）。
    pub fn attackers_with(&self, c: Color, sq: u8, removed: u128) -> u128 {
        let mut set = 0u128;
        for o in CHECK_OFFSETS {
            let x = (sq as i32 - o as i32) as u8;
            let v = self.board[x as usize];
            if v == EMPTY || v == WALL || piece_color(v) != c || removed & bit81(x) != 0 {
                continue;
            }
            let m = &MOVES[v as usize];
            if m.short[..m.n_short as usize].contains(&o) {
                set |= bit81(x);
            }
        }
        for (d, &delta) in DELTA.iter().enumerate() {
            let mut t = (sq as i32 + delta as i32) as u8;
            loop {
                let v = self.board[t as usize];
                if v == WALL {
                    break;
                }
                if v == EMPTY || removed & bit81(t) != 0 {
                    t = (t as i32 + delta as i32) as u8;
                    continue;
                }
                if piece_color(v) == c && MOVES[v as usize].long & (1 << opp(d)) != 0 {
                    set |= bit81(t);
                }
                break;
            }
        }
        set
    }

    /// 升aと升bの間の升の集合（同じ線に無ければ空）。
    pub fn between81(&self, a: u8, b: u8) -> u128 {
        let Some(d) = dir_between(a, b) else {
            return 0;
        };
        let delta = DELTA[d] as i32;
        let mut m = 0u128;
        let mut t = (a as i32 + delta) as u8;
        while t != b {
            m |= bit81(t);
            t = (t as i32 + delta) as u8;
        }
        m
    }

    /// 升sへの歩打ちが打ち歩詰めか（sへの歩打ちが相手玉への王手である前提）。盤は動かさない。
    ///
    /// 1. 玉以外の駒で歩を取れるか。ピンされた駒は取れない。ただし歩が玉とその駒の間に入るなら、
    ///    歩が線を遮るのでピンでなくなる
    /// 2. 玉が逃げられるか（歩を取る手を含む）。歩は自分の飛び利きを遮るので、
    ///    sを通って先の升へ届いていた線の分を利き数から引く
    pub(crate) fn pawn_drop_mates(&self, s: u8) -> bool {
        let us = self.side;
        let ui = us.index();
        let them = us.flip();
        let ti = them.index();
        let k = self.king[ti];
        let pawn_dir = dir_between(k, s);
        for x in iter81(self.attackers_with(them, s, 0) & !bit81(k)) {
            let blocked_by_pawn = dir_between(k, x) == pawn_dir;
            if blocked_by_pawn || !self.is_pinned(them, x) {
                return false;
            }
        }
        let through = self.longdir[ui][s as usize];
        for &delta in DELTA.iter() {
            let t = (k as i32 + delta as i32) as u8;
            if t != s && !CAN_MOVE_TO[ti][self.board[t as usize] as usize] {
                continue;
            }
            let mut n = self.count[ui][t as usize];
            if t != s {
                let mut dirs = through;
                while dirs != 0 {
                    let e = dirs.trailing_zeros() as usize;
                    dirs &= dirs - 1;
                    if (s as i32 + DELTA[e] as i32) as u8 == t {
                        n -= 1;
                    }
                }
            }
            if n == 0 {
                return false;
            }
        }
        true
    }

    /// 擬似合法手mが合法か（coreの `is_legal` と同じ値）。
    pub fn is_legal(&self, m: Move) -> bool {
        let us = self.side;
        let ti = us.flip().index();
        if m.is_drop() {
            if m.drop_type() == PAWN && (m.to() as i32 + pawn_delta(us)) as u8 == self.king[ti] {
                return !self.pawn_drop_mates(m.to());
            }
            return true;
        }
        let from = m.from();
        let to = m.to();
        if piece_type(self.board[from as usize]) == KING {
            let xray = self.longdir[ti][from as usize];
            return self.count[ti][to as usize] == 0
                && dir_between(from, to).is_none_or(|d| xray & (1 << d) == 0);
        }
        if self.is_pinned(us, from) {
            let k = self.king[us.index()];
            let d = dir_between(k, from).expect("ピンされた駒は玉と同じ線にいる");
            return matches!(dir_between(k, to), Some(e) if e == d || e == opp(d));
        }
        true
    }

    /// mがこの局面の擬似合法手か（coreの `pseudo_legal` と同じ値）。
    pub fn pseudo_legal(&self, m: Move) -> bool {
        if m == Move::NONE {
            return false;
        }
        let us = self.side;
        let ui = us.index();
        let to = m.to();
        if to as usize >= LEN || self.board[to as usize] == WALL {
            return false;
        }
        let pa = m.piece_after();
        if pa == EMPTY || piece_color(pa) != us {
            return false;
        }
        let checkers = self.state().checkers;
        let k = self.king[ui];
        if m.is_drop() {
            let pt = m.drop_type();
            if !(GOLD..=ROOK).contains(&pt)
                || self.hand_ids[ui][hand_index(pt)] == 0
                || self.board[to as usize] != EMPTY
            {
                return false;
            }
            let rel = rel_rank(us, to);
            let ok = match pt {
                PAWN => rel >= 1 && self.pawn_files[ui] & (1 << file_of(to)) == 0,
                LANCE => rel >= 1,
                KNIGHT => rel >= 2,
                _ => true,
            };
            if !ok {
                return false;
            }
            if checkers != 0 {
                if checkers.count_ones() > 1 {
                    return false;
                }
                let csq = IDX81[checkers.trailing_zeros() as usize];
                if self.between81(k, csq) & bit81(to) == 0 {
                    return false;
                }
            }
            return true;
        }
        let from = m.from();
        if from as usize >= LEN {
            return false;
        }
        let pc = self.board[from as usize];
        if pc != m.piece_before() || pc == EMPTY || pc == WALL || piece_color(pc) != us {
            return false;
        }
        if self.targets_of(from, pc, ui) & bit81(to) == 0 {
            return false;
        }
        let pt = piece_type(pc);
        if m.is_promote() {
            if !can_promote(pt) || !(rel_rank(us, from) <= 2 || rel_rank(us, to) <= 2) {
                return false;
            }
        } else if dead_square(us, pt, to) {
            return false;
        }
        if checkers != 0 && pt != KING {
            if checkers.count_ones() > 1 {
                return false;
            }
            let csq = IDX81[checkers.trailing_zeros() as usize];
            if to != csq && self.between81(k, csq) & bit81(to) == 0 {
                return false;
            }
        }
        true
    }

    /// 合法手mが相手玉への王手になるか（coreの `gives_check` と同じ値）。
    pub fn gives_check(&self, m: Move) -> bool {
        let us = self.side;
        let ksq = self.king[us.flip().index()];
        let to = m.to();
        let np = m.piece_after();
        let mv = &MOVES[np as usize];
        let diff = ksq as i32 - to as i32;
        if diff.abs() <= 2 * STRIDE && mv.short[..mv.n_short as usize].contains(&(diff as i8)) {
            return true;
        }
        let from = if m.is_drop() { 0 } else { m.from() };
        if let Some(d) = dir_between(to, ksq)
            && mv.long & (1 << d) != 0
        {
            let delta = DELTA[d] as i32;
            let mut t = (to as i32 + delta) as u8;
            let mut clear = true;
            while t != ksq {
                if self.board[t as usize] != EMPTY && t != from {
                    clear = false;
                    break;
                }
                t = (t as i32 + delta) as u8;
            }
            if clear {
                return true;
            }
        }
        if m.is_drop() {
            return false;
        }
        // 開き王手: 移動元が、相手玉へ向かう自分の飛び利きをただ1枚で遮っている
        let Some(d) = dir_between(ksq, from) else {
            return false;
        };
        self.first_piece(ksq, d) == Some(from)
            && self.longdir[us.index()][from as usize] & (1 << opp(d)) != 0
            && !matches!(dir_between(ksq, to), Some(e) if e == d || e == opp(d))
    }

    /// 攻め駒の集合から最も安い駒を選ぶ。同じ組なら小さい升。(升, 価値, 玉か)
    #[inline]
    fn least_valuable(&self, att: u128) -> (u8, i32, bool) {
        let mut best = (0u8, u8::MAX);
        for s in iter81(att) {
            let r = see_rank(piece_type(self.board[s as usize]));
            if r < best.1 {
                best = (s, r);
            }
        }
        (best.0, SEE_VALUE[best.1 as usize], best.1 == 9)
    }

    /// mの静的交換評価がthreshold以上か（coreの `see_ge` と同じ手順と値）。
    pub fn see_ge(&self, m: Move, threshold: i32) -> bool {
        let us = self.side;
        let them = us.flip();
        let to = m.to();
        let (captured, placed, removed0) = if m.is_drop() {
            (0, PIECE_VALUE[m.drop_type() as usize], 0u128)
        } else {
            let from = m.from();
            let before = PIECE_VALUE[piece_type(self.board[from as usize]) as usize];
            let after = PIECE_VALUE[piece_type(m.piece_after()) as usize];
            (
                PIECE_VALUE[piece_type(self.board[to as usize]) as usize] + after - before,
                after,
                bit81(from),
            )
        };
        let mut swap = captured - threshold;
        if swap < 0 {
            return false;
        }
        swap = placed - swap;
        if swap <= 0 {
            return true;
        }
        // 取り合いなし: 移動先に相手の利きがなく、移動元に相手の飛び利きも届いていない
        if self.count[them.index()][to as usize] == 0
            && (m.is_drop() || self.longdir[them.index()][m.from() as usize] == 0)
        {
            return true;
        }
        let mut removed = removed0;
        let mut stm = us;
        let mut res = true;
        loop {
            stm = stm.flip();
            let att = self.attackers_with(stm, to, removed);
            if att == 0 {
                break;
            }
            res = !res;
            let (best, val, is_king) = self.least_valuable(att);
            if is_king {
                if self.attackers_with(stm.flip(), to, removed | bit81(best)) != 0 {
                    res = !res;
                }
                break;
            }
            swap = val - swap;
            if swap < i32::from(res) {
                break;
            }
            removed |= bit81(best);
        }
        res
    }

    /// 1手詰めがあればその手を返す（coreの `mate_1ply` と同じ手を返す）。王手されていない局面で呼ぶ。
    ///
    /// 候補はcoreと同じで、同じ順に調べる。盤上の駒は玉の8近傍へ動く手（桂は王手の升へ跳ぶ手）、
    /// 成りを先に、行き所のある不成を後に。駒打ちは金・銀・桂・香・飛・角の順に、玉の隣（桂は王手の升）へ打つ手。
    pub fn mate_1ply(&mut self) -> Option<Move> {
        debug_assert!(!self.in_check());
        let us = self.side;
        let ui = us.index();
        let ksq = self.king[us.flip().index()];
        let mut near = 0u128;
        for &delta in DELTA.iter() {
            let t = (ksq as i32 + delta as i32) as u8;
            if self.board[t as usize] != WALL {
                near |= bit81(t);
            }
        }
        let hit_from = |pt: u8| -> u128 {
            let m = &MOVES[make_piece(us, pt) as usize];
            let mut set = 0u128;
            for k in 0..m.n_short as usize {
                let x = (ksq as i32 - m.short[k] as i32) as u8;
                if self.board[x as usize] != WALL {
                    set |= bit81(x);
                }
            }
            set
        };
        let adjacent = |dirs: [usize; 4]| -> u128 {
            let mut set = 0u128;
            for d in dirs {
                let x = (ksq as i32 + DELTA[d] as i32) as u8;
                if self.board[x as usize] != WALL {
                    set |= bit81(x);
                }
            }
            set
        };
        let knight_to = hit_from(KNIGHT);
        let empty = ALL81 & !(self.occ[0] | self.occ[1]);
        let lance_sq = (ksq as i32 - pawn_delta(us)) as u8;
        let lance_to = if self.board[lance_sq as usize] != WALL {
            bit81(lance_sq)
        } else {
            0
        };
        // 駒打ちの候補の升。coreと同じ駒種の順
        let drops: [(u8, u128); 6] = [
            (GOLD, hit_from(GOLD)),
            (SILVER, hit_from(SILVER)),
            (KNIGHT, knight_to),
            (LANCE, lance_to),
            (ROOK, adjacent([UP, DOWN, LEFT, RIGHT])),
            (BISHOP, adjacent([UL, DR, UR, DL])),
        ];
        let movers = self.occ[ui] & !bit81(self.king[ui]);
        for s in iter81(movers) {
            let p = self.board[s as usize];
            let pt = piece_type(p);
            let targets = self.targets_of(s, p, ui) & if pt == KNIGHT { knight_to } else { near };
            for t in iter81(targets) {
                if can_promote(pt) && (rel_rank(us, s) <= 2 || rel_rank(us, t) <= 2) {
                    let m = Move::normal(s, t, true, p - 8);
                    if self.is_mate_move(m) {
                        return Some(m);
                    }
                }
                if !dead_square(us, pt, t) {
                    let m = Move::normal(s, t, false, p);
                    if self.is_mate_move(m) {
                        return Some(m);
                    }
                }
            }
        }
        for (pt, targets) in drops {
            if self.hand_ids[ui][hand_index(pt)] == 0 {
                continue;
            }
            for t in iter81(targets & empty) {
                if dead_square(us, pt, t) {
                    continue;
                }
                let m = Move::drop(pt, t, us);
                if self.is_mate_move(m) {
                    return Some(m);
                }
            }
        }
        None
    }

    /// 候補mが詰みか。合法なら進め、王手を外す手があるかを利きの配列で見て戻す。
    fn is_mate_move(&mut self, m: Move) -> bool {
        if !self.is_legal(m) {
            return false;
        }
        self.do_move(m);
        let mate = self.no_evasion_core_rules();
        self.undo_move(m);
        mate
    }

    /// 手番側（王手をかけられた側）に、coreの1手詰めの判定で認める王手の外し方が無いか。
    ///
    /// 玉の移動・王手駒を取る手・盤上の駒の合駒を見る。合駒の升があって持ち駒が1枚でもあれば、
    /// 打って遮れるとみなす（coreの取り決め。打てない歩でも数える）。
    fn no_evasion_core_rules(&self) -> bool {
        let def = self.side;
        let di = def.index();
        let ai = def.flip().index();
        let k = self.king[di];
        let checkers = self.state().checkers;
        if checkers == 0 {
            return false;
        }
        let xray = self.longdir[ai][k as usize];
        for (d, &delta) in DELTA.iter().enumerate() {
            let t = (k as i32 + delta as i32) as usize;
            if CAN_MOVE_TO[di][self.board[t] as usize]
                && self.count[ai][t] == 0
                && xray & (1 << d) == 0
            {
                return false;
            }
        }
        if checkers.count_ones() >= 2 {
            return true;
        }
        let csq = IDX81[checkers.trailing_zeros() as usize];
        let not_king = !bit81(k);
        for x in iter81(self.attackers_with(def, csq, 0) & not_king) {
            if !self.is_pinned(def, x) {
                return false;
            }
        }
        let inter = self.between81(k, csq);
        if inter != 0 {
            if self.hand_ids[di].iter().any(|&h| h != 0) {
                return false;
            }
            for s in iter81(inter) {
                for x in iter81(self.attackers_with(def, s, 0) & not_king) {
                    if !self.is_pinned(def, x) {
                        return false;
                    }
                }
            }
        }
        true
    }
}
