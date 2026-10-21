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

/// 生成した手の受け手（ADR-0241の第15回）。
///
/// 升は81升の通し番号、駒は移動後の駒（先後込み）で渡す。探索（engine）はcoreの手の列へ
/// 直接書く受け手を作り、配列盤の手の列への詰め替えと変換を省く。
pub trait MoveSink {
    /// 盤上の手。
    fn push_normal(&mut self, from81: u8, to81: u8, promote: bool, piece_after: u8);
    /// 駒打ち。ptは打つ駒種、pieceは打った駒。
    fn push_drop(&mut self, pt: u8, to81: u8, piece: u8);
}

impl MoveSink for MoveList {
    #[inline(always)]
    fn push_normal(&mut self, from81: u8, to81: u8, promote: bool, piece_after: u8) {
        self.push(Move::normal(
            IDX81[from81 as usize],
            IDX81[to81 as usize],
            promote,
            piece_after,
        ));
    }

    #[inline(always)]
    fn push_drop(&mut self, pt: u8, to81: u8, piece: u8) {
        self.push(Move::drop(pt, IDX81[to81 as usize], piece_color(piece)));
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
    /// 手番側の玉の81升の通し番号。
    k81: u8,
    /// ピンされた駒の升の集合。ピンの無い駒は線を引かずに済ませる
    pinned: u128,
}

impl Ctx {
    /// 81升の升s81の駒が動ける升の集合。
    ///
    /// ピンされた駒は、玉からその駒の方向へ盤の端までの線の上だけを動ける。線は表から引くので、
    /// ピンごとの線を配列に持たない（ADR-0241の第16回）。
    #[inline(always)]
    fn pin_mask(&self, s81: u8) -> u128 {
        if self.pinned & (1u128 << s81) == 0 {
            return ALL81;
        }
        let k81 = self.k81 as usize;
        ray81(k81, dir81(k81, s81 as usize) as usize)
    }
}

use crate::position::CHECK_OFFSETS;

impl Position {
    /// 手番cの玉から方向dへ歩いて最初の駒の升（無ければNone）。
    #[inline]
    pub(crate) fn first_piece(&self, k: u8, d: usize) -> Option<u8> {
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
    pub(crate) fn is_pinned(&self, us: Color, x: u8) -> bool {
        let k = self.king[us.index()];
        let Some(d) = dir_between(k, x) else {
            return false;
        };
        self.first_piece(k, d) == Some(x)
            && self.longdir[us.flip().index()][x as usize] & (1 << opp(d)) != 0
    }

    /// 玉から8方向の最初の駒を、占有の集合と線の表から引き、ピンされた自駒を集める。
    /// 方向dで最初に当たった自駒の升に、相手の飛び利きが方向opp(d)（玉へ向かう向き）で
    /// 届いていればピンである。
    fn collect_pins(&self, ctx: &mut Ctx) {
        let us = ctx.us;
        let ui = us.index();
        let them = us.flip().index();
        let k81 = SQ81[self.king[ui] as usize] as usize;
        let own = self.occ[ui];
        let occ_all = own | self.occ[them];
        // 線の上に相手の飛び駒が1枚も無い向きは、最初の駒を探さずに飛ばす
        let ps = &self.psets[them];
        let orth = ps[ROOK as usize] | ps[DRAGON as usize];
        let diag = ps[BISHOP as usize] | ps[HORSE as usize];
        let lance = ps[LANCE as usize];
        // 相手の香が玉へ向かって走る線は、玉から相手の陣の側（先手の玉なら上）へ伸びる
        let lance_dir = if them == Color::White.index() {
            UP
        } else {
            DOWN
        };
        for (d, &inc) in INCREASING.iter().enumerate() {
            let ray = ray81(k81, d);
            let sliders = if d >= UL {
                diag
            } else if d == lance_dir {
                orth | lance
            } else {
                orth
            };
            if ray & sliders == 0 {
                continue;
            }
            let hit = ray & occ_all;
            let first = if inc {
                hit.trailing_zeros()
            } else {
                127 - hit.leading_zeros()
            } as u8;
            if own & (1u128 << first) != 0
                && self.longdir[them][idx81(first as usize) as usize] & (1 << opp(d)) != 0
            {
                ctx.pinned |= 1u128 << first;
            }
        }
    }

    /// 1つの王手駒の升と、合駒の升の集合。王手が1つのときに呼ぶ。
    pub(crate) fn checker(&self, us: Color) -> (u8, u128) {
        let k = self.king[us.index()];
        let them = us.flip();
        let mut between = 0u128;
        // 飛び利きの王手。玉へ向かう方向dの飛び利きを逆にたどって駒を探す
        let dirs = self.longdir[them.index()][k as usize];
        if dirs != 0 {
            // 玉から逆向きの線で最初の駒が王手駒。占有の集合と線の表で引く（盤を歩かない）
            let d = dirs.trailing_zeros() as usize;
            let od = opp(d);
            let k81 = SQ81[k as usize] as usize;
            let seg = ray_to_blocker(k81, od, self.occ[0] | self.occ[1]);
            let first = if INCREASING[od] {
                127 - seg.leading_zeros()
            } else {
                seg.trailing_zeros()
            } as usize;
            between = seg & !(1u128 << first);
            return (IDX81[first], between);
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
    pub(crate) fn targets_of(&self, s: u8, p: u8, ui: usize) -> u128 {
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
    pub fn generate_legal<L: MoveSink>(&self, all: bool, list: &mut L) {
        if self.in_check() {
            self.generate(GenType::Evasions, all, list);
        } else {
            self.generate(GenType::NonEvasions, all, list);
        }
    }

    /// 分類ごとの合法手を生成する。Evasionsは王手中だけ、ほかは王手でないときだけ呼ぶ。
    pub fn generate<L: MoveSink>(&self, gt: GenType, all: bool, list: &mut L) {
        let us = self.side;
        let ui = us.index();
        let them = us.flip().index();
        let k = self.king[ui];
        let n_checks = self.count[them][k as usize];
        debug_assert_eq!(gt == GenType::Evasions, n_checks != 0);
        let mut ctx = Ctx {
            us,
            evasion: ALL81,
            k81: SQ81[k as usize],
            pinned: 0,
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
                #[cfg(mb_order_type)]
                self.gen_captures_by_target(&ctx, enemy, all, list);
                #[cfg(not(mb_order_type))]
                {
                    self.gen_board_captures(&ctx, enemy, all, list);
                    self.gen_king(&ctx, enemy, list);
                }
                // 空き升への歩の成り。行き先の小さい順に回すと、移動元も同じ順に並ぶ
                let fwd81 = pawn_delta81(us);
                let pro = make_piece(us, PRO_PAWN);
                let to = self.pawn_pushes(&ctx, empty) & zone81(ui);
                for_each_bit81(to, |t| {
                    list.push_normal((t as i32 - fwd81) as u8, t, true, pro);
                });
            }
            GenType::Quiets => {
                self.collect_pins(&mut ctx);
                self.gen_board(&ctx, empty, all, true, list);
                // 歩の静かな手。成りはCaptures側で、不成だけをここで積む。
                // 敵陣の外へ進む手と、allなら敵陣の2・3段目への不成
                let fwd81 = pawn_delta81(us);
                let pawn = make_piece(us, PAWN);
                let zone = zone81(ui);
                let allowed = if all {
                    !zone | (zone & !rank_mask(if us == Color::Black { 0 } else { 8 }))
                } else {
                    !zone
                };
                let to = self.pawn_pushes(&ctx, empty) & allowed;
                for_each_bit81(to, |t| {
                    list.push_normal((t as i32 - fwd81) as u8, t, false, pawn);
                });
                self.gen_king(&ctx, empty, list);
                self.gen_drops(&ctx, empty, list);
            }
        }
    }

    /// 手番側の歩が1升進める空き升の集合（ピンを守る）。行き先の升で返す。
    ///
    /// ピンされていない歩は、歩の集合をずらしてまとめて作る。歩は行き所の無い段にいないので、
    /// ずらしても筋をまたがない（ADR-0241の第16回）。
    #[inline(always)]
    fn pawn_pushes(&self, ctx: &Ctx, empty: u128) -> u128 {
        let ui = ctx.us.index();
        let pawns = self.psets[ui][PAWN as usize];
        let shift = |b: u128| {
            if ctx.us == Color::Black {
                b >> 1
            } else {
                (b << 1) & ALL81
            }
        };
        let mut to = shift(pawns & !ctx.pinned);
        let mut pinned = pawns & ctx.pinned;
        while pinned != 0 {
            let s = pinned.trailing_zeros() as u8;
            pinned &= pinned - 1;
            to |= shift(1u128 << s) & ctx.pin_mask(s);
        }
        to & empty
    }

    fn gen_king<L: MoveSink>(&self, ctx: &Ctx, target: u128, list: &mut L) {
        let ui = ctx.us.index();
        let them = ctx.us.flip().index();
        let k = self.king[ui];
        let k81 = SQ81[k as usize];
        let xray = self.longdir[them][k as usize];
        let kp = make_piece(ctx.us, KING);
        let near = short81(kp, k81 as usize);
        let mut cand = near & !self.occ[ui] & target;
        // 玉の升に届く飛び利きの延長（玉が線に沿って逃げる升）は、利き数に入っていない。
        // 延長の升は、玉からその向きの線と玉の近傍の積の1升である
        let mut dirs = xray;
        while dirs != 0 {
            let d = dirs.trailing_zeros() as usize;
            dirs &= dirs - 1;
            cand &= !(ray81(k81 as usize, d) & near);
        }
        for_each_bit81(cand, |t| {
            if self.count[them][idx81(t as usize) as usize] == 0 {
                list.push_normal(k81, t, false, kp);
            }
        });
    }

    /// 玉以外の盤上の駒の手（並びの変わる版、`HIMAWARI_MB_ORDER=type`）。skip_pawnsなら歩を飛ばす。
    ///
    /// 駒種ごとの集合から回し、駒種をconstの引数にする。駒を盤から引く手間と駒種での分岐が消え、
    /// 短い利き・飛び利き・成りの判定が駒種ごとに畳まれる。手の集合はcoreと同じで、並びは
    /// 駒種の順（歩・香・桂・銀・角・飛・金・成駒4種・馬・竜）の中で移動元の升の順になる（ADR-0241の第16回）。
    #[cfg(mb_order_type)]
    fn gen_board<L: MoveSink>(
        &self,
        ctx: &Ctx,
        target: u128,
        all: bool,
        skip_pawns: bool,
        list: &mut L,
    ) {
        let ui = ctx.us.index();
        let own = self.occ[ui];
        let occ_all = own | self.occ[ctx.us.flip().index()];
        let target = target & ctx.evasion & !own;
        let ps = &self.psets[ui];
        if !skip_pawns {
            self.gen_type::<PAWN, L>(ctx, ps[PAWN as usize], target, occ_all, all, list);
        }
        self.gen_type::<LANCE, L>(ctx, ps[LANCE as usize], target, occ_all, all, list);
        self.gen_type::<KNIGHT, L>(ctx, ps[KNIGHT as usize], target, occ_all, all, list);
        self.gen_type::<SILVER, L>(ctx, ps[SILVER as usize], target, occ_all, all, list);
        self.gen_type::<BISHOP, L>(ctx, ps[BISHOP as usize], target, occ_all, all, list);
        self.gen_type::<ROOK, L>(ctx, ps[ROOK as usize], target, occ_all, all, list);
        self.gen_type::<GOLD, L>(ctx, ps[GOLD as usize], target, occ_all, all, list);
        self.gen_type::<PRO_PAWN, L>(ctx, ps[PRO_PAWN as usize], target, occ_all, all, list);
        self.gen_type::<3, L>(ctx, ps[3], target, occ_all, all, list);
        self.gen_type::<4, L>(ctx, ps[4], target, occ_all, all, list);
        self.gen_type::<5, L>(ctx, ps[5], target, occ_all, all, list);
        self.gen_type::<HORSE, L>(ctx, ps[HORSE as usize], target, occ_all, all, list);
        self.gen_type::<DRAGON, L>(ctx, ps[DRAGON as usize], target, occ_all, all, list);
    }

    /// 駒取りを行き先の側から引く（並びの変わる版）。手番側が利いている相手の駒の升を小さい順に回し、
    /// その升へ利いている手番側の駒から手を作る（ADR-0241の第16回）。
    ///
    /// 飛び利きの元の駒は、行き先の升の方向ビットから線の表で引く。短い利きの元の駒は、利き数から
    /// 飛び利きの本数を引いて残りがあるときだけ、近傍と桂の升を見る。玉で取れるのは、相手の利きが
    /// 無く、玉の升に届く相手の飛び利きの延長でもない升だけである。
    #[cfg(mb_order_type)]
    fn gen_captures_by_target<L: MoveSink>(&self, ctx: &Ctx, enemy: u128, all: bool, list: &mut L) {
        let us = ctx.us;
        let ui = us.index();
        let them = us.flip().index();
        let occ_all = self.occ[ui] | enemy;
        let k = self.king[ui];
        let k81 = SQ81[k as usize];
        let kp = make_piece(us, KING);
        let xray = self.longdir[them][k as usize];
        // 手番側が利いている相手の駒の升
        let mut hit = 0u128;
        for_each_bit81(enemy, |t| {
            hit |= u128::from(self.count[ui][idx81(t as usize) as usize] != 0) << t;
        });
        for_each_bit81(hit, |t| {
            let ti = idx81(t as usize);
            let dirs = self.longdir[ui][ti as usize];
            let mut from = 0u128;
            let mut ds = dirs;
            while ds != 0 {
                let d = ds.trailing_zeros() as usize;
                ds &= ds - 1;
                let od = opp(d);
                let h = ray81(t as usize, od) & occ_all;
                debug_assert!(h != 0);
                let first = if INCREASING[od] {
                    h.trailing_zeros()
                } else {
                    127 - h.leading_zeros()
                };
                from |= 1u128 << first;
            }
            if u32::from(self.count[ui][ti as usize]) > dirs.count_ones() {
                for o in CHECK_OFFSETS {
                    let x = ti.wrapping_sub(o as u8);
                    let v = self.board[x as usize];
                    if v != EMPTY
                        && v != WALL
                        && piece_color(v) == us
                        && short81(v, SQ81[x as usize] as usize) & (1u128 << t) != 0
                    {
                        from |= bit81(x);
                    }
                }
            }
            for_each_bit81(from, |s| {
                if s == k81 {
                    let d = dir81(k81 as usize, t as usize);
                    if self.count[them][ti as usize] == 0 && xray & (1 << d) == 0 {
                        list.push_normal(k81, t, false, kp);
                    }
                    return;
                }
                if ctx.pin_mask(s) & (1u128 << t) == 0 {
                    return;
                }
                let pt = piece_type(self.board[idx81(s as usize) as usize]);
                push_variants(us, pt, s, t, all, list);
            });
        });
    }

    /// 駒種PTの駒の手を、移動元の升の順に積む（並びの変わる版）。
    #[cfg(mb_order_type)]
    #[inline(always)]
    fn gen_type<const PT: u8, L: MoveSink>(
        &self,
        ctx: &Ctx,
        set: u128,
        target: u128,
        occ_all: u128,
        all: bool,
        list: &mut L,
    ) {
        if set == 0 {
            return;
        }
        let us = ctx.us;
        let p = make_piece(us, PT);
        let long = MOVES[(p & 31) as usize].long;
        for_each_bit81(set, |s| {
            let mut to = short81(p, s as usize);
            let mut dirs = long;
            while dirs != 0 {
                let d = dirs.trailing_zeros() as usize;
                dirs &= dirs - 1;
                to |= ray_to_blocker(s as usize, d, occ_all);
            }
            let to = to & target & ctx.pin_mask(s);
            for_each_bit81(to, |t| push_variants(us, PT, s, t, all, list));
        });
    }

    /// 玉以外の盤上の駒の手。skip_pawnsなら歩を飛ばす（Quietsは歩を別に積む）。
    ///
    /// 81升の番号のまま回す。短い利きは表、飛び利きは占有の集合と線の表から引くので、盤を歩かない。
    #[cfg(not(mb_order_type))]
    fn gen_board<L: MoveSink>(
        &self,
        ctx: &Ctx,
        target: u128,
        all: bool,
        skip_pawns: bool,
        list: &mut L,
    ) {
        let us = ctx.us;
        let ui = us.index();
        let own = self.occ[ui];
        let occ_all = own | self.occ[us.flip().index()];
        let movers = own & !bit81(self.king[ui]);
        let target = target & ctx.evasion & !own;
        for_each_bit81(movers, |s| {
            let p = self.board[idx81(s as usize) as usize];
            let pt = piece_type(p);
            if skip_pawns && pt == PAWN {
                return;
            }
            let m = &MOVES[(p & 31) as usize];
            let mut to = short81(p, s as usize);
            let mut dirs = m.long;
            while dirs != 0 {
                let d = dirs.trailing_zeros() as usize;
                dirs &= dirs - 1;
                to |= ray_to_blocker(s as usize, d, occ_all);
            }
            let to = to & target;
            if to == 0 {
                return;
            }
            push_targets(us, pt, s, to & ctx.pin_mask(s), all, list);
        });
    }

    /// 玉以外の盤上の駒の駒取り（`gen_board` に相手の駒の集合を渡したときと同じ手を同じ順に積む）。
    ///
    /// 行き先が相手の駒に限られるので、飛び利きは線の最初の駒の升だけを見る。線の升の集合を作らない。
    #[cfg(not(mb_order_type))]
    fn gen_board_captures<L: MoveSink>(&self, ctx: &Ctx, enemy: u128, all: bool, list: &mut L) {
        let us = ctx.us;
        let ui = us.index();
        let own = self.occ[ui];
        let occ_all = own | enemy;
        let movers = own & !bit81(self.king[ui]);
        for_each_bit81(movers, |s| {
            let p = self.board[idx81(s as usize) as usize];
            let pt = piece_type(p);
            let m = &MOVES[(p & 31) as usize];
            let mut to = short81(p, s as usize);
            let mut dirs = m.long;
            while dirs != 0 {
                let d = dirs.trailing_zeros() as usize;
                dirs &= dirs - 1;
                let hit = ray81(s as usize, d) & occ_all;
                if hit != 0 {
                    let first = if INCREASING[d] {
                        hit.trailing_zeros()
                    } else {
                        127 - hit.leading_zeros()
                    };
                    to |= 1u128 << first;
                }
            }
            let to = to & enemy;
            if to == 0 {
                return;
            }
            push_targets(us, pt, s, to & ctx.pin_mask(s), all, list);
        });
    }

    fn gen_drops<L: MoveSink>(&self, ctx: &Ctx, target: u128, list: &mut L) {
        let us = ctx.us;
        let ui = us.index();
        if !self.has_any_hand(ui) {
            return;
        }
        let r1 = rank_mask(if us == Color::Black { 0 } else { 8 });
        let r2 = rank_mask(if us == Color::Black { 1 } else { 7 });
        for pt in DROP_KINDS {
            if !self.has_hand(ui, pt) {
                continue;
            }
            let mut mask = match pt {
                PAWN => {
                    // 二歩の筋。筋fのビットを81升の番号9fへ散らし、0x1FFを掛けて筋全体へ広げる。
                    // 筋ごとの9bitの塊は重ならないので繰り上がりは出ない。分岐しない
                    let files = u128::from(self.pawn_files[ui]);
                    let mut spread = 0u128;
                    for f in 0..9 {
                        spread |= ((files >> f) & 1) << (9 * f);
                    }
                    let nifu = spread * 0x1FF;
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
            let pc = make_piece(us, pt);
            for_each_bit81(mask, |t| list.push_drop(pt, t, pc));
        }
    }
}

/// 歩が進む向きの差分。
#[inline]
pub(crate) const fn pawn_delta(us: Color) -> i32 {
    match us {
        Color::Black => -1,
        Color::White => 1,
    }
}

/// 手番uiから見た敵陣（1〜3段目）の升の集合。
#[inline(always)]
fn zone81(ui: usize) -> u128 {
    const ZONE: [u128; 2] = [
        rank_mask(0) | rank_mask(1) | rank_mask(2),
        rank_mask(6) | rank_mask(7) | rank_mask(8),
    ];
    ZONE[ui & 1]
}

/// 歩が進む向きの、81升の通し番号の差分。
#[inline]
const fn pawn_delta81(us: Color) -> i32 {
    match us {
        Color::Black => -1,
        Color::White => 1,
    }
}

/// 升from81の駒種ptの駒の、行き先の集合toへの手を、行き先の小さい順に積む。各升の成り・不成は
/// `push_variants` と同じで、同じ順に並ぶ。
///
/// 成りの升と不成の升を先に集合で決め、行き先ごとに段の表を引かない（ADR-0241の第17回）。
/// 成れない駒と、行き先がどれも成れない升の駒は、不成の手だけを積む。
#[cfg(not(mb_order_type))]
#[inline(always)]
fn push_targets<L: MoveSink>(us: Color, pt: u8, from81: u8, to: u128, all: bool, list: &mut L) {
    let pc = make_piece(us, pt);
    if !can_promote(pt) {
        for_each_bit81(to, |t| list.push_normal(from81, t, false, pc));
        return;
    }
    let ui = us.index();
    let zone = zone81(ui);
    let pro = if zone & (1u128 << from81) != 0 {
        to
    } else {
        to & zone
    };
    if pro == 0 {
        // 敵陣の外の行き先だけ。不成で行き所の無くなる升は敵陣の中なので、ここには来ない
        for_each_bit81(to, |t| list.push_normal(from81, t, false, pc));
        return;
    }
    // 成れる升のうち、不成も積む升。`push_variants` の成りの枝の条件を段の集合にしたもの
    let r1 = rank_mask(if us == Color::Black { 0 } else { 8 });
    let r2 = rank_mask(if us == Color::Black { 1 } else { 7 });
    let keep = match pt {
        PAWN => {
            if all {
                !r1
            } else {
                0
            }
        }
        LANCE => {
            if all {
                !r1
            } else {
                !(r1 | r2)
            }
        }
        KNIGHT => !(r1 | r2),
        SILVER => ALL81,
        _ => {
            if all {
                ALL81
            } else {
                0
            }
        }
    };
    let non = (to & !pro) | (pro & keep);
    let ppc = make_piece(us, pt - 8);
    for_each_bit81(to, |t| {
        let b = 1u128 << t;
        if pro & b != 0 {
            list.push_normal(from81, t, true, ppc);
        }
        if non & b != 0 {
            list.push_normal(from81, t, false, pc);
        }
    });
}

/// 盤上の駒の手を、成り・不成を分けて積む。規約はcoreの `push_variants` と同じ。升は81升の通し番号。
#[inline(always)]
#[cfg(mb_order_type)]
fn push_variants<L: MoveSink>(us: Color, pt: u8, from81: u8, to81: u8, all: bool, list: &mut L) {
    let ui = us.index();
    let rel = rel_rank81(ui, to81 as usize);
    if can_promote(pt) && (rel_rank81(ui, from81 as usize) <= 2 || rel <= 2) {
        list.push_normal(from81, to81, true, make_piece(us, pt - 8));
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
            list.push_normal(from81, to81, false, make_piece(us, pt));
        }
    } else {
        let ok = match pt {
            PAWN | LANCE => rel >= 1,
            KNIGHT => rel >= 2,
            _ => true,
        };
        if ok {
            list.push_normal(from81, to81, false, make_piece(us, pt));
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
