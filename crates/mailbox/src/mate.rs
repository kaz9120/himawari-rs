//! 1手詰め（ADR-0241の第8回）。盤を動かさずに判定する。
//!
//! coreの `mate_1ply` と同じ候補を同じ順に調べ、同じ手を返す。
//! coreの判定は健全だが不完全で、合駒の升があれば持ち駒が1枚でもあるだけで「打って遮れる」とみなす。
//! 返す手を揃えるため、その取り決めも同じにする。
//!
//! 候補ごとの判定（`is_mate_move`）は、coreの `is_mate_move` と同じ4段で行う。
//! coreは移動後の占有でBitboardの利きを引き直す。配列盤は指す前の利き数と方向ビットを読み、
//! 指した手で変わる分だけを足し引きする。変わるのは3つの升だけである。
//!
//! - 行き先: 駒が置かれる。空きだったなら、そこを通っていた飛び利きが切れる
//! - 移動元: 空く。そこで止まっていた飛び利きが先へ伸びる
//! - 相手玉: 逃げ場を調べるときは空きとみなす。指す前に手番側の利きは玉へ届いていない
//!   （手番でない側は王手されていない）ので、玉の升で止まっていた手番側の線は無い

use crate::board::*;
use crate::check::{dead_square, iter81};
use crate::movegen::pawn_delta;
use crate::moves::Move;
use crate::position::Position;

/// 駒打ちの手の移動元に入れる値。盤の歩きで触れる升とは一致しない。
const NO_FROM: u8 = 0xFF;

/// 空の盤で、升fromの駒pが利く升の集合（81升）。
fn empty_board_targets(from: u8, p: u8) -> u128 {
    let on = |t: i32| (0..LEN as i32).contains(&t) && SQ81[t as usize] != 0xFF;
    let m = &MOVES[p as usize];
    let mut out = 0u128;
    for k in 0..m.n_short as usize {
        let t = from as i32 + m.short[k] as i32;
        if on(t) {
            out |= bit81(t as u8);
        }
    }
    for (d, &delta) in DELTA.iter().enumerate() {
        if m.long & (1 << d) == 0 {
            continue;
        }
        let mut t = from as i32 + delta as i32;
        while on(t) {
            out |= bit81(t as u8);
            t += delta as i32;
        }
    }
    out
}

/// 1手詰めの候補の移動元（coreの `CHECK_CAND` と同じ考え方）。
/// `[玉の81升][駒の符号]` は、その駒が空の盤で玉の近傍（桂は王手の升）へ利く移動元の集合。
/// どの占有に対しても上位集合なので、外れた駒は利きを歩かずに捨てられる。
fn mate_cand() -> &'static [[u128; 32]; 81] {
    static T: std::sync::OnceLock<Box<[[u128; 32]; 81]>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = Box::new([[0u128; 32]; 81]);
        for k81 in 0..81u8 {
            let ksq = IDX81[k81 as usize];
            let near = empty_board_targets(ksq, make_piece(Color::Black, KING));
            for p in 0..32u8 {
                let pt = piece_type(p);
                if pt < 2 || pt == KING {
                    continue;
                }
                let target = if pt == KNIGHT {
                    // 王手の升: 玉の升へ跳べる升
                    let mv = &MOVES[p as usize];
                    let mut s = 0u128;
                    for k in 0..mv.n_short as usize {
                        let x = ksq as i32 - mv.short[k] as i32;
                        if (0..LEN as i32).contains(&x) && SQ81[x as usize] != 0xFF {
                            s |= bit81(x as u8);
                        }
                    }
                    s
                } else {
                    near
                };
                for f81 in 0..81u8 {
                    if f81 == k81 {
                        continue;
                    }
                    let from = IDX81[f81 as usize];
                    if empty_board_targets(from, p) & target != 0 {
                        t[k81 as usize][p as usize] |= bit81(from);
                    }
                }
            }
        }
        t
    })
}

/// 候補の手を指した後の盤を、盤を書き換えずに読むための差分。
struct After {
    us: Color,
    ui: usize,
    them: Color,
    ti: usize,
    /// 相手玉の升
    ksq: u8,
    /// 移動元。駒打ちは `NO_FROM`
    from: u8,
    to: u8,
    /// 行き先に置かれる駒
    np: u8,
    /// 指す前の行き先の中身（空きか、取られる相手の駒）
    cap: u8,
}

impl After {
    #[inline]
    fn is_drop(&self) -> bool {
        self.from == NO_FROM
    }
}

