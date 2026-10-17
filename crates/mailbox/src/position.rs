//! 配列盤の局面。駒番号・手番別の利き数・飛び利きの方向ビットを持つ。
//!
//! 利きの差分更新は、盤上の駒の移動をまとめて扱う `transfer` と、駒打ちの `put`・`lift` で行う。
//! 利きは盤面だけで決まる関数なので、途中の各段が整合していれば結果は全計算と一致する。
//!
//! `transfer` は升の空き具合の変化を順に当てる。
//!
//! 1. 行き先が空きなら、行き先を通る他の駒の飛び利きを切る。駒取りなら取られる駒の利きを引く
//! 2. 動く駒の利きを引く
//! 3. 盤を書き換える
//! 4. 移動元が空くなら、移動元で止まっていた飛び利きを延ばす（行き先で止まる）
//! 5. 動いた駒の利きを足す
//!
//! 切るのを盤の書き換えの前に、延ばすのを後に置くので、移動元と行き先を両方通る
//! 飛び利きを、行き先の先まで延ばしてから切り戻す無駄が無い。
//!
//! 飛び駒が自分の飛び利きの線に沿って動くとき（滑り）は、その線の利きを全部引き直さない。
//! 移動元から行き先までの升だけを、前向きの線から後ろ向きの線へ付け替える。
//!
//! 戻しは既定で差分（逆向きの `transfer`）である。フィーチャ `copy-undo` を有効にすると、
//! 進めるたびに利きの表を積み、戻しは表を書き戻して盤だけを戻す（ADR-0241の段階4で比べる）。

use crate::board::*;
use crate::moves::Move;
use crate::zobrist::{self, HAND_PAWN, HAND_SHIFT, PIECE_VALUE, PSQ, is_minor_piece};

/// 王手駒を探す短い利きの差分（玉の8近傍と、両者の桂の位置）。
pub(crate) const CHECK_OFFSETS: [i8; 12] = [
    DELTA[0],
    DELTA[1],
    DELTA[2],
    DELTA[3],
    DELTA[4],
    DELTA[5],
    DELTA[6],
    DELTA[7],
    KNIGHT_STEPS[0],
    KNIGHT_STEPS[1],
    -KNIGHT_STEPS[0],
    -KNIGHT_STEPS[1],
];

/// 駒番号の無い升の値。盤上と手駒の駒は40枚。
pub const NO_ID: u8 = 0xFF;

/// 81升の通し番号の「升なし」。coreの `Square::NONE` と同じ値。
pub const SQ_NONE: u8 = 81;

/// NNUEの差分の材料（coreの `DirtyPiece` と同じ中身。ADR-0014）。
/// 駒は駒の符号、升は81升の通し番号で持つ。1手で変わる駒は最大2。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DirtyPiece {
    pub count: u8,
    pub king_moved: bool,
    pub piece_old: [u8; 2],
    pub piece_new: [u8; 2],
    pub from: [u8; 2],
    pub to: [u8; 2],
}

impl Default for DirtyPiece {
    fn default() -> Self {
        DirtyPiece {
            count: 0,
            king_moved: false,
            piece_old: [EMPTY; 2],
            piece_new: [EMPTY; 2],
            from: [SQ_NONE; 2],
            to: [SQ_NONE; 2],
        }
    }
}

/// 1手ごとに積む状態。coreの `StateInfo` と同じ値を持つ（遅延で作るピン・利きの地図は除く）。
#[derive(Copy, Clone, Debug, Default)]
pub struct StateInfo {
    pub captured: u8,
    cap_id: u8,
    pub board_key: u64,
    pub hand_key: u64,
    pub pawn_key: u64,
    pub non_pawn_key: [u64; 2],
    pub minor_piece_key: u64,
    /// 手番側の玉に王手をかけている駒の升の集合（81升の通し番号のビット）。
    pub checkers: u128,
    pub continuous_check: [u16; 2],
    pub plies_from_null: u16,
    pub material: i32,
    pub dirty: DirtyPiece,
}

/// 複写で戻すときに積む利きの表。
#[cfg(feature = "copy-undo")]
#[derive(Clone)]
struct EffectSnapshot {
    count: [[u8; LEN]; 2],
    longdir: [[u8; LEN]; 2],
}

