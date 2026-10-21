//! 配列盤の局面。手番別の利き数・飛び利きの方向ビットと、駒のいる升の集合を持つ。
//!
//! 利きの差分更新は、盤上の駒の移動をまとめて扱う `transfer` と、駒打ちの `put` で行う。
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
//! 戻しは既定で記録による（ADR-0241の第15回）。短い利きは窓を逆の向きに当て直し、飛び利きは進めるときに
//! 積んだ書き換えの記録（`ray_log`）へ逆の操作を当てる。フィーチャ `copy-undo` を有効にすると、
//! 進めるたびに利きの表を積み、戻しは表を書き戻して盤だけを戻す（ADR-0241の段階4で比べる）。

use crate::board::*;
use crate::check::{CheckSlot, SHORT_FROM, first81};
use crate::moves::Move;
use crate::zobrist::{self, HAND_MASK, HAND_PAWN, HAND_SHIFT, PIECE_VALUE, PSQ, is_minor_piece};

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

/// 駒の短い利きを、3列ぶんの8バイトの窓へまとめて足すための表（ADR-0241の第15回）。
///
/// `SHORT_WIN[p][j]` は、升sの駒pが列 j−1（差分 (j−1)·STRIDE）へ持つ短い利きを、
/// 升 s+(j−1)·STRIDE−4 から8升ぶんの語（リトルエンディアン、1升1バイト）にしたもの。
/// 短い利きの段の差は±2に収まるので、どの利きも窓の3〜7バイト目に入る。
/// 利き数は升ごとに255を超えず、引くときはその駒の分が必ず入っているので、
/// 語のまま足し引きしても隣の升へ桁が溢れない（壁の升の利き数も同じ理由で崩れない）。
#[cfg(target_endian = "big")]
compile_error!("SHORT_WIN はリトルエンディアンの語を前提にしている");
const SHORT_WIN: [[u64; 3]; 32] = {
    let mut t = [[0u64; 3]; 32];
    let mut p = 0;
    while p < 32 {
        let m = &MOVES[p];
        let mut k = 0;
        while k < m.n_short as usize {
            let o = m.short[k] as i32;
            let dc = (o + STRIDE / 2).div_euclid(STRIDE);
            let dr = o - dc * STRIDE;
            t[p][(dc + 1) as usize] |= 1u64 << (8 * (dr + 4));
            k += 1;
        }
        p += 1;
    }
    t
};

/// 駒種ごとの、金の動きの駒（金と成駒4種）なら全部のビットが立つ語。`psets[c][1]` の更新に使う。
const GOLD_LIKE_MASK: [u128; 16] = {
    let mut t = [0u128; 16];
    let mut pt = 2;
    while pt <= 5 {
        t[pt] = u128::MAX;
        pt += 1;
    }
    t[GOLD as usize] = u128::MAX;
    t
};

