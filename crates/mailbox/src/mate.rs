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

/// 玉の升と手番ごとの、1手詰めの候補の升の集合（81升）。毎回の作り直しをやめるため表に持つ。
#[derive(Clone, Copy, Default)]
struct KingSets {
    /// 玉の8近傍
    near: u128,
    /// 桂・金・銀・香を打てば（動かせば）王手になる升
    knight_to: u128,
    gold_to: u128,
    silver_to: u128,
    lance_to: u128,
    /// 飛・角を打って王手になる隣の升
    rook_adj: u128,
    bishop_adj: u128,
    /// どの駒種でも、空の盤で短い利きだけで近傍（桂は王手の升）へ届きうる移動元の和集合。
    /// 飛び利きで届く駒は、近傍の升の方向ビットから別に引く
    cand_short: u128,
}

/// 1手詰めの候補を絞る表。
struct MateTable {
    /// `[玉の81升][駒の符号]` は、その駒が空の盤で玉の近傍（桂は王手の升）へ利く移動元の集合
    /// （coreの `CHECK_CAND` と同じ考え方）。どの占有に対しても上位集合なので、外れた駒は利きを歩かずに捨てられる
    cand: [[u128; 32]; 81],
    /// `[玉の81升][手番]`
    sets: [[KingSets; 2]; 81],
}

fn mate_table() -> &'static MateTable {
    static T: std::sync::OnceLock<Box<MateTable>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = Box::new(MateTable {
            cand: [[0u128; 32]; 81],
            sets: [[KingSets::default(); 2]; 81],
        });
        let on = |x: i32| (0..LEN as i32).contains(&x) && SQ81[x as usize] != 0xFF;
        for k81 in 0..81u8 {
            let ksq = IDX81[k81 as usize];
            let near = empty_board_targets(ksq, make_piece(Color::Black, KING));
            // 色cの駒pを置いて玉へ短い利きで届く升
            let hit_from = |p: u8| -> u128 {
                let mv = &MOVES[p as usize];
                let mut s = 0u128;
                for k in 0..mv.n_short as usize {
                    let x = ksq as i32 - mv.short[k] as i32;
                    if on(x) {
                        s |= bit81(x as u8);
                    }
                }
                s
            };
            let adjacent = |dirs: [usize; 4]| -> u128 {
                let mut s = 0u128;
                for d in dirs {
                    let x = ksq as i32 + DELTA[d] as i32;
                    if on(x) {
                        s |= bit81(x as u8);
                    }
                }
                s
            };
            for c in [Color::Black, Color::White] {
                let lance_sq = ksq as i32 - pawn_delta(c);
                let ks = &mut t.sets[k81 as usize][c.index()];
                ks.near = near;
                ks.knight_to = hit_from(make_piece(c, KNIGHT));
                ks.gold_to = hit_from(make_piece(c, GOLD));
                ks.silver_to = hit_from(make_piece(c, SILVER));
                ks.lance_to = if on(lance_sq) {
                    bit81(lance_sq as u8)
                } else {
                    0
                };
                ks.rook_adj = adjacent([UP, DOWN, LEFT, RIGHT]);
                ks.bishop_adj = adjacent([UL, DR, UR, DL]);
            }
            for p in 0..32u8 {
                let pt = piece_type(p);
                if pt < 2 || pt == KING {
                    continue;
                }
                let c = piece_color(p);
                let target = if pt == KNIGHT {
                    t.sets[k81 as usize][c.index()].knight_to
                } else {
                    near
                };
                let mut set = 0u128;
                for f81 in 0..81u8 {
                    if f81 == k81 {
                        continue;
                    }
                    let from = IDX81[f81 as usize];
                    if empty_board_targets(from, p) & target != 0 {
                        set |= bit81(from);
                    }
                }
                t.cand[k81 as usize][p as usize] = set;
                let mv = &MOVES[p as usize];
                for f81 in 0..81u8 {
                    let from = IDX81[f81 as usize];
                    for k in 0..mv.n_short as usize {
                        let x = from as i32 + mv.short[k] as i32;
                        if f81 != k81 && on(x) && target & bit81(x as u8) != 0 {
                            t.sets[k81 as usize][c.index()].cand_short |= bit81(from);
                        }
                    }
                }
            }
        }
        t
    })
}