/// 盤と利きの表を先頭に並べ、64バイト境界にそろえる（`stride16` では各表が64バイトの倍数になる）。
#[derive(Clone)]
#[repr(C, align(64))]
pub struct Position {
    /// 升の中身（駒の符号・EMPTY・WALL）。
    pub(crate) board: [u8; LEN],
    /// 手番別の利き数。
    pub(crate) count: [[u8; LEN]; 2],
    /// 手番別の飛び利きの方向ビット。bit dは「方向dへ進む飛び利きがこの升に届く」。
    pub(crate) longdir: [[u8; LEN]; 2],
    /// 升にいる駒の番号。空き升と壁はNO_ID。
    pub(crate) id_on: [u8; LEN],
    /// 駒番号ごとの升（手駒なら0）。
    pub(crate) sq_of: [u8; 40],
    /// 盤上の駒番号の集合（持ち主別）。
    pub(crate) on_board: [u64; 2],
    /// 手駒の駒番号の集合（持ち主別、駒種別。添字はhand_index）。
    pub(crate) hand_ids: [[u64; 8]; 2],
    /// 持ち主別の駒のいる升の集合（81升の通し番号のビット）。
    /// 空き升の列挙と、生成の順をcoreに揃えるために持つ。利きの計算には使わない。
    pub(crate) occ: [u128; 2],
    pub(crate) king: [u8; 2],
    /// 生の歩がいる筋のビット（二歩の判定）。
    pub(crate) pawn_files: [u16; 2],
    pub(crate) side: Color,
    ply: u32,
    /// 手駒をcoreの `Hand` と同じ詰め方で持った値（hand_keyに使う）。
    hand_raw: [u32; 2],
    states: Vec<StateInfo>,
    #[cfg(feature = "copy-undo")]
    snapshots: Vec<EffectSnapshot>,
}