/// 8升ぶんの利き数に1ずつ足す語。
const ONES64: u64 = 0x0101_0101_0101_0101;

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
    /// この手を進める前の `ray_log` の長さ。戻しはここから後ろを逆順に戻す
    pub(crate) log_start: u32,
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
    /// 持ち主別の駒のいる升の集合（81升の通し番号のビット）。
    /// 空き升の列挙と、生成の順をcoreに揃えるために持つ。利きの計算には使わない。
    pub(crate) occ: [u128; 2],
    /// 持ち主別・駒種別の駒のいる升の集合（81升）。評価関数の駒の特徴の作り直しに使う。
    /// 盤を書き換えるたびに差分で直す。駒種1（壁）の欄には金の動きの5駒種（金と成駒4種）の和を持ち、
    /// 駒種0の欄は使わない
    pub(crate) psets: [[u128; 16]; 2],
    pub(crate) king: [u8; 2],
    /// 生の歩がいる筋のビット（二歩の判定）。
    pub(crate) pawn_files: [u16; 2],
    pub(crate) side: Color,
    ply: u32,
    /// 手駒をcoreの `Hand` と同じ詰め方で持った値（hand_keyに使う）。
    hand_raw: [u32; 2],
    pub(crate) states: Vec<StateInfo>,
    /// 局面の深さごとの、王手の判定の手がかり（`gives_check` が初めて呼ばれたときに作る）。
    /// 添字は `states` と同じ。進めるたびにその深さの欄を無効にする
    pub(crate) check_slots: Vec<CheckSlot>,
    /// 進めるときに飛び利きで書き換えた升の記録（`log_ray` の語）。戻しで逆順に戻す
    ray_log: Vec<u32>,
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
            occ: [0; 2],
            psets: [[0; 16]; 2],
            count: [[0; LEN]; 2],
            longdir: [[0; LEN]; 2],
            king: [0; 2],
            pawn_files: [0; 2],
            side: Color::Black,
            ply: 1,
            hand_raw: [0; 2],
            states: Vec::with_capacity(512),
            check_slots: Vec::with_capacity(512),
            ray_log: Vec::with_capacity(4096),
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
        (self.hand_raw[c.index()] >> HAND_SHIFT[pt as usize]) & HAND_MASK[pt as usize]
    }

    /// 色（添字ci）が駒種ptの手駒を持つか。
    #[inline]
    pub(crate) fn has_hand(&self, ci: usize, pt: u8) -> bool {
        self.hand_raw[ci] & (HAND_MASK[pt as usize] << HAND_SHIFT[pt as usize]) != 0
    }

    /// 色（添字ci）が手駒を1枚でも持つか。
    #[inline]
    pub(crate) fn has_any_hand(&self, ci: usize) -> bool {
        self.hand_raw[ci] != 0
    }

    #[inline]
    pub fn state(&self) -> &StateInfo {
        self.states.last().expect("state stack is never empty")
    }

    /// いまの局面の深さ（`states` と `check_slots` の添字）。
    #[inline]
    pub(crate) fn check_slots_depth(&self) -> usize {
        self.states.len() - 1
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
    /// LOGなら、飛び利きで書き換えた升を戻しの記録（`ray_log`）へ積む。
    #[inline]
    fn piece_effect<const ADD: bool, const LOG: bool>(&mut self, p: u8, s: u8, skip: u8) {
        self.short_effect::<ADD>(p, s);
        let c = piece_color(p).index();
        // 駒の符号は32未満。マスクで境界検査を消す
        let mut dirs = MOVES[(p & 31) as usize].long & !skip;
        while dirs != 0 {
            let d = dirs.trailing_zeros() as usize;
            dirs &= dirs - 1;
            self.ray::<ADD, LOG>(c, s, d);
        }
    }

    /// 升sの駒pの短い利きを足す（ADD）か引く。3列ぶんの8升の窓を語のまま足し引きする。
    /// 壁の升にも書くが、壁の値は読まない。利き数の足し引きだけなので、戻しは逆の向きで当て直す
    #[inline]
    fn short_effect<const ADD: bool>(&mut self, p: u8, s: u8) {
        let c = piece_color(p).index();
        let win = &SHORT_WIN[(p & 31) as usize];
        let row = self.count[c].as_mut_ptr();
        for (j, &w) in win.iter().enumerate() {
            let at = s as isize + (j as isize - 1) * STRIDE as isize - 4;
            debug_assert!(at >= 0 && at as usize + 8 <= LEN);
            // SAFETY: sは盤上の升なので、隣の列の窓 [s±STRIDE−4, s±STRIDE+4) も配列の内側にある
            // （`every_step_from_board_stays_in_array` と同じ壁の幅による）
            unsafe {
                let q = row.offset(at).cast::<u64>();
                let v = q.read_unaligned();
                q.write_unaligned(if ADD {
                    v.wrapping_add(w)
                } else {
                    v.wrapping_sub(w)
                });
            }
        }
    }

    /// 飛び利きの書き換えを1つ、戻しの記録（`ray_log`）へ積む。
    ///
    /// 書き換えは「色cの表の添字iから始まるn升（1〜8）の利き数に1を足す（ADD）か引き、方向ビットdを反転する」
    /// の形にそろえてある。n升は、縦（上・下）なら添字の増える向きに続いた升、横と斜めならiから方向dへ進んだ升である。方向ビットは、足すときは立っていない升に立て、引くときは立っている升から下ろすので、
    /// どちらも反転に等しい。書き換えはどれも足し引きと反転だけなので、戻しは逆の操作を順不同で当てればよい。
    /// 語は 下位10ビットが表の通しの添字（c×LEN＋i）、10〜13ビットがn、14〜16ビットがd、17ビットがADDである
    #[inline(always)]
    fn log_ray<const ADD: bool>(&mut self, c: usize, i: usize, n: u32, d: usize) {
        let e = (c * LEN + i) as u32 | n << 10 | (d as u32) << 14 | u32::from(ADD) << 17;
        self.ray_log.push(e);
    }

    /// 色cの升iから続くn升（1〜8、同じ列の中）の利き数に1を足すか引き、方向ビットdを反転する。
    #[inline(always)]
    fn column_run<const ADD: bool, const LOG: bool>(
        &mut self,
        c: usize,
        i: usize,
        n: u32,
        d: usize,
    ) {
        debug_assert!((1..=8).contains(&n) && i + 8 <= LEN);
        let mask = ONES64 >> (64 - 8 * n);
        // SAFETY: iは盤上の升で、その後ろ8升も配列の内側にある（下の壁が6升以上ある）
        unsafe {
            let q = self.count[c].as_mut_ptr().add(i).cast::<u64>();
            let v = q.read_unaligned();
            q.write_unaligned(if ADD {
                v.wrapping_add(mask)
            } else {
                v.wrapping_sub(mask)
            });
            let q = self.longdir[c].as_mut_ptr().add(i).cast::<u64>();
            q.write_unaligned(q.read_unaligned() ^ (mask << d));
        }
        if LOG {
            self.log_ray::<ADD>(c, i, n, d);
        }
    }

    /// `ray_log` のstartから後ろの書き換えを、逆の操作で戻す。
    #[cfg(not(feature = "copy-undo"))]
    #[inline]
    fn replay_ray_log(&mut self, start: usize) {
        let count = self.count.as_flattened_mut().as_mut_ptr();
        let longdir = self.longdir.as_flattened_mut().as_mut_ptr();
        for &e in self.ray_log[start..].iter() {
            let off = (e & 0x3FF) as usize;
            let n = (e >> 10) & 15;
            let d = (e >> 14) & 7;
            if d >= 2 {
                // 横と斜め: iから方向dへn升。同じ色の表の盤上の升だけを触る
                let step = DELTA[d as usize] as isize;
                let b = 1u8 << d;
                let add = (e >> 17) & 1 != 0;
                for k in 0..n as isize {
                    let at = off as isize + k * step;
                    debug_assert!(at >= 0 && (at as usize) < 2 * LEN);
                    // SAFETY: 記録した升は `ray`・`segment` が書いた盤上の升で、表の内側にある
                    unsafe {
                        let q = count.offset(at);
                        *q = if add { *q - 1 } else { *q + 1 };
                        *longdir.offset(at) ^= b;
                    }
                }
                continue;
            }
            let add = u64::from((e >> 17) & 1);
            let mask = ONES64 >> (64 - 8 * n);
            debug_assert!(off + 8 <= 2 * LEN);
            // SAFETY: 記録した添字は `column_run` と同じく、後ろ8升が表の内側にある
            unsafe {
                let q = count.add(off).cast::<u64>();
                // 足した升は1引き、引いた升は1足す
                let v = q.read_unaligned().wrapping_add(mask);
                q.write_unaligned(v.wrapping_sub((mask & add.wrapping_neg()) << 1));
                let q = longdir.add(off).cast::<u64>();
                q.write_unaligned(q.read_unaligned() ^ (mask << d));
            }
        }
        self.ray_log.truncate(start);
    }

    /// 升sの先、方向dへ伸びる色cの飛び利きを足す（ADD）か引く。最初の駒の升まで含める。
    ///
    /// 縦の線（上・下）は同じ列の続いた升なので、駒のいる升の集合から長さを引き、語1つでまとめて書く。
    #[inline]
    fn ray<const ADD: bool, const LOG: bool>(&mut self, c: usize, s: u8, d: usize) {
        if d == UP || d == DOWN {
            let f = file_of(s);
            let r = u32::from(rank_of(s));
            let col = ((self.occ[0] | self.occ[1]) >> (9 * u32::from(f))) as u32 & 0x1FF;
            if d == UP {
                if r == 0 {
                    return;
                }
                let o = col & ((1 << r) - 1);
                let n = if o == 0 {
                    r
                } else {
                    r - (31 - o.leading_zeros())
                };
                self.column_run::<ADD, LOG>(c, s as usize - n as usize, n, d);
            } else {
                if r == 8 {
                    return;
                }
                let o = col >> (r + 1);
                let n = if o == 0 {
                    8 - r
                } else {
                    o.trailing_zeros() + 1
                };
                self.column_run::<ADD, LOG>(c, s as usize + 1, n, d);
            }
            return;
        }
        // 横と斜めは盤を歩きながら書き、記録は線ごとに1件だけ積む
        let delta = DELTA[d] as u8;
        let b = 1u8 << d;
        let start = s.wrapping_add(delta);
        let mut t = start;
        let mut n = 0u32;
        loop {
            let i = t as usize;
            let v = self.board[i];
            if v == WALL {
                break;
            }
            n += 1;
            if ADD {
                self.count[c][i] += 1;
                self.longdir[c][i] |= b;
            } else {
                self.count[c][i] -= 1;
                self.longdir[c][i] &= !b;
            }
            if v != EMPTY {
                break;
            }
            t = t.wrapping_add(delta);
        }
        if LOG && n != 0 {
            self.log_ray::<ADD>(c, start as usize, n, d);
        }
    }

    /// 升sに届く飛び利きを、sの先で切る（ADD=false）か延ばす（ADD=true）。
    /// 色exclの方向ビットexcl_bitsは触らない（動く駒自身の線）。
    #[inline]
    fn through<const ADD: bool, const LOG: bool>(&mut self, s: u8, excl: usize, excl_bits: u8) {
        for c in 0..2 {
            let mut dirs = self.longdir[c][s as usize];
            if c == excl {
                dirs &= !excl_bits;
            }
            while dirs != 0 {
                let d = dirs.trailing_zeros() as usize;
                dirs &= dirs - 1;
                self.ray::<ADD, LOG>(c, s, d);
            }
        }
    }

    /// 升aの次から升bまで（両端のうちbを含む）、方向dへ進む色cの飛び利きを足すか引く。
    /// 間の升が空きで、bがaから方向dの線上にあることが前提。
    #[inline]
    fn segment<const ADD: bool, const LOG: bool>(&mut self, c: usize, a: u8, b: u8, d: usize) {
        if d == UP {
            self.column_run::<ADD, LOG>(c, b as usize, u32::from(a - b), d);
            return;
        }
        if d == DOWN {
            self.column_run::<ADD, LOG>(c, a as usize + 1, u32::from(b - a), d);
            return;
        }
        let delta = DELTA[d] as u8;
        let bit = 1u8 << d;
        let start = a.wrapping_add(delta);
        let mut t = a;
        let mut n = 0u32;
        loop {
            t = t.wrapping_add(delta);
            let i = t as usize;
            n += 1;
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
        if LOG {
            self.log_ray::<ADD>(c, start as usize, n, d);
        }
    }

    #[inline]
    fn set_square(&mut self, s: u8, p: u8) {
        let old = self.board[s as usize];
        let b = bit81(s);
        if old != EMPTY {
            let c = piece_color(old).index();
            self.occ[c] &= !b;
            let pt = piece_type(old) as usize;
            self.psets[c][pt] &= !b;
            self.psets[c][1] &= !(b & GOLD_LIKE_MASK[pt]);
        }
        if p != EMPTY {
            let c = piece_color(p).index();
            self.occ[c] |= b;
            let pt = piece_type(p) as usize;
            self.psets[c][pt] |= b;
            self.psets[c][1] |= b & GOLD_LIKE_MASK[pt];
        }
        self.board[s as usize] = p;
    }

    /// 進めと戻しで、盤上の移動に合わせて駒種ごとの集合（`psets`）を反転する。
    /// 色cの駒pが81升のビットbfからbtへ動いて駒npになり、btにいた相手の駒cap（空きもある）が消える。
    /// 反転なので、進めと戻しで同じ呼び出しになる
    #[inline(always)]
    fn toggle_piece_sets(&mut self, c: usize, p: u8, np: u8, cap: u8, bf: u128, bt: u128) {
        let (pt, npt) = (piece_type(p) as usize, piece_type(np) as usize);
        let ps = &mut self.psets[c];
        ps[pt] ^= bf;
        ps[npt] ^= bt;
        ps[1] ^= (bf & GOLD_LIKE_MASK[pt]) | (bt & GOLD_LIKE_MASK[npt]);
        if cap != EMPTY {
            let xt = piece_type(cap) as usize;
            let ps = &mut self.psets[c ^ 1];
            ps[xt] ^= bt;
            ps[1] ^= bt & GOLD_LIKE_MASK[xt];
        }
    }

    /// 駒打ちと、その戻しで駒種ごとの集合を反転する。
    #[inline(always)]
    fn toggle_drop_sets(&mut self, c: usize, pc: u8, b: u128) {
        let pt = piece_type(pc) as usize;
        let ps = &mut self.psets[c];
        ps[pt] ^= b;
        ps[1] ^= b & GOLD_LIKE_MASK[pt];
    }

    /// 升srcの駒を升dstへ動かし、dstに駒npを置く（進めるときだけ使う）。dstは空きか取られる駒である。
    /// bf・btは移動元と行き先の81升のビットである。盤と駒のいる升の集合を書き換え、利きを差分で直す。
    /// 駒種ごとの集合（`psets`）は呼び出し側が直す
    #[inline]
    fn transfer(&mut self, src: u8, dst: u8, np: u8, bf: u128, bt: u128) {
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
                    self.through::<false, true>(dst, c, 1 << d);
                } else {
                    self.piece_effect::<false, true>(x, dst, 0);
                }
                self.piece_effect::<false, true>(p, src, axis);
                self.segment::<false, true>(c, src, dst, d);
                if lp & (1 << od) != 0 {
                    self.segment::<true, true>(c, dst, src, od);
                }
                self.move_on_board(src, dst, np, x, c, bf, bt);
                self.through::<true, true>(src, c, 1 << od);
                self.piece_effect::<true, true>(np, dst, axis);
                if x != EMPTY {
                    // 取った駒の先へ前向きの線が伸びる
                    self.ray::<true, true>(c, dst, d);
                }
            }
            _ => {
                // 自分の線の上の行き先で成って飛び利きの形が変わる手（香の成り）は、
                // 行き先を通る自分の線を切らずに、駒の利きとしてまとめて引く
                let excl = along.map_or(0, |d| 1u8 << d);
                if x == EMPTY {
                    self.through::<false, true>(dst, c, excl);
                } else {
                    self.piece_effect::<false, true>(x, dst, 0);
                }
                self.piece_effect::<false, true>(p, src, 0);
                self.move_on_board(src, dst, np, x, c, bf, bt);
                self.through::<true, true>(src, c, 0);
                self.piece_effect::<true, true>(np, dst, 0);
            }
        }
    }

    /// 盤の上で駒を動かす（盤と駒のいる升の集合だけ）。進めと戻しの両方で使う。
    /// 進めるときは src→dst（dstの駒xが消える）、戻すときは引数の役を入れ替えて呼ぶ。
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn move_on_board(&mut self, src: u8, dst: u8, np: u8, x: u8, c: usize, bf: u128, bt: u128) {
        self.board[src as usize] = EMPTY;
        self.board[dst as usize] = np;
        self.occ[c] ^= bf | bt;
        if x != EMPTY {
            self.occ[c ^ 1] ^= bt;
        }
    }

    /// 空き升sへ駒pを打つ。sを通っていた飛び利きを切り、駒の利きを足す。
    #[inline]
    fn put(&mut self, s: u8, p: u8, b: u128) {
        self.through::<false, true>(s, 2, 0);
        self.board[s as usize] = p;
        let c = piece_color(p).index();
        self.occ[c] ^= b;
        self.toggle_drop_sets(c, p, b);
        self.piece_effect::<true, true>(p, s, 0);
    }

    /// 利きを盤面から全部作り直す。
    fn effects_from_scratch(&self) -> ([[u8; LEN]; 2], [[u8; LEN]; 2]) {
        let mut tmp = self.clone();
        tmp.count = [[0; LEN]; 2];
        tmp.longdir = [[0; LEN]; 2];
        for &i in IDX81.iter() {
            let p = tmp.board[i as usize];
            if p != EMPTY {
                tmp.piece_effect::<true, false>(p, i, 0);
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
        let k81 = SQ81[k as usize];
        let occ = self.occ[0] | self.occ[1];
        let mut set = 0u128;
        let ld = self.longdir[ti][k as usize];
        let mut dirs = ld;
        while dirs != 0 {
            let d = dirs.trailing_zeros() as usize;
            dirs &= dirs - 1;
            let x81 = first81(k81, opp(d), occ).expect("飛び利きの元の駒がある");
            set |= 1u128 << x81;
        }
        let mut need = u32::from(self.count[ti][k as usize]) - ld.count_ones();
        if need != 0 {
            for &(o, mask) in SHORT_FROM[ti].iter() {
                let x = k.wrapping_sub(o as u8);
                if (mask >> (self.board[x as usize] & 31)) & 1 != 0 {
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

    /// キーと駒割りを盤面と手駒から全部作り直す（初期化と照合に使う）。
    /// 返す値の継続王手と手数の欄は0にする。
    fn state_from_scratch(&self) -> StateInfo {
        let mut st = StateInfo {
            captured: EMPTY,
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
            log_start: self.ray_log.len() as u32,
        };
        let to = m.to();
        let to81 = SQ81[to as usize];
        if m.is_drop() {
            let pt = m.drop_type();
            let pc = make_piece(us, pt);
            debug_assert!(self.hand_count(us, pt) != 0);
            self.hand_raw[ui] -= 1 << HAND_SHIFT[pt as usize];
            self.put(to, pc, 1u128 << to81);
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
            let cap = self.board[to as usize];
            let (bf, bt) = (1u128 << from81, 1u128 << to81);
            self.transfer(from, to, np, bf, bt);
            self.toggle_piece_sets(ui, p, np, cap, bf, bt);
            if cap != EMPTY {
                let hk = piece_type(cap) | 8;
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
                st.dirty.count = 2;
                st.dirty.piece_old[1] = cap;
                st.dirty.piece_new[1] = make_piece(us, hk);
                st.dirty.from[1] = to81;
            } else {
                st.dirty.count = 1;
            }
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
        // 王手の情報。利き数で王手の有無が決まるので、王手のときだけ王手駒を探す。
        // 利き数の読みはここに置き、王手駒を探す関数の呼び出しは王手のときだけにする
        let k = self.king[self.side.index()];
        st.checkers = if self.count[ui][k as usize] == 0 {
            0
        } else {
            self.compute_checkers()
        };
        if st.checkers != 0 {
            st.continuous_check[ui] += 1;
        } else {
            st.continuous_check[ui] = 0;
        }
        self.push_state(st);
        #[cfg(debug_assertions)]
        self.assert_consistent();
    }

    /// 状態を積み、その深さの王手の手がかりを無効にする。
    #[inline]
    pub(crate) fn push_state(&mut self, st: StateInfo) {
        self.states.push(st);
        let n = self.states.len();
        if self.check_slots.len() < n {
            self.check_slots.push(CheckSlot::default());
        } else {
            self.check_slots[n - 1].valid.set(false);
        }
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
            // 短い利きは逆の向きに当て直し、飛び利きは記録から戻す
            let pc = self.board[to as usize];
            if !cfg!(feature = "copy-undo") {
                self.short_effect::<false>(pc, to);
            }
            let b = bit81(to);
            self.board[to as usize] = EMPTY;
            self.occ[ui] ^= b;
            self.toggle_drop_sets(ui, pc, b);
            self.hand_raw[ui] += 1 << HAND_SHIFT[pt as usize];
            if pt == PAWN {
                self.pawn_files[ui] &= !(1 << file_of(to));
            }
        } else {
            let from = m.from();
            let np = self.board[to as usize];
            let p = if m.is_promote() { np + 8 } else { np };
            // 短い利きは逆の向きに当て直し、飛び利きは記録から戻す
            if !cfg!(feature = "copy-undo") {
                self.short_effect::<false>(np, to);
                self.short_effect::<true>(p, from);
                if u.captured != EMPTY {
                    self.short_effect::<true>(u.captured, to);
                }
            }
            let (bf, bt) = (bit81(from), bit81(to));
            self.board[from as usize] = p;
            self.board[to as usize] = u.captured;
            self.occ[ui] ^= bf | bt;
            if u.captured != EMPTY {
                self.occ[them] ^= bt;
            }
            self.toggle_piece_sets(ui, p, np, u.captured, bf, bt);
            if u.captured != EMPTY {
                let hk = piece_type(u.captured) | 8;
                self.hand_raw[ui] -= 1 << HAND_SHIFT[hk as usize];
                if piece_type(u.captured) == PAWN {
                    self.pawn_files[them] |= 1 << file_of(to);
                }
            }
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
            self.ray_log.truncate(u.log_start as usize);
        }
        #[cfg(not(feature = "copy-undo"))]
        self.replay_ray_log(u.log_start as usize);
        #[cfg(debug_assertions)]
        self.assert_consistent();
    }

    /// null moveで1手進める（coreの `do_null_move` と同じ値を積む）。王手されていない局面で呼ぶ。
    pub fn do_null_move(&mut self) {
        debug_assert!(!self.in_check());
        #[cfg(feature = "copy-undo")]
        self.snapshots.push(EffectSnapshot {
            count: self.count,
            longdir: self.longdir,
        });
        let prev = *self.state();
        let mut st = StateInfo {
            captured: EMPTY,
            board_key: prev.board_key ^ zobrist::SIDE,
            checkers: 0,
            plies_from_null: 0,
            dirty: DirtyPiece::default(),
            log_start: self.ray_log.len() as u32,
            ..prev
        };
        // パスなので手番側は王手を掛けていない
        st.continuous_check[self.side.index()] = 0;
        self.side = self.side.flip();
        self.ply += 1;
        // 手番が替わった側の玉に、相手の利きが当たっていることはない（パスの前は手番側が王手されていない）
        let k = self.king[self.side.index()];
        if self.count[self.side.flip().index()][k as usize] != 0 {
            st.checkers = self.compute_checkers();
        }
        self.push_state(st);
    }

    /// `do_null_move` を戻す。
    pub fn undo_null_move(&mut self) {
        self.states
            .pop()
            .expect("undo_null_move without do_null_move");
        #[cfg(feature = "copy-undo")]
        self.snapshots.pop().expect("snapshot stack");
        self.side = self.side.flip();
        self.ply -= 1;
    }

    /// 手数（coreの `game_ply` と同じ）。
    #[inline]
    pub fn game_ply(&self) -> u32 {
        self.ply
    }

    /// 色ごとの利き数を、1枚以上・2枚以上・3枚以上の3枚の81升の集合で返す。
    ///
    /// 評価関数の利き数の入力（ADR-0237）が使う段階の集合で、engineの `EffectLevels` と同じ中身になる。
    /// 配列盤は利き数を升ごとに持つので、駒ごとに利きを数え直さずに読むだけで済む。
    pub fn effect_levels(&self) -> [[u128; 3]; 2] {
        let mut out = [[0u128; 3]; 2];
        for (c, o) in out.iter_mut().enumerate() {
            let cnt = &self.count[c];
            let (mut g0, mut g1, mut g2) = (0u128, 0u128, 0u128);
            for (s81, &i) in IDX81.iter().enumerate() {
                let n = cnt[i as usize];
                g0 |= u128::from(n >= 1) << s81;
                g1 |= u128::from(n >= 2) << s81;
                g2 |= u128::from(n >= 3) << s81;
            }
            *o = [g0, g1, g2];
        }
        out
    }

    /// 81升の利きの語を書く。engineの `effect_code`（ADR-0237のD2）と同じ値になる。
    ///
    /// 上位が駒（0は空き、先手の14種を1〜14、後手を15〜28）、下位4ビットが先手と後手の
    /// 利き数（各2ビット、3で打ち切り）である。配列盤は駒と利き数を升ごとに持つので、
    /// 読んで詰めるだけで済む。1筋の9升は配列の上で続いている。
    pub fn effect_codes(&self, out: &mut [u16; 81]) {
        use std::simd::Simd;
        use std::simd::cmp::SimdOrd;
        use std::simd::num::SimdUint;
        type U8 = Simd<u8, 16>;
        type U16 = Simd<u16, 16>;
        // 1筋の9升を16升ぶん読み、9升だけ使う。最後の筋の書き出しが81升を越えるので、
        // 余白のある置き場へ書いてから写す。壁の升の値は使わない
        let mut tmp = [0u16; 9 * 8 + 16];
        let three = U8::splat(3);
        for f in 0..9u8 {
            let base = crate::board::idx(f, 0) as usize;
            let p = U8::from_slice(&self.board[base..base + 16]);
            let cb = U8::from_slice(&self.count[0][base..base + 16]).simd_min(three);
            let cw = U8::from_slice(&self.count[1][base..base + 16]).simd_min(three);
            // 駒: 空きは0、先手の駒種2〜15を1〜14、後手は14を足す
            let kind = (p & U8::splat(15)).saturating_sub(U8::splat(1))
                + ((p >> U8::splat(4)) & U8::splat(1)) * U8::splat(14);
            let code: U16 = (kind.cast::<u16>() << U16::splat(4))
                | (cb.cast::<u16>() << U16::splat(2))
                | cw.cast::<u16>();
            let at = f as usize * 9;
            tmp[at..at + 16].copy_from_slice(code.as_array());
        }
        out.copy_from_slice(&tmp[..81]);
    }

    /// coreの `Move16` のビット列から手を復元する。coreの `to_move` と同じ検査をし、同じ場面で `None` を返す。
    ///
    /// 復元できても擬似合法とは限らない。`pseudo_legal` で別に確かめる。
    pub fn move_from16(&self, bits: u16) -> Option<Move> {
        let to_i = (bits & 0x7F) as u8;
        let from_i = ((bits >> 7) & 0x7F) as u8;
        if to_i >= 81 {
            return None;
        }
        let to = IDX81[to_i as usize];
        if bits & 0x8000 != 0 {
            if !(GOLD..=ROOK).contains(&from_i) {
                return None;
            }
            Some(Move::drop(from_i, to, self.side))
        } else {
            if from_i >= 81 || from_i == to_i {
                return None;
            }
            let from = IDX81[from_i as usize];
            let pc = self.board[from as usize];
            if pc == EMPTY {
                return None;
            }
            let promote = bits & 0x4000 != 0;
            let after = if promote {
                if !can_promote(piece_type(pc)) {
                    return None;
                }
                pc - 8
            } else {
                pc
            };
            Some(Move::normal(from, to, promote, after))
        }
    }

    /// 不変条件を全部検査する。利きは全計算と、集合と手駒は盤から作り直した値と突き合わせる。
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
        let mut pawn_files = [0u16; 2];
        let mut pieces = 0u32;
        for &i in IDX81.iter() {
            let p = self.board[i as usize];
            if p == EMPTY {
                continue;
            }
            pieces += 1;
            let c = piece_color(p).index();
            if piece_type(p) == KING && self.king[c] != i {
                return Err("玉の升の食い違い".into());
            }
            if piece_type(p) == PAWN {
                pawn_files[c] |= 1 << file_of(i);
            }
        }
        for c in [Color::Black, Color::White] {
            for pt in HAND_KINDS {
                pieces += self.hand_count(c, pt);
            }
        }
        if pieces > 40 {
            return Err(format!("盤上と手駒の駒が40枚を超える（{pieces}枚）"));
        }
        let mut occ = [0u128; 2];
        let mut psets = [[0u128; 16]; 2];
        for &i in IDX81.iter() {
            let p = self.board[i as usize];
            if p != EMPTY {
                occ[piece_color(p).index()] |= bit81(i);
                let pt = piece_type(p);
                psets[piece_color(p).index()][pt as usize] |= bit81(i);
                if pt == GOLD || (2..=5).contains(&pt) {
                    psets[piece_color(p).index()][1] |= bit81(i);
                }
            }
        }
        if occ != self.occ {
            return Err("駒のいる升の集合の食い違い".into());
        }
        if psets != self.psets {
            return Err("駒種ごとの升の集合の食い違い".into());
        }
        if pawn_files != self.pawn_files {
            return Err("二歩の筋の食い違い".into());
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
        let mut pieces = 0u32;
        let mut new_piece = || -> Result<(), SfenError> {
            if pieces >= 40 {
                return Err(SfenError("駒が40枚を超える"));
            }
            pieces += 1;
            Ok(())
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
                    new_piece()?;
                    pos.set_square(i, make_piece(c, pt));
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
        let mut hand = [[0u32; 8]; 2];
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
                        new_piece()?;
                        hand[c][hand_index(pt)] += 1;
                    }
                    n = 0;
                }
            }
        }
        pos.ply = ply_str.parse().map_err(|_| SfenError("手数が数でない"))?;
        let (count, longdir) = pos.effects_from_scratch();
        pos.count = count;
        pos.longdir = longdir;
        for (raw, h) in pos.hand_raw.iter_mut().zip(hand.iter()) {
            for (k, &n) in h.iter().enumerate().skip(1) {
                if n > HAND_MASK[k + 8] {
                    return Err(SfenError("手駒の枚数が駒種の上限を超える"));
                }
                *raw += n << HAND_SHIFT[k + 8];
            }
        }
        let st = pos.state_from_scratch();
        pos.push_state(st);
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