/// 升の添字の81升のビット。壁なら0。
#[inline]
fn bit81_or_zero(i: u8) -> u128 {
    let s = SQ81[i as usize];
    if s == 0xFF { 0 } else { 1u128 << s }
}

/// 1手詰めの列挙と判定の仕事の量（`mate_1ply_counted` が足す。ADR-0241の第10回）。
#[derive(Clone, Copy, Debug, Default)]
pub struct MateCount {
    /// 呼び出しの回数
    pub calls: u64,
    /// 列挙で回した手番側の駒の数
    pub pieces_seen: u64,
    /// そのうち、候補の表で落ちずに行き先を求めた駒の数
    pub pieces_kept: u64,
    /// 盤上の駒の候補の行き先の数
    pub targets: u64,
    /// 駒打ちの候補の升の数
    pub drop_targets: u64,
    /// 候補の判定（`is_mate_move`）まで届いた回数
    pub judged: u64,
    /// 回す駒を絞るために、近傍の升から飛び利きの元の駒まで歩いた回数
    pub line_walks: u64,
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
            if self.has_any_hand(a.ti) {
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
            if self.has_any_hand(di) {
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
        self.mate_1ply_impl::<false>(&mut MateCount::default())
    }

    /// `mate_1ply` と同じ手を返し、列挙と判定の仕事の量をcへ足す。数え上げのテストだけが使う。
    #[doc(hidden)]
    pub fn mate_1ply_counted(&self, c: &mut MateCount) -> Option<Move> {
        self.mate_1ply_impl::<true>(c)
    }

    #[inline(always)]
    fn mate_1ply_impl<const COUNT: bool>(&self, cnt: &mut MateCount) -> Option<Move> {
        debug_assert!(!self.in_check());
        if COUNT {
            cnt.calls += 1;
        }
        let us = self.side;
        let ui = us.index();
        let ksq = self.king[us.flip().index()];
        let tab = mate_table();
        let k81 = SQ81[ksq as usize] as usize;
        let ks = &tab.sets[k81][ui];
        let near = ks.near;
        let knight_to = ks.knight_to;
        let occ_all = self.occ[0] | self.occ[1];
        let empty = ALL81 & !occ_all;
        // 玉の逃げ場: 玉の隣で、相手（玉の側）の駒がなく、手番側の利きもない升（方向のビット）
        let escapes = self.mate_escapes(us, ksq);
        let cand = &tab.cand[k81];
        let movers = self.mate_movers::<COUNT>(ui, near, ks.cand_short, occ_all, cnt)
            & !bit81(self.king[ui]);
        if let Some(m) =
            self.mate_board::<COUNT>(cand, movers, escapes, ksq, near, knight_to, occ_all, cnt)
        {
            return Some(m);
        }
        self.mate_drops::<COUNT>(ks, escapes, ksq, near, empty, occ_all, cnt)
    }

    /// 相手玉の逃げ場（方向のビット）。1手詰めの列挙の1段目。
    #[inline(always)]
    fn mate_escapes(&self, us: Color, ksq: u8) -> u8 {
        let ui = us.index();
        let mut escapes = 0u8;
        for (d, &delta) in DELTA.iter().enumerate() {
            let t = (ksq as i32 + delta as i32) as usize;
            let v = self.board[t];
            if v != WALL && (v == EMPTY || piece_color(v) == us) && self.count[ui][t] == 0 {
                escapes |= 1 << d;
            }
        }
        escapes
    }

    /// 回す駒の集合（81升）。1手詰めの列挙の2段目。
    #[inline(always)]
    fn mate_movers<const COUNT: bool>(
        &self,
        ui: usize,
        near: u128,
        cand_short: u128,
        occ_all: u128,
        cnt: &mut MateCount,
    ) -> u128 {
        // 回す駒を先に絞る。短い利きで近傍（桂は王手の升）へ届きうる升の駒と、
        // 近傍の升に届いている手番側の飛び利きの元の駒である。ほかの駒は行き先が近傍に無い
        let mut movers = self.occ[ui] & cand_short;
        let mut nb = near;
        while nb != 0 {
            let t81 = nb.trailing_zeros() as usize;
            nb &= nb - 1;
            let t = idx81(t81);
            let mut dirs = self.longdir[ui][t as usize];
            while dirs != 0 {
                let d = dirs.trailing_zeros() as usize;
                dirs &= dirs - 1;
                // 1つ手前の升も近傍の空き升で同じ線が届いていれば、元の駒はそちらで拾う
                let prev = t.wrapping_add(DELTA[opp(d)] as u8);
                if near & bit81_or_zero(prev) != 0
                    && self.board[prev as usize] == EMPTY
                    && self.longdir[ui][prev as usize] & (1 << d) != 0
                {
                    continue;
                }
                if COUNT {
                    cnt.line_walks += 1;
                }
                // 線の元の駒は、近傍の升から逆向きに占有の集合と線の表で引く（盤を歩かない）。
                // 方向ビットが立っているので、線の上に必ず駒がある
                let od = opp(d);
                let hit = ray81(t81, od) & occ_all;
                debug_assert!(hit != 0);
                let first = if INCREASING[od] {
                    hit.trailing_zeros()
                } else {
                    127 - hit.leading_zeros()
                };
                movers |= 1u128 << first;
            }
        }
        movers
    }

    /// 盤上の駒の候補を調べる。1手詰めの列挙の3段目。
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn mate_board<const COUNT: bool>(
        &self,
        cand: &[u128; 32],
        movers: u128,
        escapes: u8,
        ksq: u8,
        near: u128,
        knight_to: u128,
        occ_all: u128,
        cnt: &mut MateCount,
    ) -> Option<Move> {
        let us = self.side;
        let ui = us.index();
        for s in iter81(movers) {
            if COUNT {
                cnt.pieces_seen += 1;
            }
            let p = self.board[s as usize];
            // 空の盤でも玉の近傍（桂は王手の升）へ届かない駒は、利きを歩かずに捨てる
            if cand[p as usize] & bit81(s) == 0 {
                continue;
            }
            if COUNT {
                cnt.pieces_kept += 1;
            }
            let pt = piece_type(p);
            let targets =
                self.near_targets(s, p, ui, occ_all) & if pt == KNIGHT { knight_to } else { near };
            // 移動元に手番側の飛び利きが当たっていれば、動かすと利きが伸びうる。
            // その手は利き数で決めず、玉に取られるかを升の利きで調べ直す（coreの `opens_line`）
            let open = self.longdir[ui][s as usize] != 0;
            for t in iter81(targets) {
                if COUNT {
                    cnt.targets += 1;
                }
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
                    if !(!open && self.escape_left(escapes, ksq, t, p - 8, s, occ_all))
                        && self.judge::<COUNT>(m, cnt)
                    {
                        return Some(m);
                    }
                }
                if !dead_square(us, pt, t) {
                    let m = Move::normal(s, t, false, p);
                    if !(!open && self.escape_left(escapes, ksq, t, p, s, occ_all))
                        && self.judge::<COUNT>(m, cnt)
                    {
                        return Some(m);
                    }
                }
            }
        }
        None
    }