#[derive(Debug)]
pub struct SfenError(pub &'static str);

impl Position {
    fn empty() -> Position {
        let mut board = [WALL; LEN];
        for &i in IDX81.iter() {
            board[i as usize] = EMPTY;
        }
        Position {
            board,
            id_on: [NO_ID; LEN],
            sq_of: [0; 40],
            on_board: [0; 2],
            hand_ids: [[0; 8]; 2],
            occ: [0; 2],
            count: [[0; LEN]; 2],
            longdir: [[0; LEN]; 2],
            king: [0; 2],
            pawn_files: [0; 2],
            side: Color::Black,
            ply: 1,
            hand_raw: [0; 2],
            states: Vec::with_capacity(512),
            #[cfg(feature = "copy-undo")]
            snapshots: Vec::with_capacity(512),
        }
    }

    #[inline]
    pub fn side_to_move(&self) -> Color {
        self.side
    }

    #[inline]
    pub fn piece_on(&self, i: u8) -> u8 {
        self.board[i as usize]
    }

    #[inline]
    pub fn effect_count(&self, c: Color, i: u8) -> u8 {
        self.count[c.index()][i as usize]
    }

    #[inline]
    pub fn long_dirs(&self, c: Color, i: u8) -> u8 {
        self.longdir[c.index()][i as usize]
    }

    #[inline]
    pub fn king_square(&self, c: Color) -> u8 {
        self.king[c.index()]
    }

    #[inline]
    pub fn hand_count(&self, c: Color, pt: u8) -> u32 {
        self.hand_ids[c.index()][hand_index(pt)].count_ones()
    }

    #[inline]
    pub fn state(&self) -> &StateInfo {
        self.states.last().expect("state stack is never empty")
    }

    /// 置換表用の合成キー（coreと同じ定義）。
    #[inline]
    pub fn key(&self) -> u64 {
        let st = self.state();
        st.board_key ^ st.hand_key.wrapping_mul(zobrist::HAND_MIX)
    }

    #[inline]
    pub fn checkers(&self) -> u128 {
        self.state().checkers
    }

    #[inline]
    pub fn in_check(&self) -> bool {
        let us = self.side;
        self.count[us.flip().index()][self.king[us.index()] as usize] != 0
    }

    // --- 利きの差分更新 ---

    /// 駒pを升sに置いたときの利きを足す（ADD）か引く。skipの方向の飛び利きは触らない。
    #[inline]
    fn piece_effect<const ADD: bool>(&mut self, p: u8, s: u8, skip: u8) {
        let c = piece_color(p).index();
        let m = &MOVES[p as usize];
        for k in 0..m.n_short as usize {
            // 短い利きは壁の升にも書く（分岐を省く）。壁の値は読まない
            let t = (s as i32 + m.short[k] as i32) as usize;
            if ADD {
                self.count[c][t] += 1;
            } else {
                self.count[c][t] -= 1;
            }
        }
        let mut dirs = m.long & !skip;
        while dirs != 0 {
            let d = dirs.trailing_zeros() as usize;
            dirs &= dirs - 1;
            self.ray::<ADD>(c, s, d);
        }
    }

    /// 升sの先、方向dへ伸びる色cの飛び利きを足す（ADD）か引く。最初の駒の升まで含める。
    #[inline]
    fn ray<const ADD: bool>(&mut self, c: usize, s: u8, d: usize) {
        let delta = DELTA[d] as i32;
        let b = 1u8 << d;
        let mut t = (s as i32 + delta) as usize;
        loop {
            let v = self.board[t];
            if v == WALL {
                break;
            }
            if ADD {
                self.count[c][t] += 1;
                self.longdir[c][t] |= b;
            } else {
                self.count[c][t] -= 1;
                self.longdir[c][t] &= !b;
            }
            if v != EMPTY {
                break;
            }
            t = (t as i32 + delta) as usize;
        }
    }

    /// 升sに届く飛び利きを、sの先で切る（ADD=false）か延ばす（ADD=true）。
    /// 色exclの方向ビットexcl_bitsは触らない（動く駒自身の線）。
    #[inline]
    fn through<const ADD: bool>(&mut self, s: u8, excl: usize, excl_bits: u8) {
        for c in 0..2 {
            let mut dirs = self.longdir[c][s as usize];
            if c == excl {
                dirs &= !excl_bits;
            }
            while dirs != 0 {
                let d = dirs.trailing_zeros() as usize;
                dirs &= dirs - 1;
                self.ray::<ADD>(c, s, d);
            }
        }
    }

    /// 升aの次から升bまで（両端のうちbを含む）、方向dへ進む色cの飛び利きを足すか引く。
    /// 間の升が空きで、bがaから方向dの線上にあることが前提。
    #[inline]
    fn segment<const ADD: bool>(&mut self, c: usize, a: u8, b: u8, d: usize) {
        let delta = DELTA[d] as i32;
        let bit = 1u8 << d;
        let mut t = a as i32;
        loop {
            t += delta;
            let i = t as usize;
            if ADD {
                self.count[c][i] += 1;
                self.longdir[c][i] |= bit;
            } else {
                self.count[c][i] -= 1;
                self.longdir[c][i] &= !bit;
            }
            if i == b as usize {
                break;
            }
        }
    }

    #[inline]
    fn set_square(&mut self, s: u8, p: u8) {
        let old = self.board[s as usize];
        if old != EMPTY {
            self.occ[piece_color(old).index()] &= !bit81(s);
        }
        if p != EMPTY {
            self.occ[piece_color(p).index()] |= bit81(s);
        }
        self.board[s as usize] = p;
    }

    /// 升srcの駒を升dstへ動かし、dstに駒npを置き、srcにleaveを残す。
    /// dstは空きか取られる駒、leaveは空きか戻ってくる駒（駒取りの戻し）。
    #[inline]
    fn transfer(&mut self, src: u8, dst: u8, np: u8, leave: u8) {
        let p = self.board[src as usize];
        let c = piece_color(p).index();
        let x = self.board[dst as usize];
        let lp = MOVES[p as usize].long;
        let along = if lp != 0 {
            dir_between(src, dst).filter(|&d| lp & (1 << d) != 0)
        } else {
            None
        };
        match along {
            Some(d) if MOVES[np as usize].long == lp => {
                // 滑り: 自分の線dの上を動く。線dとその逆向きの線は付け替えるだけにする
                let od = opp(d);
                let axis = (1u8 << d) | (1u8 << od);
                if x == EMPTY {
                    self.through::<false>(dst, c, 1 << d);
                } else {
                    self.piece_effect::<false>(x, dst, 0);
                }
                self.piece_effect::<false>(p, src, axis);
                self.segment::<false>(c, src, dst, d);
                if lp & (1 << od) != 0 {
                    if leave != EMPTY {
                        // 移動元が塞がるので、後ろ向きの線の移動元より先を引く
                        self.ray::<false>(c, src, od);
                    }
                    self.segment::<true>(c, dst, src, od);
                }
                self.set_square(src, leave);
                self.set_square(dst, np);
                if leave == EMPTY {
                    self.through::<true>(src, c, 1 << od);
                } else {
                    self.piece_effect::<true>(leave, src, 0);
                }
                self.piece_effect::<true>(np, dst, axis);
                if x != EMPTY {
                    // 取った駒の先へ前向きの線が伸びる
                    self.ray::<true>(c, dst, d);
                }
            }
            _ => {
                // 自分の線の上の行き先で成って飛び利きの形が変わる手（香の成り）は、
                // 行き先を通る自分の線を切らずに、駒の利きとしてまとめて引く
                let excl = along.map_or(0, |d| 1u8 << d);
                if x == EMPTY {
                    self.through::<false>(dst, c, excl);
                } else {
                    self.piece_effect::<false>(x, dst, 0);
                }
                self.piece_effect::<false>(p, src, 0);
                self.set_square(src, leave);
                self.set_square(dst, np);
                if leave == EMPTY {
                    self.through::<true>(src, c, 0);
                } else {
                    self.piece_effect::<true>(leave, src, 0);
                }
                self.piece_effect::<true>(np, dst, 0);
            }
        }
    }

    /// 空き升sへ駒pを打つ。sを通っていた飛び利きを切り、駒の利きを足す。
    #[inline]
    fn put(&mut self, s: u8, p: u8) {
        self.through::<false>(s, 2, 0);
        self.set_square(s, p);
        self.piece_effect::<true>(p, s, 0);
    }

    /// 升sの駒を取り除く（駒打ちの戻し）。駒の利きを引き、sで止まっていた飛び利きを延ばす。
    #[inline]
    fn lift(&mut self, s: u8) {
        let p = self.board[s as usize];
        self.piece_effect::<false>(p, s, 0);
        self.set_square(s, EMPTY);
        self.through::<true>(s, 2, 0);
    }

    /// 利きを盤面から全部作り直す。
    fn effects_from_scratch(&self) -> ([[u8; LEN]; 2], [[u8; LEN]; 2]) {
        let mut tmp = self.clone();
        tmp.count = [[0; LEN]; 2];
        tmp.longdir = [[0; LEN]; 2];
        for &i in IDX81.iter() {
            let p = tmp.board[i as usize];
            if p != EMPTY {
                tmp.piece_effect::<true>(p, i, 0);
            }
        }
        (tmp.count, tmp.longdir)
    }

    // --- キーと王手の情報 ---

    /// 駒pが81升の升sに出入りしたときの、部分キー（歩構造・歩以外・小駒）の更新。
    #[inline]
    fn xor_partial_keys(st: &mut StateInfo, p: u8, s: u8) {
        let k = PSQ[p as usize][s as usize];
        let pt = piece_type(p);
        if pt == PAWN {
            st.pawn_key ^= k;
        } else {
            if is_minor_piece(pt) {
                st.minor_piece_key ^= k;
            }
            st.non_pawn_key[piece_color(p).index()] ^= k;
        }
    }

    #[inline]
    fn hand_key_of(&self) -> u64 {
        u64::from(self.hand_raw[0]) | (u64::from(self.hand_raw[1]) << 32)
    }

    /// 手番側の玉に王手をかけている駒の升の集合。利き数が0なら歩かずに0を返す。
    #[inline]
    fn compute_checkers(&self) -> u128 {
        let us = self.side;
        let k = self.king[us.index()];
        let them = us.flip();
        let ti = them.index();
        if self.count[ti][k as usize] == 0 {
            return 0;
        }
        let mut set = 0u128;
        let mut dirs = self.longdir[ti][k as usize];
        while dirs != 0 {
            let d = dirs.trailing_zeros() as usize;
            dirs &= dirs - 1;
            let back = DELTA[opp(d)] as i32;
            let mut x = (k as i32 + back) as u8;
            while self.board[x as usize] == EMPTY {
                x = (x as i32 + back) as u8;
            }
            set |= bit81(x);
        }
        for o in CHECK_OFFSETS {
            let x = (k as i32 - o as i32) as usize;
            let v = self.board[x];
            if v == EMPTY || v == WALL || piece_color(v) != them {
                continue;
            }
            let m = &MOVES[v as usize];
            if m.short[..m.n_short as usize].contains(&o) {
                set |= bit81(x as u8);
            }
        }
        set
    }

    /// キーと駒割りを盤面と手駒から全部作り直す（初期化と照合に使う）。
    /// 返す値の継続王手と手数の欄は0にする。
    fn state_from_scratch(&self) -> StateInfo {
        let mut st = StateInfo {
            captured: EMPTY,
            cap_id: NO_ID,
            dirty: DirtyPiece::default(),
            ..StateInfo::default()
        };
        for (c, &cc) in [Color::Black, Color::White].iter().enumerate() {
            let sign = if c == 0 { 1 } else { -1 };
            for &pt in HAND_KINDS.iter() {
                st.material += sign * PIECE_VALUE[pt as usize] * self.hand_count(cc, pt) as i32;
            }
            st.pawn_key ^= HAND_PAWN[c][self.hand_count(cc, PAWN) as usize];
        }
        for &i in IDX81.iter() {
            let p = self.board[i as usize];
            if p == EMPTY {
                continue;
            }
            let s81 = SQ81[i as usize];
            st.board_key ^= PSQ[p as usize][s81 as usize];
            Self::xor_partial_keys(&mut st, p, s81);
            let sign = if piece_color(p) == Color::Black {
                1
            } else {
                -1
            };
            st.material += sign * PIECE_VALUE[piece_type(p) as usize];
        }
        if self.side == Color::White {
            st.board_key ^= zobrist::SIDE;
        }
        st.hand_key = self.hand_key_of();
        st.checkers = self.compute_checkers();
        st
    }

    // --- 指し手 ---

    pub fn do_move(&mut self, m: Move) {
        #[cfg(feature = "copy-undo")]
        self.snapshots.push(EffectSnapshot {
            count: self.count,
            longdir: self.longdir,
        });
        let us = self.side;
        let ui = us.index();
        let them = us.flip().index();
        let sign = if us == Color::Black { 1 } else { -1 };
        let prev = self.state();
        let mut st = StateInfo {
            captured: EMPTY,
            cap_id: NO_ID,
            board_key: prev.board_key,
            hand_key: 0,
            pawn_key: prev.pawn_key,
            non_pawn_key: prev.non_pawn_key,
            minor_piece_key: prev.minor_piece_key,
            checkers: 0,
            continuous_check: prev.continuous_check,
            plies_from_null: prev.plies_from_null + 1,
            material: prev.material,
            dirty: DirtyPiece::default(),
        };
        let to = m.to();
        let to81 = SQ81[to as usize];
        if m.is_drop() {
            let pt = m.drop_type();
            let pc = make_piece(us, pt);
            let h = &mut self.hand_ids[ui][hand_index(pt)];
            debug_assert!(*h != 0);
            let id = h.trailing_zeros() as u8;
            *h &= *h - 1;
            self.hand_raw[ui] -= 1 << HAND_SHIFT[pt as usize];
            self.on_board[ui] |= 1 << id;
            self.sq_of[id as usize] = to;
            self.id_on[to as usize] = id;
            self.put(to, pc);
            st.board_key ^= PSQ[pc as usize][to81 as usize];
            if pt == PAWN {
                self.pawn_files[ui] |= 1 << file_of(to);
                let new = self.hand_count(us, PAWN) as usize;
                st.pawn_key ^= HAND_PAWN[ui][new + 1] ^ HAND_PAWN[ui][new];
            }
            Self::xor_partial_keys(&mut st, pc, to81);
            st.dirty.count = 1;
            st.dirty.piece_new[0] = pc;
            st.dirty.to[0] = to81;
        } else {
            let from = m.from();
            let from81 = SQ81[from as usize];
            let p = self.board[from as usize];
            let np = if m.is_promote() { p - 8 } else { p };
            let id = self.id_on[from as usize];
            let cap = self.board[to as usize];
            self.transfer(from, to, np, EMPTY);
            self.id_on[from as usize] = NO_ID;
            if cap != EMPTY {
                let cid = self.id_on[to as usize];
                let hk = piece_type(cap) | 8;
                self.on_board[them] &= !(1u64 << cid);
                self.hand_ids[ui][hand_index(hk)] |= 1 << cid;
                self.hand_raw[ui] += 1 << HAND_SHIFT[hk as usize];
                if piece_type(cap) == PAWN {
                    self.pawn_files[them] &= !(1 << file_of(to));
                }
                st.board_key ^= PSQ[cap as usize][to81 as usize];
                Self::xor_partial_keys(&mut st, cap, to81);
                if hk == PAWN {
                    let new = self.hand_count(us, PAWN) as usize;
                    st.pawn_key ^= HAND_PAWN[ui][new - 1] ^ HAND_PAWN[ui][new];
                }
                st.material +=
                    sign * (PIECE_VALUE[piece_type(cap) as usize] + PIECE_VALUE[hk as usize]);
                st.captured = cap;
                st.cap_id = cid;
                st.dirty.count = 2;
                st.dirty.piece_old[1] = cap;
                st.dirty.piece_new[1] = make_piece(us, hk);
                st.dirty.from[1] = to81;
            } else {
                st.dirty.count = 1;
            }
            self.id_on[to as usize] = id;
            self.sq_of[id as usize] = to;
            st.board_key ^= PSQ[p as usize][from81 as usize] ^ PSQ[np as usize][to81 as usize];
            Self::xor_partial_keys(&mut st, p, from81);
            Self::xor_partial_keys(&mut st, np, to81);
            if m.is_promote() {
                st.material += sign
                    * (PIECE_VALUE[piece_type(np) as usize] - PIECE_VALUE[piece_type(p) as usize]);
            }
            if piece_type(p) == KING {
                self.king[ui] = to;
                st.dirty.king_moved = true;
            } else if piece_type(p) == PAWN && m.is_promote() {
                self.pawn_files[ui] &= !(1 << file_of(to));
            }
            st.dirty.piece_old[0] = p;
            st.dirty.piece_new[0] = np;
            st.dirty.from[0] = from81;
            st.dirty.to[0] = to81;
        }
        st.board_key ^= zobrist::SIDE;
        st.hand_key = self.hand_key_of();
        self.side = us.flip();
        self.ply += 1;
        // 王手の情報。利き数で王手の有無が決まるので、王手のときだけ王手駒を探す
        st.checkers = self.compute_checkers();
        if st.checkers != 0 {
            st.continuous_check[ui] += 1;
        } else {
            st.continuous_check[ui] = 0;
        }
        self.states.push(st);
        #[cfg(debug_assertions)]
        self.assert_consistent();
    }

    pub fn undo_move(&mut self, m: Move) {
        let u = self.states.pop().expect("undo_move without do_move");
        #[cfg(feature = "copy-undo")]
        let snap = self.snapshots.pop().expect("snapshot stack");
        self.side = self.side.flip();
        self.ply -= 1;
        let us = self.side;
        let ui = us.index();
        let them = us.flip().index();
        let to = m.to();
        if m.is_drop() {
            let pt = m.drop_type();
            let id = self.id_on[to as usize];
            if cfg!(feature = "copy-undo") {
                self.set_square(to, EMPTY);
            } else {
                self.lift(to);
            }
            self.id_on[to as usize] = NO_ID;
            self.on_board[ui] &= !(1u64 << id);
            self.hand_ids[ui][hand_index(pt)] |= 1 << id;
            self.hand_raw[ui] += 1 << HAND_SHIFT[pt as usize];
            if pt == PAWN {
                self.pawn_files[ui] &= !(1 << file_of(to));
            }
        } else {
            let from = m.from();
            let np = self.board[to as usize];
            let p = if m.is_promote() { np + 8 } else { np };
            let id = self.id_on[to as usize];
            if cfg!(feature = "copy-undo") {
                self.set_square(to, u.captured);
                self.set_square(from, p);
            } else {
                self.transfer(to, from, p, u.captured);
            }
            if u.captured != EMPTY {
                let cid = u.cap_id;
                self.id_on[to as usize] = cid;
                self.sq_of[cid as usize] = to;
                self.on_board[them] |= 1 << cid;
                let hk = piece_type(u.captured) | 8;
                self.hand_ids[ui][hand_index(hk)] &= !(1u64 << cid);
                self.hand_raw[ui] -= 1 << HAND_SHIFT[hk as usize];
                if piece_type(u.captured) == PAWN {
                    self.pawn_files[them] |= 1 << file_of(to);
                }
            } else {
                self.id_on[to as usize] = NO_ID;
            }
            self.id_on[from as usize] = id;
            self.sq_of[id as usize] = from;
            if piece_type(p) == KING {
                self.king[ui] = from;
            } else if piece_type(p) == PAWN && m.is_promote() {
                self.pawn_files[ui] |= 1 << file_of(from);
            }
        }
        #[cfg(feature = "copy-undo")]
        {
            self.count = snap.count;
            self.longdir = snap.longdir;
        }
        #[cfg(debug_assertions)]
        self.assert_consistent();
    }

    /// 不変条件を全部検査する。利きは全計算と、駒番号は盤と手駒と突き合わせる。
    /// 食い違いがあれば説明を返す。
    pub fn check_consistency(&self) -> Result<(), String> {
        let (count, longdir) = self.effects_from_scratch();
        for c in 0..2 {
            for &i in IDX81.iter() {
                let i = i as usize;
                if count[c][i] != self.count[c][i] || longdir[c][i] != self.longdir[c][i] {
                    return Err(format!(
                        "利きの不一致 色{c} 升{}: 差分 {}/{:08b} 全計算 {}/{:08b}",
                        usi_square(i as u8),
                        self.count[c][i],
                        self.longdir[c][i],
                        count[c][i],
                        longdir[c][i]
                    ));
                }
            }
        }
        let fresh = self.state_from_scratch();
        let st = self.state();
        let keys = |x: &StateInfo| {
            (
                x.board_key,
                x.hand_key,
                x.pawn_key,
                x.non_pawn_key,
                x.minor_piece_key,
                x.material,
                x.checkers,
            )
        };
        if keys(&fresh) != keys(st) {
            return Err(format!(
                "キー・駒割り・王手駒の食い違い: 差分 {:?} 全計算 {:?}",
                keys(st),
                keys(&fresh)
            ));
        }
        let mut seen = 0u64;
        let mut pawn_files = [0u16; 2];
        for &i in IDX81.iter() {
            let p = self.board[i as usize];
            let id = self.id_on[i as usize];
            if p == EMPTY {
                if id != NO_ID {
                    return Err(format!("空き升に駒番号 {}", usi_square(i)));
                }
                continue;
            }
            if id == NO_ID || self.sq_of[id as usize] != i {
                return Err(format!("駒番号の食い違い {}", usi_square(i)));
            }
            let c = piece_color(p).index();
            if self.on_board[c] & (1 << id) == 0 {
                return Err(format!("盤上の集合に無い駒番号 {id}"));
            }
            seen |= 1 << id;
            if piece_type(p) == KING && self.king[c] != i {
                return Err("玉の升の食い違い".into());
            }
            if piece_type(p) == PAWN {
                pawn_files[c] |= 1 << file_of(i);
            }
        }
        if seen != self.on_board[0] | self.on_board[1] || self.on_board[0] & self.on_board[1] != 0 {
            return Err("盤上の駒番号の集合が盤と合わない".into());
        }
        let mut occ = [0u128; 2];
        for &i in IDX81.iter() {
            let p = self.board[i as usize];
            if p != EMPTY {
                occ[piece_color(p).index()] |= bit81(i);
            }
        }
        if occ != self.occ {
            return Err("駒のいる升の集合の食い違い".into());
        }
        if pawn_files != self.pawn_files {
            return Err("二歩の筋の食い違い".into());
        }
        let mut hands = 0u64;
        for c in 0..2 {
            for k in 0..8 {
                let h = self.hand_ids[c][k];
                if h & (hands | seen) != 0 {
                    return Err("手駒の駒番号が重なる".into());
                }
                hands |= h;
            }
        }
        Ok(())
    }

    #[cfg(debug_assertions)]
    fn assert_consistent(&self) {
        if let Err(e) = self.check_consistency() {
            panic!("{e}: {}", self.to_sfen());
        }
    }

    // --- SFEN ---

    pub fn from_sfen(sfen: &str) -> Result<Position, SfenError> {
        let mut tok = sfen.split_whitespace();
        let board_str = tok.next().ok_or(SfenError("盤面がない"))?;
        let side_str = tok.next().ok_or(SfenError("手番がない"))?;
        let hands_str = tok.next().ok_or(SfenError("手駒がない"))?;
        let ply_str = tok.next().unwrap_or("1");
        let mut pos = Position::empty();
        let mut next_id = 0u8;
        let mut new_id = || -> Result<u8, SfenError> {
            if next_id >= 40 {
                return Err(SfenError("駒が40枚を超える"));
            }
            next_id += 1;
            Ok(next_id - 1)
        };
        let mut kings = [0u8; 2];
        let ranks: Vec<&str> = board_str.split('/').collect();
        if ranks.len() != 9 {
            return Err(SfenError("段の数が9でない"));
        }
        for (r, rs) in ranks.iter().enumerate() {
            let mut f: i32 = 8;
            let mut promote = false;
            for ch in rs.chars() {
                if let Some(d) = ch.to_digit(10) {
                    if promote {
                        return Err(SfenError("+の後に数字"));
                    }
                    f -= d as i32;
                } else if ch == '+' {
                    promote = true;
                } else {
                    if f < 0 {
                        return Err(SfenError("筋があふれた"));
                    }
                    let mut pt =
                        type_of_sfen(ch.to_ascii_uppercase()).ok_or(SfenError("不明な駒文字"))?;
                    if promote {
                        if !can_promote(pt) {
                            return Err(SfenError("成れない駒に+"));
                        }
                        pt -= 8;
                    }
                    promote = false;
                    let c = if ch.is_ascii_uppercase() {
                        Color::Black
                    } else {
                        Color::White
                    };
                    let i = idx(f as u8, r as u8);
                    let rr = rel_rank(c, i);
                    if (rr == 0 && (pt == PAWN || pt == LANCE || pt == KNIGHT))
                        || (rr == 1 && pt == KNIGHT)
                    {
                        return Err(SfenError("行き所のない駒"));
                    }
                    if pt == PAWN {
                        if pos.pawn_files[c.index()] & (1 << f) != 0 {
                            return Err(SfenError("二歩"));
                        }
                        pos.pawn_files[c.index()] |= 1 << f;
                    }
                    if pt == KING {
                        kings[c.index()] += 1;
                        pos.king[c.index()] = i;
                    }
                    let id = new_id()?;
                    pos.set_square(i, make_piece(c, pt));
                    pos.id_on[i as usize] = id;
                    pos.sq_of[id as usize] = i;
                    pos.on_board[c.index()] |= 1 << id;
                    f -= 1;
                }
            }
            if f != -1 {
                return Err(SfenError("筋の数が9でない"));
            }
        }
        if kings != [1, 1] {
            return Err(SfenError("玉が先後1枚ずつでない"));
        }
        pos.side = match side_str {
            "b" => Color::Black,
            "w" => Color::White,
            _ => return Err(SfenError("手番はbかw")),
        };
        if hands_str != "-" {
            let mut n = 0u32;
            for ch in hands_str.chars() {
                if let Some(d) = ch.to_digit(10) {
                    n = n * 10 + d;
                    if n > 18 {
                        return Err(SfenError("手駒の枚数が過大"));
                    }
                } else {
                    let pt =
                        type_of_sfen(ch.to_ascii_uppercase()).ok_or(SfenError("不明な手駒文字"))?;
                    if pt == KING {
                        return Err(SfenError("玉は手駒にできない"));
                    }
                    let c = if ch.is_ascii_uppercase() { 0 } else { 1 };
                    for _ in 0..n.max(1) {
                        let id = new_id()?;
                        pos.hand_ids[c][hand_index(pt)] |= 1 << id;
                    }
                    n = 0;
                }
            }
        }
        pos.ply = ply_str.parse().map_err(|_| SfenError("手数が数でない"))?;
        let (count, longdir) = pos.effects_from_scratch();
        pos.count = count;
        pos.longdir = longdir;
        for c in 0..2 {
            for k in 1..8 {
                let n = pos.hand_ids[c][k].count_ones();
                pos.hand_raw[c] += n << HAND_SHIFT[k + 8];
            }
        }
        let st = pos.state_from_scratch();
        pos.states.push(st);
        // 手番でない側の玉に王手がかかっている局面は受け付けない
        let them = pos.side.flip();
        if pos.count[pos.side.index()][pos.king[them.index()] as usize] != 0 {
            return Err(SfenError("手番でない側の玉に王手がかかっている"));
        }
        Ok(pos)
    }

    pub fn to_sfen(&self) -> String {
        let mut s = String::new();
        for r in 0..9 {
            let mut empty = 0;
            for f in (0..9).rev() {
                let p = self.board[idx(f, r) as usize];
                if p == EMPTY {
                    empty += 1;
                    continue;
                }
                if empty > 0 {
                    s.push_str(&empty.to_string());
                    empty = 0;
                }
                let t = sfen_of_type(piece_type(p));
                if piece_color(p) == Color::Black {
                    s.push_str(t);
                } else {
                    s.push_str(&t.to_lowercase());
                }
            }
            if empty > 0 {
                s.push_str(&empty.to_string());
            }
            if r < 8 {
                s.push('/');
            }
        }
        s.push(' ');
        s.push(if self.side == Color::Black { 'b' } else { 'w' });
        s.push(' ');
        let mut hand = String::new();
        for c in [Color::Black, Color::White] {
            for pt in HAND_KINDS {
                let n = self.hand_count(c, pt);
                if n == 0 {
                    continue;
                }
                if n > 1 {
                    hand.push_str(&n.to_string());
                }
                let t = sfen_of_type(pt);
                if c == Color::Black {
                    hand.push_str(t);
                } else {
                    hand.push_str(&t.to_lowercase());
                }
            }
        }
        if hand.is_empty() {
            hand.push('-');
        }
        s.push_str(&hand);
        s.push(' ');
        s.push_str(&self.ply.to_string());
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startpos_roundtrip_and_effects() {
        let pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
        assert_eq!(pos.to_sfen(), SFEN_STARTPOS);
        pos.check_consistency().unwrap();
        // 7六の升（7g7fの行き先）には先手の歩と角の利きが届かない。歩1枚だけ
        let i = parse_usi_square(b"7f").unwrap();
        assert_eq!(pos.effect_count(Color::Black, i), 1);
        // 2八の飛は2七〜2二と2九、1八〜3八へ利く。2七には歩もいるので飛の利きは2七で止まる
        let i = parse_usi_square(b"2g").unwrap();
        assert_eq!(pos.long_dirs(Color::Black, i), 1 << UP);
        assert!(!pos.in_check());
    }
}