impl Position {
    /// 指した後の升xの中身。king_emptyなら相手玉の升も空きとみなす（玉が逃げた後の利きを見る）。
    #[inline]
    fn after_on(&self, a: &After, x: u8, king_empty: bool) -> u8 {
        if x == a.to {
            a.np
        } else if x == a.from || (king_empty && x == a.ksq) {
            EMPTY
        } else {
            self.board[x as usize]
        }
    }

    /// 指した後の盤で、startから方向dへ進んでtargetに届くか（target自身の中身は問わない）。
    /// targetがその方向の線に無ければ、壁で止まってfalseを返す。
    #[inline]
    fn reach_after(&self, a: &After, start: u8, d: usize, target: u8, king_empty: bool) -> bool {
        let delta = DELTA[d] as i32;
        let mut t = start;
        loop {
            t = (t as i32 + delta) as u8;
            if t == target {
                return true;
            }
            if self.after_on(a, t, king_empty) != EMPTY {
                return false;
            }
        }
    }

    /// 指す前の盤で、startから方向dへ進んでtargetに届くか。
    #[inline]
    fn reach_before(&self, start: u8, d: usize, target: u8) -> bool {
        let delta = DELTA[d] as i32;
        let mut t = start;
        loop {
            t = (t as i32 + delta) as u8;
            if t == target {
                return true;
            }
            if self.board[t as usize] != EMPTY {
                return false;
            }
        }
    }

    /// 指す前の盤で、升sの駒pが升eへ利いているか。
    #[inline]
    fn attacks_before(&self, p: u8, s: u8, e: u8) -> bool {
        if short_hit(p, e as i32 - s as i32) {
            return true;
        }
        let long = MOVES[p as usize].long;
        long != 0
            && matches!(dir_between(s, e), Some(d) if long & (1 << d) != 0 && self.reach_before(s, d, e))
    }

    /// 指した後の盤で、行き先の駒が升eへ利いているか。
    #[inline]
    fn placed_attacks(&self, a: &After, e: u8, king_empty: bool) -> bool {
        if short_hit(a.np, e as i32 - a.to as i32) {
            return true;
        }
        let long = MOVES[a.np as usize].long;
        long != 0
            && matches!(dir_between(a.to, e), Some(d)
                if long & (1 << d) != 0 && self.reach_after(a, a.to, d, e, king_empty))
    }

    /// 指す前に移動元へ届いていた色ciの飛び利きのうち、移動元が空いて升sqまで伸びるものの元の駒。
    /// 元の駒と移動元の間に行き先が入る手では、線は行き先で止まるので伸びない。
    #[inline]
    fn opened_line(&self, a: &After, ci: usize, sq: u8, king_empty: bool) -> Option<u8> {
        if a.is_drop() {
            return None;
        }
        let d = dir_between(a.from, sq)?;
        if self.longdir[ci][a.from as usize] & (1 << d) == 0
            || !self.reach_after(a, a.from, d, sq, king_empty)
        {
            return None;
        }
        let od = opp(d);
        if dir_between(a.from, a.to) == Some(od) && self.reach_before(a.from, od, a.to) {
            return None;
        }
        self.first_piece(a.from, od)
    }

    /// 相手玉が升eへ逃げた後に、手番側の駒がeへ利いているか。
    ///
    /// 指す前のeの利き数から、動いた駒の分と、行き先で切れる線の分を引き、移動元で伸びる線の分を足す。
    /// 行き先に置いた駒の利きは別に見る。
    fn us_attacks_after(&self, a: &After, e: u8) -> bool {
        if self.placed_attacks(a, e, true) {
            return true;
        }
        let mut n = i32::from(self.count[a.ui][e as usize]);
        if !a.is_drop() && self.attacks_before(self.board[a.from as usize], a.from, e) {
            n -= 1;
        }
        // 行き先が空きだったなら、行き先を通ってeへ届いていた線が切れる。動いた駒自身の線は上で引いた
        if a.cap == EMPTY
            && e != a.to
            && let Some(d) = dir_between(a.to, e)
            && self.longdir[a.ui][a.to as usize] & (1 << d) != 0
            && self.reach_before(a.to, d, e)
            && self.first_piece(a.to, opp(d)) != Some(a.from)
        {
            n -= 1;
        }
        if self.opened_line(a, a.ui, e, true).is_some() {
            n += 1;
        }
        n > 0
    }

