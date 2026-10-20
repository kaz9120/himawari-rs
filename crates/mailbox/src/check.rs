//! 段階5の盤面操作。合法性・擬似合法性・王手の判定・SEE（ADR-0241の第6回）。1手詰めは `mate.rs` にある。
//!
//! どれもcoreの同名の関数と同じ値を返す（照合のテストで確かめる）。
//! 利きの配列を引いて決め、Bitboardの利きの計算は使わない。
//!
//! - `is_legal`: 擬似合法手の合法性。玉の手は利き数と飛び利きの延長、ほかはピンで決める
//! - `pseudo_legal`: 置換表から来た手が、この局面の擬似合法手か
//! - `gives_check`: ノードごとに1回、相手玉から8方向の線と開き王手の駒を求めておき、表を引いて決める
//! - `see_ge`: coreと同じswapの手順。攻め駒は升から8方向と近傍を見て集める

use std::cell::Cell;

use crate::board::*;
use crate::movegen::pawn_delta;
use crate::moves::Move;
use crate::position::Position;
use crate::zobrist::PIECE_VALUE;

/// 81升の集合を小さい升の順に、升の添字で返す。
#[inline]
pub(crate) fn iter81(mut m: u128) -> impl Iterator<Item = u8> {
    std::iter::from_fn(move || {
        if m == 0 {
            return None;
        }
        let i = m.trailing_zeros() as usize;
        m &= m - 1;
        Some(IDX81[i])
    })
}

/// 相手玉から見た王手の手がかり（coreの `check_squares` と `blockers_for_king` に当たる）。
#[derive(Copy, Clone, Default, PartialEq, Eq, Debug)]
pub(crate) struct CheckInfo {
    /// 方向dの、玉から最初の駒まで（その駒の升を含む）の升の集合（81升）
    rays: [u128; 8],
    /// 動くと開き王手になりうる手番側の駒の升の集合
    blockers: u128,
    /// 相手玉の81升の通し番号
    k81: u8,
}

/// 局面の深さ1つ分の `CheckInfo` の置き場。
#[derive(Clone, Default)]
pub(crate) struct CheckSlot {
    pub(crate) valid: Cell<bool>,
    info: Cell<CheckInfo>,
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

/// 81升の通し番号が増える向きの方向（下・左・左上・左下）のビット。
const INCREASING81: u8 = (1 << DOWN) | (1 << LEFT) | (1 << UL) | (1 << DL);

/// 81升の通し番号aから方向dへ進み、集合occの中で最初に当たる升（81升）。無ければNone。
///
/// 盤を1升ずつ歩かずに、線の集合と駒のいる升の集合の積をビット走査して決める。
#[inline]
pub(crate) fn first81(a81: u8, d: usize, occ: u128) -> Option<u8> {
    let r = RAY81[a81 as usize][d] & occ;
    if r == 0 {
        return None;
    }
    Some(if INCREASING81 & (1 << d) != 0 {
        r.trailing_zeros() as u8
    } else {
        127 - r.leading_zeros() as u8
    })
}

/// 色cの駒が短い利きで升toへ利くとき、駒のいる升の差分 `to − 駒の升` の一覧と、
/// その差分で利く駒の符号の集合（bit p）。玉の8近傍と、色cの桂の2升の10通り。
pub(crate) const SHORT_FROM: [[(i8, u32); 10]; 2] = {
    let mut t = [[(0i8, 0u32); 10]; 2];
    let mut c = 0;
    while c < 2 {
        let mut k = 0;
        while k < 10 {
            let o = if k < 8 {
                DELTA[k]
            } else if c == 0 {
                KNIGHT_STEPS[k - 8]
            } else {
                -KNIGHT_STEPS[k - 8]
            };
            let mut mask = 0u32;
            let mut p = 2;
            while p < 32 {
                if (p >= 16) == (c == 1) && (p & 15) >= 2 {
                    let m = &MOVES[p];
                    let mut j = 0;
                    while j < m.n_short as usize {
                        if m.short[j] == o {
                            mask |= 1 << p;
                        }
                        j += 1;
                    }
                }
                p += 1;
            }
            t[c][k] = (o, mask);
            k += 1;
        }
        c += 1;
    }
    t
};

const SEE_VALUE: [i32; 10] = [90, 315, 405, 495, 540, 855, 945, 990, 1395, 15000];

/// 行き所のない駒になるので不成では入れない升か。
#[inline]
pub(crate) fn dead_square(us: Color, pt: u8, to: u8) -> bool {
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
        let ci = c.index();
        let mut set = 0u128;
        for &(o, mask) in SHORT_FROM[ci].iter() {
            let x = sq.wrapping_sub(o as u8);
            if (mask >> (self.board[x as usize] & 31)) & 1 != 0 {
                set |= bit81(x);
            }
        }
        set &= !removed;
        let occ = (self.occ[0] | self.occ[1]) & !removed;
        let s81 = SQ81[sq as usize];
        for d in 0..8 {
            if let Some(x81) = first81(s81, d, occ)
                && self.occ[ci] & (1u128 << x81) != 0
                && MOVES[self.board[IDX81[x81 as usize] as usize] as usize].long & (1 << opp(d))
                    != 0
            {
                set |= 1u128 << x81;
            }
        }
        set
    }