    /// 駒打ちの候補を調べる。1手詰めの列挙の4段目。
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn mate_drops<const COUNT: bool>(
        &self,
        ks: &KingSets,
        escapes: u8,
        ksq: u8,
        near: u128,
        empty: u128,
        occ_all: u128,
        cnt: &mut MateCount,
    ) -> Option<Move> {
        let us = self.side;
        let ui = us.index();
        // 駒打ちの候補の升。coreと同じ駒種の順。配列を組まずに表から直接読む
        for pt in [GOLD, SILVER, KNIGHT, LANCE, ROOK, BISHOP] {
            let targets = match pt {
                GOLD => ks.gold_to,
                SILVER => ks.silver_to,
                KNIGHT => ks.knight_to,
                LANCE => ks.lance_to,
                ROOK => ks.rook_adj,
                _ => ks.bishop_adj,
            };
            if !self.has_hand(ui, pt) {
                continue;
            }
            for t in iter81(targets & empty) {
                if COUNT {
                    cnt.drop_targets += 1;
                }
                if dead_square(us, pt, t) {
                    continue;
                }
                // 打った駒は利き数に入っていないので、守りが0枚なら玉に取られる
                if near & bit81(t) != 0 && self.count[ui][t as usize] == 0 {
                    continue;
                }
                if self.escape_left(escapes, ksq, t, make_piece(us, pt), NO_FROM, occ_all) {
                    continue;
                }
                let m = Move::drop(pt, t, us);
                if self.judge::<COUNT>(m, cnt) {
                    return Some(m);
                }
            }
        }
        None
    }