    /// 指した後に、相手（玉の側）の駒が升sqへ利いている集合。玉と行き先の駒は除く。
    fn them_attackers_after(&self, a: &After, sq: u8) -> u128 {
        let mut set = self.attackers_with(a.them, sq, 0);
        // 行き先が空きだったなら、行き先の先からsqへ届いていた線が切れる
        if a.cap == EMPTY
            && sq != a.to
            && let Some(d) = dir_between(sq, a.to)
            && self.reach_before(sq, d, a.to)
            && let Some(x) = self.first_piece(a.to, d)
        {
            set &= !bit81(x);
        }
        if let Some(x) = self.opened_line(a, a.ti, sq, false) {
            set |= bit81(x);
        }
        set & !bit81(a.ksq) & !bit81(a.to)
    }

    /// 指した後の盤で、相手の駒xを取り除く（blockの升には駒を置く）と、手番側の駒が相手玉へ利くか。
    /// exceptの升の駒は数えない（王手駒を取る手で、取られる王手駒を除く）。
    fn pinned_after(&self, a: &After, x: u8, except: u8, block: u8) -> bool {
        let Some(d) = dir_between(a.ksq, x) else {
            return false;
        };
        let delta = DELTA[d] as i32;
        let mut t = a.ksq;
        loop {
            t = (t as i32 + delta) as u8;
            if t == x {
                break;
            }
            if t == block || self.after_on(a, t, false) != EMPTY {
                return false;
            }
        }
        loop {
            t = (t as i32 + delta) as u8;
            if t == block {
                return false;
            }
            let v = self.after_on(a, t, false);
            if v == EMPTY {
                continue;
            }
            return v != WALL
                && piece_color(v) == a.us
                && MOVES[v as usize].long & (1 << opp(d)) != 0
                && t != except;
        }
    }