    /// 升aと升bの間の升の集合（同じ線に無ければ空）。
    pub fn between81(&self, a: u8, b: u8) -> u128 {
        // 盤を歩かず、aからbへの線とbから先の線の差で引く（ADR-0241の第16回）
        let (a81, b81) = (SQ81[a as usize] as usize, SQ81[b as usize] as usize);
        let d = dir81(a81, b81);
        if d == NO_DIR {
            return 0;
        }
        let d = d as usize;
        ray81(a81, d) & !ray81(b81, d) & !(1u128 << b81)
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
        // ピン: 自玉から移動元への線の上で、移動元が最初の駒であり、相手の飛び利きが玉へ向かって届いている。
        // 方向ビットの検査を先に置く（ほとんどの手はここで抜ける）
        let k81 = SQ81[self.king[us.index()] as usize];
        let f81 = SQ81[from as usize];
        let d = DIR81[k81 as usize][f81 as usize];
        if d == NO_DIR || self.longdir[ti][from as usize] & (1 << opp(d as usize)) == 0 {
            return true;
        }
        if first81(k81, d as usize, self.occ[0] | self.occ[1]) != Some(f81) {
            return true;
        }
        let e = DIR81[k81 as usize][SQ81[to as usize] as usize];
        e == d || e == (d ^ 1)
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
                || !self.has_hand(ui, pt)
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
        // 駒がtoへ動けるか。短い利きは表、飛び利きは線の間の升が空いているかで決める（盤を歩かない）
        let (from81, to81) = (SQ81[from as usize] as usize, SQ81[to as usize] as usize);
        if self.occ[ui] & (1u128 << to81) != 0 {
            return false;
        }
        if short81(pc, from81) & (1u128 << to81) == 0 {
            let d = dir81(from81, to81);
            if d == NO_DIR
                || MOVES[(pc & 31) as usize].long & (1 << d) == 0
                || self.between81(from, to) & (self.occ[0] | self.occ[1]) != 0
            {
                return false;
            }
        }
        debug_assert!(self.targets_of(from, pc, ui) & bit81(to) != 0);
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

    /// 王手の判定の手がかりを、この深さで初めてなら作って返す。
    #[inline]
    fn check_info(&self) -> &CheckInfo {
        let slot = &self.check_slots[self.check_slots_depth()];
        if !slot.valid.get() {
            slot.info.set(self.compute_check_info());
            slot.valid.set(true);
        }
        // SAFETY: 中身を書き換えるのは、上の無効な欄を埋めるときと、`push_state`（&mut self）が
        // 欄を無効にした後だけである。有効な欄へ返した参照が生きている間は&selfが借りられているので、
        // 同じ欄が書き換えられることはない
        let ci = unsafe { &*slot.info.as_ptr() };
        debug_assert!(
            *ci == self.compute_check_info(),
            "王手の手がかりが盤面と食い違う（無効化漏れ）"
        );
        ci
    }

    /// 相手玉から8方向の、最初の駒まで（その駒の升を含む）の線。`check_squares` が使う。
    #[inline]
    pub(crate) fn check_rays(&self) -> &[u128; 8] {
        &self.check_info().rays
    }

    /// 相手玉から8方向の最初の駒まで（その駒の升を含む）の線と、開き王手の駒を求める。
    fn compute_check_info(&self) -> CheckInfo {
        let us = self.side;
        let ui = us.index();
        let ksq = self.king[us.flip().index()];
        let k81 = SQ81[ksq as usize];
        let mut ci = CheckInfo {
            k81,
            ..CheckInfo::default()
        };
        let occ = self.occ[0] | self.occ[1];
        let mine = self.occ[ui];
        for (d, &ray) in RAY81[k81 as usize].iter().enumerate() {
            match first81(k81, d, occ) {
                None => ci.rays[d] = ray,
                Some(x81) => {
                    ci.rays[d] = ray & !RAY81[x81 as usize][d];
                    // 開き王手の駒: 玉から最初の駒が手番側の駒で、その升に玉へ向かう自分の飛び利きが届いている
                    let xb = 1u128 << x81;
                    if mine & xb != 0
                        && self.longdir[ui][IDX81[x81 as usize] as usize] & (1 << opp(d)) != 0
                    {
                        ci.blockers |= xb;
                    }
                }
            }
        }
        ci
    }

    /// 合法手mが相手玉への王手になるか（coreの `gives_check` と同じ値）。
    ///
    /// 直接王手の飛び利きは、指す前の盤の線で決める。coreの `gives_check` と同じ理由で、指す前と後で
    /// 結果は変わらない。移動元が行き先と玉の間にあるとき、指す前の駒も同じ線を走るので、
    /// 玉との間が空いていれば指す前から王手になり、局面が不正になる。
    #[inline]
    pub fn gives_check(&self, m: Move) -> bool {
        let r = self.gives_check_with(self.check_info(), m);
        debug_assert_eq!(r, self.gives_check_slow(m), "gives_check: {}", m.to_usi());
        r
    }

    #[inline]
    fn gives_check_with(&self, ci: &CheckInfo, m: Move) -> bool {
        let ksq = self.king[self.side.flip().index()];
        let to = m.to();
        let np = m.piece_after();
        if short_hit(np, ksq as i32 - to as i32) {
            return true;
        }
        let to81 = SQ81[to as usize];
        let to_bit = 1u128 << to81;
        // 玉から行き先への方向。駒は逆向きに走って玉へ届く
        let dk = DIR81[ci.k81 as usize][to81 as usize];
        if dk != NO_DIR
            && MOVES[np as usize].long & (1 << opp(dk as usize)) != 0
            && ci.rays[dk as usize] & to_bit != 0
        {
            return true;
        }
        if m.is_drop() {
            return false;
        }
        // 開き王手: 移動元が開き王手の駒で、行き先がその線（玉を通る直線）から外れる
        let from81 = SQ81[m.from() as usize];
        if ci.blockers & (1u128 << from81) == 0 {
            return false;
        }
        let df = DIR81[ci.k81 as usize][from81 as usize];
        dk != df && dk != (df ^ 1)
    }

    /// 盤を歩いて決める版。キャッシュの経路の正解器で、デバッグの検査とテストからだけ呼ぶ。
    #[doc(hidden)]
    pub fn gives_check_slow(&self, m: Move) -> bool {
        let us = self.side;
        let ksq = self.king[us.flip().index()];
        let to = m.to();
        let np = m.piece_after();
        let mv = &MOVES[np as usize];
        if short_hit(np, ksq as i32 - to as i32) {
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
        // 取り合いなし: 移動先に相手の利きがなく、移動元を通って移動先へ向かう相手の飛び利きも無い
        if self.count[them.index()][to as usize] == 0
            && (m.is_drop() || {
                let d = DIR81[SQ81[m.from() as usize] as usize][SQ81[to as usize] as usize];
                d == NO_DIR || self.longdir[them.index()][m.from() as usize] & (1 << d) == 0
            })
        {
            return true;
        }
        // 攻め駒は最初に1回だけ集め、取り除いた駒の後ろから伸びる飛び利きだけを足していく
        let mut att = [
            self.see_attackers(Color::Black, to),
            self.see_attackers(Color::White, to),
        ];
        let mut removed = removed0;
        if !m.is_drop() {
            let from = m.from();
            att[us.index()] &= !bit81(from);
            self.add_xray(to, from, removed, &mut att);
        }
        let mut stm = us;
        let mut res = true;
        loop {
            stm = stm.flip();
            let a = att[stm.index()];
            if a == 0 {
                break;
            }
            res = !res;
            let (best, val, is_king) = self.least_valuable(a);
            if is_king {
                let mut rest = att;
                rest[stm.index()] &= !bit81(best);
                self.add_xray(to, best, removed | bit81(best), &mut rest);
                if rest[stm.flip().index()] != 0 {
                    res = !res;
                }
                break;
            }
            swap = val - swap;
            if swap < i32::from(res) {
                break;
            }
            removed |= bit81(best);
            att[stm.index()] &= !bit81(best);
            self.add_xray(to, best, removed, &mut att);
        }
        res
    }

    /// 色cの駒のうち、いまの盤で升toへ利いている駒の升の集合。
    /// 利き数が0なら調べない。飛び利きは方向ビットの向きの最初の駒を、駒のいる升の集合から引く。
    /// 短い利きの駒は、利き数から飛び利きの本数を引いた数だけ見つけたら打ち切る。
    #[inline]
    fn see_attackers(&self, c: Color, to: u8) -> u128 {
        let ci = c.index();
        let n = self.count[ci][to as usize];
        if n == 0 {
            return 0;
        }
        let ld = self.longdir[ci][to as usize];
        let to81 = SQ81[to as usize];
        let mut set = 0u128;
        if ld != 0 {
            let occ = self.occ[0] | self.occ[1];
            let mut dirs = ld;
            while dirs != 0 {
                let d = dirs.trailing_zeros() as usize;
                dirs &= dirs - 1;
                // 方向dへ進む飛び利きなので、駒はopp(d)の向きにいる
                let x81 = first81(to81, opp(d), occ).expect("飛び利きの元の駒がある");
                set |= 1u128 << x81;
            }
        }
        let mut need = u32::from(n) - ld.count_ones();
        if need != 0 {
            for &(o, mask) in SHORT_FROM[ci].iter() {
                let x = to.wrapping_sub(o as u8);
                let v = self.board[x as usize];
                if (mask >> (v & 31)) & 1 != 0 {
                    set |= bit81(x);
                    need -= 1;
                    if need == 0 {
                        break;
                    }
                }
            }
        }
        set
    }

    /// 升toへの攻め駒xを取り除いたとき、xの後ろからtoへ伸びる飛び利きの駒をattへ足す。
    /// removedの升は空きとみなして通す。
    #[inline]
    fn add_xray(&self, to: u8, x: u8, removed: u128, att: &mut [u128; 2]) {
        let x81 = SQ81[x as usize];
        let d = DIR81[SQ81[to as usize] as usize][x81 as usize];
        if d == NO_DIR {
            return;
        }
        let d = d as usize;
        let occ = (self.occ[0] | self.occ[1]) & !removed;
        let Some(y81) = first81(x81, d, occ) else {
            return;
        };
        let v = self.board[IDX81[y81 as usize] as usize];
        if MOVES[v as usize].long & (1 << opp(d)) != 0 {
            att[piece_color(v).index()] |= 1u128 << y81;
        }
    }
}