    /// 升sの駒pの行き先の集合（`targets_of` と同じ値）。
    /// 飛び利きは盤を歩かず、81升の線の集合と占有の積から最初の駒を引く。
    #[inline]
    fn near_targets(&self, s: u8, p: u8, ui: usize, occ_all: u128) -> u128 {
        let m = &MOVES[(p & 31) as usize];
        let s81 = SQ81[s as usize] as usize;
        let mut out = short81(p, s81);
        let mut dirs = m.long;
        while dirs != 0 {
            let d = dirs.trailing_zeros() as usize;
            dirs &= dirs - 1;
            out |= ray_to_blocker(s81, d, occ_all);
        }
        let out = out & !self.occ[ui];
        debug_assert_eq!(out, self.targets_of(s, p, ui), "near_targets");
        out
    }

    /// 候補の判定。数えるときは判定の回数も足す。
    #[inline(always)]
    fn judge<const COUNT: bool>(&self, m: Move, cnt: &mut MateCount) -> bool {
        if COUNT {
            cnt.judged += 1;
        }
        self.is_mate_move(m)
    }

    /// 升tへ置いた駒npの利きで埋まらない逃げ場が残るか（移動元に飛び利きが当たっていない手に限る）。
    ///
    /// 逃げ場は指す前に手番側の利きがない升なので、指した後に利きが増えるのはnpの利きだけである。
    /// npの飛び利きは、玉と移動元fromを空きとみなして通す（玉が線に沿って逃げても利きは届く）。
    /// 駒打ちはfromに `NO_FROM` を渡す。
    #[inline]
    fn escape_left(&self, escapes: u8, ksq: u8, t: u8, np: u8, from: u8, occ_all: u128) -> bool {
        let mv = &MOVES[(np & 31) as usize];
        let t81 = SQ81[t as usize] as usize;
        let k81 = SQ81[ksq as usize] as usize;
        // 線を通すときに空きとみなす升（玉と移動元）を抜いた占有
        let from_bit = if from == NO_FROM {
            0
        } else {
            bit81_or_zero(from)
        };
        let occ = occ_all & !(1u128 << k81) & !from_bit;
        let mut dirs = escapes;
        while dirs != 0 {
            let d = dirs.trailing_zeros() as usize;
            dirs &= dirs - 1;
            let e = (ksq as i32 + DELTA[d] as i32) as u8;
            if e == t || short_hit(np, e as i32 - t as i32) {
                continue;
            }
            let e81 = SQ81[e as usize] as usize;
            let dd = dir81(t81, e81) as usize;
            // 置いた駒の飛び利きがeへ届くか。tからeの間（両端を含まない）が空いていれば届く
            let covered = dd != NO_DIR as usize
                && mv.long & (1 << dd) != 0
                && RAY81[t81][dd] & !RAY81[e81][dd] & !(1u128 << e81) & occ == 0;
            if !covered {
                return true;
            }
        }
        false
    }
}