    /// 候補mが詰みか。盤は動かさない。coreの `is_mate_move` と同じ4段で、同じ値を返す。
    fn is_mate_move(&self, m: Move) -> bool {
        if !self.is_legal(m) {
            return false;
        }
        let us = self.side;
        let them = us.flip();
        let (from, cap) = if m.is_drop() {
            (NO_FROM, EMPTY)
        } else {
            (m.from(), self.board[m.to() as usize])
        };
        let a = After {
            us,
            ui: us.index(),
            them,
            ti: them.index(),
            ksq: self.king[them.index()],
            from,
            to: m.to(),
            np: m.piece_after(),
            cap,
        };
        let ksq = a.ksq;

        // 1. 王手になっているか。行き先の駒による直接王手と、移動元が空いた開き王手
        let direct = self.placed_attacks(&a, ksq, false);
        let disc = self.opened_line(&a, a.ui, ksq, false);
        if !direct && disc.is_none() {
            return false;
        }

        // 2. 玉の逃げ場。行き先の駒を取って逃げる手も含む
        for &delta in DELTA.iter() {
            let e = (ksq as i32 + delta as i32) as u8;
            let v = self.after_on(&a, e, false);
            if v == WALL || (v != EMPTY && piece_color(v) != us) {
                continue;
            }
            if !self.us_attacks_after(&a, e) {
                return false;
            }
        }

        // 両王手は玉が動くしかない
        let csq = match (direct, disc) {
            (true, Some(_)) => return true,
            (true, None) => a.to,
            (false, Some(s)) => s,
            (false, None) => unreachable!(),
        };

        // 3. 玉以外の駒で王手駒を取る。取った後も王手が残るなら回避にならない
        for x in iter81(self.them_attackers_after(&a, csq)) {
            if !self.pinned_after(&a, x, csq, NO_FROM) {
                return false;
            }
        }

        // 4. 合駒。玉と王手駒の間がある場合
        let inter = self.between81(ksq, csq);
        if inter != 0 {
            // 持ち駒があれば遮れるとみなす（coreの取り決め。打てない歩でも数える）
            if self.hand_ids[a.ti].iter().any(|&h| h != 0) {
                return false;
            }
            for s in iter81(inter) {
                for x in iter81(self.them_attackers_after(&a, s)) {
                    if !self.pinned_after(&a, x, NO_FROM, s) {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// `is_mate_move` を外から呼ぶ入口。照合のテストだけが使う。mは合法手であること。
    #[doc(hidden)]
    pub fn is_mate_move_for_test(&self, m: Move) -> bool {
        self.is_mate_move(m)
    }

    /// 手を進めて判定する版（第6回の実装）。盤を動かさない判定の正解器で、照合のテストだけが使う。
    #[doc(hidden)]
    pub fn is_mate_move_by_move(&mut self, m: Move) -> bool {
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

    /// 1手詰めがあればその手を返す（coreの `mate_1ply` と同じ手を返す）。王手されていない局面で呼ぶ。
    ///
    /// 候補はcoreと同じで、同じ順に調べる。盤上の駒は玉の8近傍へ動く手（桂は王手の升へ跳ぶ手）、
    /// 成りを先に、行き所のある不成を後に。駒打ちは金・銀・桂・香・飛・角の順に、玉の隣（桂は王手の升）へ打つ手。
    pub fn mate_1ply(&self) -> Option<Move> {
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
        // 玉の逃げ場: 玉の隣で、相手（玉の側）の駒がなく、手番側の利きもない升（方向のビット）
        let mut escapes = 0u8;
        for (d, &delta) in DELTA.iter().enumerate() {
            let t = (ksq as i32 + delta as i32) as usize;
            let v = self.board[t];
            if v != WALL && (v == EMPTY || piece_color(v) == us) && self.count[ui][t] == 0 {
                escapes |= 1 << d;
            }
        }
        let cand = &mate_cand()[SQ81[ksq as usize] as usize];
        let movers = self.occ[ui] & !bit81(self.king[ui]);
        for s in iter81(movers) {
            let p = self.board[s as usize];
            // 空の盤でも玉の近傍（桂は王手の升）へ届かない駒は、利きを歩かずに捨てる
            if cand[p as usize] & bit81(s) == 0 {
                continue;
            }
            let pt = piece_type(p);
            let targets = self.targets_of(s, p, ui) & if pt == KNIGHT { knight_to } else { near };
            // 移動元に手番側の飛び利きが当たっていれば、動かすと利きが伸びうる。
            // その手は利き数で決めず、玉に取られるかを升の利きで調べ直す（coreの `opens_line`）
            let open = self.longdir[ui][s as usize] != 0;
            for t in iter81(targets) {
                if near & bit81(t) != 0 {
                    let taken = if open {
                        self.attackers_with(us, t, bit81(s) | bit81(ksq)) & !bit81(s) == 0
                    } else {
                        // 動かす駒もtへ利いているので、2枚以上でなければほかに守りがない
                        self.count[ui][t as usize] < 2
                    };
                    if taken {
                        continue;
                    }
                }
                if can_promote(pt) && (rel_rank(us, s) <= 2 || rel_rank(us, t) <= 2) {
                    let m = Move::normal(s, t, true, p - 8);
                    if !(!open && self.escape_left(escapes, ksq, t, p - 8, s))
                        && self.is_mate_move(m)
                    {
                        return Some(m);
                    }
                }
                if !dead_square(us, pt, t) {
                    let m = Move::normal(s, t, false, p);
                    if !(!open && self.escape_left(escapes, ksq, t, p, s)) && self.is_mate_move(m) {
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
                // 打った駒は利き数に入っていないので、守りが0枚なら玉に取られる
                if near & bit81(t) != 0 && self.count[ui][t as usize] == 0 {
                    continue;
                }
                if self.escape_left(escapes, ksq, t, make_piece(us, pt), NO_FROM) {
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

    /// 升tへ置いた駒npの利きで埋まらない逃げ場が残るか（移動元に飛び利きが当たっていない手に限る）。
    ///
    /// 逃げ場は指す前に手番側の利きがない升なので、指した後に利きが増えるのはnpの利きだけである。
    /// npの飛び利きは、玉と移動元fromを空きとみなして通す（玉が線に沿って逃げても利きは届く）。
    /// 駒打ちはfromに `NO_FROM` を渡す。
    #[inline]
    fn escape_left(&self, escapes: u8, ksq: u8, t: u8, np: u8, from: u8) -> bool {
        let mv = &MOVES[np as usize];
        let mut dirs = escapes;
        while dirs != 0 {
            let d = dirs.trailing_zeros() as usize;
            dirs &= dirs - 1;
            let e = (ksq as i32 + DELTA[d] as i32) as u8;
            if e == t || short_hit(np, e as i32 - t as i32) {
                continue;
            }
            let covered = match dir_between(t, e) {
                Some(dd) if mv.long & (1 << dd) != 0 => {
                    let delta = DELTA[dd] as i32;
                    let mut x = (t as i32 + delta) as u8;
                    let mut clear = true;
                    while x != e {
                        if self.board[x as usize] != EMPTY && x != from && x != ksq {
                            clear = false;
                            break;
                        }
                        x = (x as i32 + delta) as u8;
                    }
                    clear
                }
                _ => false,
            };
            if !covered {
                return true;
            }
        }
        false
    }
}
