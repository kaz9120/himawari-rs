//! 局面（ADR-0014, 0015, 0016, 0018）。
//!
//! make/unmake方式。StateInfoはVecスタックで持ち、do_moveは常に
//! DirtyPiece（NNUE差分の材料）を記録する。SFEN入出力もここに置く。

use crate::attacks::{
    aligned, attacks, between, bishop_attacks, king_attacks, lance_attacks, rook_attacks,
};
use crate::bitboard::Bitboard;
use crate::hand::Hand;
use crate::moves::{Move, Move16, MoveList};
use crate::piece::{Piece, PieceType};
use crate::types::{Color, File, Rank, Square};
use crate::zobrist;

/// 駒の価値（材料点。添字はPieceType）。
const PIECE_VALUE: [i32; 16] = [
    0, 0, 540, 540, 540, 540, 945, 1395, 15000, 540, 90, 315, 405, 495, 855, 990,
];

/// 駒種の材料価値（評価・オーダリングで共用）。
#[inline]
pub const fn piece_value(pt: PieceType) -> i32 {
    PIECE_VALUE[pt.index()]
}

/// 小駒（minor piece）の駒種マスク。香・桂・銀・金とその成駒に限る。
/// 出典はやねうら王 position.cpp:28-36 のminor_piece_table。
const MINOR_PIECE_MASK: u16 = (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5) // と・成香・成桂・成銀
    | (1 << 9) | (1 << 11) | (1 << 12) | (1 << 13); // 金・香・桂・銀

/// 小駒か（ADR-0109）。correction historyのminor系統に使う。
#[inline]
pub const fn is_minor_piece(pt: PieceType) -> bool {
    MINOR_PIECE_MASK & (1 << pt.0) != 0
}

/// 入玉宣言の点数（ADR-0030）。飛角系5点、玉以外の他駒1点。
const fn declaration_points(pt: PieceType) -> u32 {
    match pt {
        PieceType::ROOK | PieceType::BISHOP | PieceType::DRAGON | PieceType::HORSE => 5,
        _ => 1,
    }
}

/// NNUE差分の材料（ADR-0014）。1手で変化する駒は最大2。
#[derive(Copy, Clone, Debug)]
pub struct DirtyPiece {
    pub count: u8,
    pub king_moved: bool,
    pub piece_old: [Piece; 2],
    pub piece_new: [Piece; 2],
    pub from: [Square; 2],
    pub to: [Square; 2],
}

impl Default for DirtyPiece {
    fn default() -> Self {
        DirtyPiece {
            count: 0,
            king_moved: false,
            piece_old: [Piece::EMPTY; 2],
            piece_new: [Piece::EMPTY; 2],
            from: [Square::NONE; 2],
            to: [Square::NONE; 2],
        }
    }
}

/// do_moveの巻き戻し材料と差分計算済みの付随情報（ADR-0014）。
#[derive(Clone, Default, Debug)]
pub struct StateInfo {
    pub captured: Piece,
    pub board_key: u64,
    pub hand_key: u64,
    /// 歩構造キー（盤上の歩＋両者の持ち歩枚数。ADR-0046）。
    pub pawn_key: u64,
    /// 歩以外の盤上の駒のキー（成駒を含む。ADR-0085, 0109）。
    /// correction historyが先後別に引く。持ち駒は含めない
    pub non_pawn_key: [u64; 2],
    /// 小駒のキー（香・桂・銀・金とその成駒。ADR-0109）。
    /// 先後は区別せず1本に混ぜる。出典はやねうら王 position.cpp:144
    pub minor_piece_key: u64,
    pub checkers: Bitboard,
    pub blockers_for_king: [Bitboard; 2],
    pub pinners: [Bitboard; 2],
    pub continuous_check: [u16; 2],
    pub plies_from_null: u16,
    pub material: i32,
    pub dirty: DirtyPiece,
}

#[derive(Debug, PartialEq, Eq)]
pub struct SfenError(pub String);

/// 千日手の分類（ADR-0026）。WinとLoseは連続王手の千日手。
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Repetition {
    None,
    Draw,
    Win,
    Lose,
    Superior,
    Inferior,
}

#[derive(Clone)]
pub struct Position {
    board: [Piece; 81],
    by_type: [Bitboard; 16],
    by_color: [Bitboard; 2],
    hands: [Hand; 2],
    side: Color,
    game_ply: u16,
    king_sq: [Square; 2],
    states: Vec<StateInfo>,
}

pub const SFEN_STARTPOS: &str = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";

impl Position {
    // ---- アクセサ ----

    #[inline]
    pub fn side_to_move(&self) -> Color {
        self.side
    }

    #[inline]
    pub fn game_ply(&self) -> u16 {
        self.game_ply
    }

    #[inline]
    pub fn piece_on(&self, sq: Square) -> Piece {
        self.board[sq.index()]
    }

    #[inline]
    pub fn hand(&self, c: Color) -> Hand {
        self.hands[c.index()]
    }

    #[inline]
    pub fn king(&self, c: Color) -> Square {
        self.king_sq[c.index()]
    }

    #[inline]
    pub fn occupied(&self) -> Bitboard {
        self.by_color[0] | self.by_color[1]
    }

    #[inline]
    pub fn color_bb(&self, c: Color) -> Bitboard {
        self.by_color[c.index()]
    }

    #[inline]
    pub fn pieces(&self, c: Color, pt: PieceType) -> Bitboard {
        self.by_type[pt.index()] & self.by_color[c.index()]
    }

    /// 金の動きをする駒（金＋成金4種）。
    #[inline]
    pub fn golds(&self, c: Color) -> Bitboard {
        (self.by_type[PieceType::GOLD.index()]
            | self.by_type[PieceType::PRO_PAWN.index()]
            | self.by_type[PieceType::PRO_LANCE.index()]
            | self.by_type[PieceType::PRO_KNIGHT.index()]
            | self.by_type[PieceType::PRO_SILVER.index()])
            & self.by_color[c.index()]
    }

    #[inline]
    pub fn state(&self) -> &StateInfo {
        self.states.last().expect("state stack is never empty")
    }

    #[inline]
    pub fn checkers(&self) -> Bitboard {
        self.state().checkers
    }

    #[inline]
    pub fn in_check(&self) -> bool {
        !self.checkers().is_empty()
    }

    /// 置換表用の合成キー（ADR-0015）。
    #[inline]
    pub fn key(&self) -> u64 {
        let st = self.state();
        st.board_key ^ st.hand_key.wrapping_mul(zobrist::HAND_MIX)
    }

    /// 歩構造キー（ADR-0046）。correction historyのテーブル引きに使う。
    #[inline]
    pub fn pawn_key(&self) -> u64 {
        self.state().pawn_key
    }

    /// 色cの歩以外の盤上駒のキー（ADR-0085, 0109）。correction historyに使う。
    #[inline]
    pub fn non_pawn_key(&self, c: Color) -> u64 {
        self.state().non_pawn_key[c.index()]
    }

    /// 小駒のキー（ADR-0109）。correction historyのminor系統に使う。
    #[inline]
    pub fn minor_piece_key(&self) -> u64 {
        self.state().minor_piece_key
    }

    /// 色cの歩以外の盤上駒キーの全計算。と金など成歩はこちらに入る。
    /// 持ち駒は含めない。差分更新の検証にも使う。
    pub fn compute_non_pawn_key(&self, c: Color) -> u64 {
        let mut key = 0u64;
        for i in 0..Square::NB {
            let sq = Square::from_index(i as u8);
            let pc = self.board[sq.index()];
            if !pc.is_empty() && pc.piece_type() != PieceType::PAWN && pc.color() == c {
                key ^= zobrist::psq(pc, sq);
            }
        }
        key
    }

    /// 小駒キーの全計算。差分更新の検証にも使う。
    pub fn compute_minor_piece_key(&self) -> u64 {
        let mut key = 0u64;
        for i in 0..Square::NB {
            let sq = Square::from_index(i as u8);
            let pc = self.board[sq.index()];
            if !pc.is_empty() && is_minor_piece(pc.piece_type()) {
                key ^= zobrist::psq(pc, sq);
            }
        }
        key
    }

    /// 歩構造キーの全計算（盤上の歩＋両者の持ち歩枚数）。
    /// と金など成歩は含めない。差分更新の検証にも使う。
    pub fn compute_pawn_key(&self) -> u64 {
        let mut key = 0u64;
        for c in [Color::Black, Color::White] {
            for sq in self.pieces(c, PieceType::PAWN) {
                key ^= zobrist::psq(Piece::new(c, PieceType::PAWN), sq);
            }
            key ^= zobrist::hand_pawn(c, self.hands[c.index()].count(PieceType::PAWN));
        }
        key
    }

    // ---- 盤面操作（bitboard/board配列のみ。キーは呼び出し側で管理） ----

    fn put_piece(&mut self, sq: Square, pc: Piece) {
        debug_assert!(self.board[sq.index()].is_empty());
        self.board[sq.index()] = pc;
        self.by_type[pc.piece_type().index()].set(sq);
        self.by_color[pc.color().index()].set(sq);
    }

    fn remove_piece(&mut self, sq: Square) -> Piece {
        let pc = self.board[sq.index()];
        debug_assert!(!pc.is_empty());
        self.board[sq.index()] = Piece::EMPTY;
        self.by_type[pc.piece_type().index()].clear(sq);
        self.by_color[pc.color().index()].clear(sq);
        pc
    }

    // ---- 利き・王手情報 ----

    /// 色cの駒でsqに利いているものの集合。
    pub fn attackers_to(&self, c: Color, sq: Square, occ: Bitboard) -> Bitboard {
        use crate::attacks::{gold_attacks, knight_attacks, pawn_attacks, silver_attacks};
        let e = c.flip();
        let horses = self.by_type[PieceType::HORSE.index()];
        let dragons = self.by_type[PieceType::DRAGON.index()];
        let mut a = pawn_attacks(e, sq) & self.pieces(c, PieceType::PAWN);
        a |= knight_attacks(e, sq) & self.pieces(c, PieceType::KNIGHT);
        a |= silver_attacks(e, sq) & self.pieces(c, PieceType::SILVER);
        a |= gold_attacks(e, sq) & self.golds(c);
        a |= king_attacks(sq)
            & (self.by_type[PieceType::KING.index()] | horses | dragons)
            & self.by_color[c.index()];
        a |= lance_attacks(e, sq, occ) & self.pieces(c, PieceType::LANCE);
        a |= bishop_attacks(sq, occ)
            & (self.by_type[PieceType::BISHOP.index()] | horses)
            & self.by_color[c.index()];
        a |= rook_attacks(sq, occ)
            & (self.by_type[PieceType::ROOK.index()] | dragons)
            & self.by_color[c.index()];
        a
    }

    /// 色cの玉に対するblockers（両色の駒）とpinners（敵の遠隔駒）。
    fn slider_blockers(&self, c: Color) -> (Bitboard, Bitboard) {
        let ksq = self.king_sq[c.index()];
        let e = c.flip();
        let horses = self.by_type[PieceType::HORSE.index()];
        let dragons = self.by_type[PieceType::DRAGON.index()];
        let empty = Bitboard::EMPTY;
        let mut snipers = rook_attacks(ksq, empty)
            & (self.by_type[PieceType::ROOK.index()] | dragons)
            & self.by_color[e.index()];
        snipers |= bishop_attacks(ksq, empty)
            & (self.by_type[PieceType::BISHOP.index()] | horses)
            & self.by_color[e.index()];
        snipers |= lance_attacks(c, ksq, empty) & self.pieces(e, PieceType::LANCE);
        let occ = self.occupied();
        let mut blockers = Bitboard::EMPTY;
        let mut pinners = Bitboard::EMPTY;
        for s in snipers {
            let b = between(ksq, s) & occ;
            if !b.is_empty() && !b.more_than_one() {
                blockers |= b;
                pinners.set(s);
            }
        }
        (blockers, pinners)
    }

    fn update_check_info(&mut self) {
        let us = self.side;
        let them = us.flip();
        let occ = self.occupied();
        let checkers = self.attackers_to(them, self.king_sq[us.index()], occ);
        let (b0, p0) = self.slider_blockers(Color::Black);
        let (b1, p1) = self.slider_blockers(Color::White);
        let st = self.states.last_mut().expect("state stack is never empty");
        st.checkers = checkers;
        st.blockers_for_king = [b0, b1];
        st.pinners = [p0, p1];
    }

    // ---- 合法性（ADR-0016） ----

    /// 擬似合法手に対するO(1)の合法性検査。
    pub fn is_legal(&self, m: Move) -> bool {
        let us = self.side;
        let them = us.flip();
        if m.is_drop() {
            // 歩打ちのみ打ち歩詰め検査。他の駒打ちは常に合法
            if m.drop_piece_type() == PieceType::PAWN {
                let to = m.to();
                if crate::attacks::pawn_attacks(us, to).test(self.king_sq[them.index()]) {
                    return !self.is_pawn_drop_mate(to);
                }
            }
            return true;
        }
        let from = m.from_sq();
        let to = m.to();
        if m.piece_before().piece_type() == PieceType::KING {
            let occ_without_king = self.occupied() ^ Bitboard::from_square(from);
            return self.attackers_to(them, to, occ_without_king).is_empty();
        }
        if self.state().blockers_for_king[us.index()].test(from) {
            return aligned(from, to, self.king_sq[us.index()]);
        }
        true
    }

    /// toへの歩打ちが打ち歩詰めか（toへの歩打ちが王手である前提）。
    fn is_pawn_drop_mate(&self, to: Square) -> bool {
        let us = self.side;
        let them = us.flip();
        let ksq = self.king_sq[them.index()];
        let to_bb = Bitboard::from_square(to);
        let occ2 = self.occupied() | to_bb;

        // 1. 玉以外の駒が歩を取れるか（取った後に王手が残らないこと）
        let defenders = self.attackers_to(them, to, occ2) & !self.by_type[PieceType::KING.index()];
        for d in defenders {
            let occ_after = (occ2 ^ Bitboard::from_square(d)) | to_bb;
            // 打った歩はbitboardに未反映のため、usの利きはoccupancyだけで正しく出る
            if self.attackers_to(us, ksq, occ_after).is_empty() {
                return false;
            }
        }

        // 2. 玉が逃げられるか（歩を取る移動も含む）
        let king_moves = king_attacks(ksq) & !self.by_color[them.index()];
        let occ_no_king = occ2 ^ Bitboard::from_square(ksq);
        for s in king_moves {
            let occ_after = occ_no_king | Bitboard::from_square(s);
            if self.attackers_to(us, s, occ_after).is_empty() {
                return false;
            }
        }
        true
    }

    /// mが相手玉への王手になるか。
    pub fn gives_check(&self, m: Move) -> bool {
        let us = self.side;
        let them = us.flip();
        let ksq = self.king_sq[them.index()];
        let to = m.to();
        if m.is_drop() {
            let occ = self.occupied() | Bitboard::from_square(to);
            return attacks(m.piece_after(), to, occ).test(ksq);
        }
        let from = m.from_sq();
        let occ = (self.occupied() ^ Bitboard::from_square(from)) | Bitboard::from_square(to);
        // 直接王手
        if attacks(m.piece_after(), to, occ).test(ksq) {
            return true;
        }
        // 開き王手
        self.state().blockers_for_king[them.index()].test(from) && !aligned(from, to, ksq)
    }

    /// 駒種ptの手番側の駒がそこへ動くと相手玉に直接王手になるマスの集合。
    /// 開き王手は含めない。指し手オーダリングの王手ボーナスに使う
    /// （ADR-0109。出典はやねうら王のPosition::check_squares）。
    #[inline]
    pub fn check_squares(&self, pt: PieceType) -> Bitboard {
        let them = self.side.flip();
        let ksq = self.king_sq[them.index()];
        // 相手の駒として玉の位置から利きを引くと、逆向きの利きの集合になる
        attacks(Piece::new(them, pt), ksq, self.occupied())
    }

    // ---- do/undo（ADR-0014） ----

    /// 盤上の駒1つを部分キーへXORする（ADR-0109）。
    /// 歩はpawn_key、それ以外はnon_pawn_key[色]へ入り、
    /// 小駒はminor_piece_keyにも入る。
    /// 出典はやねうら王 position.cpp:138-148。
    #[inline]
    fn xor_partial_keys(st: &mut StateInfo, pc: Piece, sq: Square) {
        let k = zobrist::psq(pc, sq);
        if pc.piece_type() == PieceType::PAWN {
            st.pawn_key ^= k;
        } else {
            if is_minor_piece(pc.piece_type()) {
                st.minor_piece_key ^= k;
            }
            st.non_pawn_key[pc.color().index()] ^= k;
        }
    }

    pub fn do_move(&mut self, m: Move) {
        debug_assert!(!m.is_special());
        let us = self.side;
        let them = us.flip();
        let prev = self.state();
        let mut st = StateInfo {
            captured: Piece::EMPTY,
            board_key: prev.board_key,
            hand_key: 0,
            pawn_key: prev.pawn_key,
            non_pawn_key: prev.non_pawn_key,
            minor_piece_key: prev.minor_piece_key,
            checkers: Bitboard::EMPTY,
            blockers_for_king: [Bitboard::EMPTY; 2],
            pinners: [Bitboard::EMPTY; 2],
            continuous_check: prev.continuous_check,
            plies_from_null: prev.plies_from_null + 1,
            material: prev.material,
            dirty: DirtyPiece::default(),
        };
        let sign = if us == Color::Black { 1 } else { -1 };

        if m.is_drop() {
            let pt = m.drop_piece_type();
            let pc = m.piece_after();
            self.hands[us.index()].sub(pt);
            self.put_piece(m.to(), pc);
            st.board_key ^= zobrist::psq(pc, m.to());
            if pt == PieceType::PAWN {
                // 持ち歩-1（盤上歩の追加はxor_partial_keysが行う）
                let new = self.hands[us.index()].count(PieceType::PAWN);
                st.pawn_key ^= zobrist::hand_pawn(us, new + 1) ^ zobrist::hand_pawn(us, new);
            }
            Self::xor_partial_keys(&mut st, pc, m.to());
            st.dirty.count = 1;
            st.dirty.piece_old[0] = Piece::EMPTY;
            st.dirty.piece_new[0] = pc;
            st.dirty.from[0] = Square::NONE;
            st.dirty.to[0] = m.to();
        } else {
            let from = m.from_sq();
            let to = m.to();
            let moved = self.remove_piece(from);
            debug_assert!(moved == m.piece_before());
            let captured = self.board[to.index()];
            if !captured.is_empty() {
                self.remove_piece(to);
                let hand_kind = captured.piece_type().unpromote();
                self.hands[us.index()].add(hand_kind);
                st.board_key ^= zobrist::psq(captured, to);
                // 盤上から取った駒を除去。取った歩・と金は持ち歩+1
                Self::xor_partial_keys(&mut st, captured, to);
                if hand_kind == PieceType::PAWN {
                    let new = self.hands[us.index()].count(PieceType::PAWN);
                    st.pawn_key ^= zobrist::hand_pawn(us, new - 1) ^ zobrist::hand_pawn(us, new);
                }
                st.material += sign
                    * (PIECE_VALUE[captured.piece_type().index()] + PIECE_VALUE[hand_kind.index()]);
                st.captured = captured;
                st.dirty.count = 2;
                st.dirty.piece_old[1] = captured;
                st.dirty.piece_new[1] = Piece::new(us, hand_kind);
                st.dirty.from[1] = to;
                st.dirty.to[1] = Square::NONE;
            } else {
                st.dirty.count = 1;
            }
            let placed = m.piece_after();
            self.put_piece(to, placed);
            st.board_key ^= zobrist::psq(moved, from) ^ zobrist::psq(placed, to);
            // fromの駒を除去し、toへ移動後の駒を追加する。
            // 成った歩（と金）は歩の側から抜けて歩以外の側へ入る
            Self::xor_partial_keys(&mut st, moved, from);
            Self::xor_partial_keys(&mut st, placed, to);
            if m.is_promote() {
                st.material += sign
                    * (PIECE_VALUE[placed.piece_type().index()]
                        - PIECE_VALUE[moved.piece_type().index()]);
            }
            if moved.piece_type() == PieceType::KING {
                self.king_sq[us.index()] = to;
                st.dirty.king_moved = true;
            }
            st.dirty.piece_old[0] = moved;
            st.dirty.piece_new[0] = placed;
            st.dirty.from[0] = from;
            st.dirty.to[0] = to;
        }

        st.board_key ^= zobrist::SIDE;
        st.hand_key = u64::from(self.hands[0].0) | (u64::from(self.hands[1].0) << 32);
        self.side = them;
        self.game_ply += 1;
        self.states.push(st);
        self.update_check_info();

        // 連続王手カウンタ（指した側が王手を掛けたか）
        let checked = self.in_check();
        let st = self.states.last_mut().expect("just pushed");
        if checked {
            st.continuous_check[us.index()] += 1;
        } else {
            st.continuous_check[us.index()] = 0;
        }
    }

    pub fn undo_move(&mut self, m: Move) {
        let st = self.states.pop().expect("undo without do");
        let them = self.side;
        let us = them.flip();
        self.side = us;
        self.game_ply -= 1;
        if m.is_drop() {
            self.remove_piece(m.to());
            self.hands[us.index()].add(m.drop_piece_type());
        } else {
            let to = m.to();
            self.remove_piece(to);
            self.put_piece(m.from_sq(), m.piece_before());
            if m.piece_before().piece_type() == PieceType::KING {
                self.king_sq[us.index()] = m.from_sq();
            }
            if !st.captured.is_empty() {
                self.put_piece(to, st.captured);
                self.hands[us.index()].sub(st.captured.piece_type().unpromote());
            }
        }
    }

    /// 入玉宣言勝ち（27点法、ADR-0030）が成立するか。
    /// 条件: 手番側の玉が敵陣三段目以内、王手されていない、
    /// 敵陣内の玉以外の駒が10枚以上、点数（飛角馬龍5点・他1点、
    /// 持ち駒含む）が先手28点以上・後手27点以上。
    pub fn can_declare_win(&self) -> bool {
        let us = self.side;
        let zone = Bitboard::promotion_zone(us);
        if !zone.test(self.king_sq[us.index()]) || self.in_check() {
            return false;
        }
        let mut count = 0u32;
        let mut points = 0u32;
        for sq in self.by_color[us.index()] & zone {
            let pt = self.board[sq.index()].piece_type();
            if pt == PieceType::KING {
                continue;
            }
            count += 1;
            points += declaration_points(pt);
        }
        if count < 10 {
            return false;
        }
        let hand = self.hands[us.index()];
        for pt in PieceType::HAND_KINDS {
            points += hand.count(pt) * declaration_points(pt);
        }
        let need = if us == Color::Black { 28 } else { 27 };
        points >= need
    }

    /// パス（null move。ADR-0028のNMP用）。王手中は呼べない。
    /// `plies_from_null = 0` により千日手走査はここで遮断される。
    pub fn do_null_move(&mut self) {
        debug_assert!(!self.in_check());
        let prev = self.state();
        let mut st = StateInfo {
            captured: Piece::EMPTY,
            board_key: prev.board_key ^ zobrist::SIDE,
            hand_key: prev.hand_key,
            pawn_key: prev.pawn_key,
            non_pawn_key: prev.non_pawn_key,
            minor_piece_key: prev.minor_piece_key,
            checkers: Bitboard::EMPTY,
            blockers_for_king: [Bitboard::EMPTY; 2],
            pinners: [Bitboard::EMPTY; 2],
            continuous_check: prev.continuous_check,
            plies_from_null: 0,
            material: prev.material,
            dirty: DirtyPiece::default(),
        };
        // パスなので手番側は王手を掛けていない
        st.continuous_check[self.side.index()] = 0;
        self.side = self.side.flip();
        self.game_ply += 1;
        self.states.push(st);
        self.update_check_info();
    }

    pub fn undo_null_move(&mut self) {
        self.states.pop().expect("undo without do");
        self.side = self.side.flip();
        self.game_ply -= 1;
    }

    // ---- SFEN（ADR-0018） ----

    pub fn from_sfen(sfen: &str) -> Result<Position, SfenError> {
        let mut tokens = sfen.split_whitespace();
        let board_str = tokens.next().ok_or_else(|| err("盤面がない"))?;
        let side_str = tokens.next().ok_or_else(|| err("手番がない"))?;
        let hands_str = tokens.next().ok_or_else(|| err("手駒がない"))?;
        let ply_str = tokens.next().unwrap_or("1");

        let mut pos = Position {
            board: [Piece::EMPTY; 81],
            by_type: [Bitboard::EMPTY; 16],
            by_color: [Bitboard::EMPTY; 2],
            hands: [Hand::EMPTY; 2],
            side: Color::Black,
            game_ply: 1,
            king_sq: [Square::NONE; 2],
            states: Vec::with_capacity(1024),
        };

        // 盤面: 9a側（左上）から段ごとに走査
        let ranks: Vec<&str> = board_str.split('/').collect();
        if ranks.len() != 9 {
            return Err(err("段の数が9でない"));
        }
        for (r, rank_str) in ranks.iter().enumerate() {
            let mut f: i32 = 8;
            let mut promote = false;
            for ch in rank_str.chars() {
                if let Some(d) = ch.to_digit(10) {
                    if promote {
                        return Err(err("+の後に数字"));
                    }
                    f -= d as i32;
                } else if ch == '+' {
                    promote = true;
                } else {
                    if f < 0 {
                        return Err(err("筋があふれた"));
                    }
                    let pt = PieceType::from_sfen_char(ch.to_ascii_uppercase())
                        .ok_or_else(|| err("不明な駒文字"))?;
                    let pt = if promote {
                        if !pt.can_promote() {
                            return Err(err("成れない駒に+"));
                        }
                        pt.promote()
                    } else {
                        pt
                    };
                    promote = false;
                    let c = if ch.is_ascii_uppercase() {
                        Color::Black
                    } else {
                        Color::White
                    };
                    let sq = Square::new(File(f as u8), Rank(r as u8));
                    pos.put_piece(sq, Piece::new(c, pt));
                    f -= 1;
                }
            }
            if f != -1 {
                return Err(err("筋の数が9でない"));
            }
        }

        pos.side = match side_str {
            "b" => Color::Black,
            "w" => Color::White,
            _ => return Err(err("手番はbかw")),
        };

        // 手駒
        if hands_str != "-" {
            let mut n = 0u32;
            for ch in hands_str.chars() {
                if let Some(d) = ch.to_digit(10) {
                    n = n * 10 + d;
                    if n > 18 {
                        return Err(err("手駒の枚数が過大"));
                    }
                } else {
                    let pt = PieceType::from_sfen_char(ch.to_ascii_uppercase())
                        .ok_or_else(|| err("不明な手駒文字"))?;
                    if pt == PieceType::KING {
                        return Err(err("玉は手駒にできない"));
                    }
                    let c = if ch.is_ascii_uppercase() {
                        Color::Black
                    } else {
                        Color::White
                    };
                    for _ in 0..n.max(1) {
                        pos.hands[c.index()].add(pt);
                    }
                    n = 0;
                }
            }
            if n != 0 {
                return Err(err("手駒の末尾に数字"));
            }
        }

        pos.game_ply = ply_str.parse().map_err(|_| err("手数が数値でない"))?;

        pos.validate()?;
        pos.king_sq = [
            pos.pieces(Color::Black, PieceType::KING).lsb(),
            pos.pieces(Color::White, PieceType::KING).lsb(),
        ];

        // 手番でない側の玉に王手が掛かっている局面は不正
        let opp = pos.side.flip();
        if !pos
            .attackers_to(pos.side, pos.king_sq[opp.index()], pos.occupied())
            .is_empty()
        {
            return Err(err("手番でない側の玉に王手が掛かっている"));
        }

        // 初期StateInfoを構築
        let mut board_key = 0u64;
        let mut material = 0i32;
        for i in 0..81 {
            let pc = pos.board[i];
            if !pc.is_empty() {
                board_key ^= zobrist::psq(pc, Square::from_index(i as u8));
                let sign = if pc.color() == Color::Black { 1 } else { -1 };
                material += sign * PIECE_VALUE[pc.piece_type().index()];
            }
        }
        if pos.side == Color::White {
            board_key ^= zobrist::SIDE;
        }
        for c in [Color::Black, Color::White] {
            let sign = if c == Color::Black { 1 } else { -1 };
            for pt in PieceType::HAND_KINDS {
                material += sign * PIECE_VALUE[pt.index()] * pos.hands[c.index()].count(pt) as i32;
            }
        }
        let pawn_key = pos.compute_pawn_key();
        let non_pawn_key = [
            pos.compute_non_pawn_key(Color::Black),
            pos.compute_non_pawn_key(Color::White),
        ];
        let minor_piece_key = pos.compute_minor_piece_key();
        pos.states.push(StateInfo {
            board_key,
            hand_key: u64::from(pos.hands[0].0) | (u64::from(pos.hands[1].0) << 32),
            pawn_key,
            non_pawn_key,
            minor_piece_key,
            material,
            ..StateInfo::default()
        });
        pos.update_check_info();
        Ok(pos)
    }

    /// 局面の静的検証（駒数上限・二歩・行き所のない駒・玉の数）。
    fn validate(&self) -> Result<(), SfenError> {
        // 玉は各側ちょうど1枚
        for c in [Color::Black, Color::White] {
            if self.pieces(c, PieceType::KING).count() != 1 {
                return Err(err("玉は各側1枚"));
            }
        }
        // 駒数上限（生駒に戻した種別で数える）
        let limits = [
            (PieceType::PAWN, 18),
            (PieceType::LANCE, 4),
            (PieceType::KNIGHT, 4),
            (PieceType::SILVER, 4),
            (PieceType::GOLD, 4),
            (PieceType::BISHOP, 2),
            (PieceType::ROOK, 2),
        ];
        for (kind, limit) in limits {
            let mut n = 0u32;
            for i in 0..81 {
                let pc = self.board[i];
                if !pc.is_empty()
                    && pc.piece_type() != PieceType::KING
                    && pc.piece_type().unpromote() == kind
                {
                    n += 1;
                }
            }
            n += self.hands[0].count(kind) + self.hands[1].count(kind);
            if n > limit {
                return Err(err("駒数が上限を超えている"));
            }
        }
        for c in [Color::Black, Color::White] {
            // 二歩
            for f in 0..9 {
                if (self.pieces(c, PieceType::PAWN) & Bitboard::file(File(f))).count() > 1 {
                    return Err(err("二歩"));
                }
            }
            // 行き所のない駒
            let r1 = Bitboard::rank(Rank(0).relative(c));
            let r2 = Bitboard::rank(Rank(1).relative(c));
            if !((self.pieces(c, PieceType::PAWN) | self.pieces(c, PieceType::LANCE)) & r1)
                .is_empty()
            {
                return Err(err("行き所のない歩・香"));
            }
            if !(self.pieces(c, PieceType::KNIGHT) & (r1 | r2)).is_empty() {
                return Err(err("行き所のない桂"));
            }
        }
        Ok(())
    }

    pub fn to_sfen(&self) -> String {
        let mut s = String::new();
        for r in 0..9 {
            let mut empty = 0;
            for f in (0..9).rev() {
                let pc = self.piece_on(Square::new(File(f), Rank(r)));
                if pc.is_empty() {
                    empty += 1;
                } else {
                    if empty > 0 {
                        s.push_str(&empty.to_string());
                        empty = 0;
                    }
                    let pt_str = pc.piece_type().to_sfen().expect("valid piece");
                    if pc.color() == Color::Black {
                        s.push_str(pt_str);
                    } else {
                        s.push_str(&pt_str.to_lowercase());
                    }
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
        if self.hands[0].is_empty() && self.hands[1].is_empty() {
            s.push('-');
        } else {
            // 慣例順: 飛角金銀桂香歩、先手→後手
            let order = [
                PieceType::ROOK,
                PieceType::BISHOP,
                PieceType::GOLD,
                PieceType::SILVER,
                PieceType::KNIGHT,
                PieceType::LANCE,
                PieceType::PAWN,
            ];
            for c in [Color::Black, Color::White] {
                for pt in order {
                    let n = self.hands[c.index()].count(pt);
                    if n == 0 {
                        continue;
                    }
                    if n > 1 {
                        s.push_str(&n.to_string());
                    }
                    let pt_str = pt.to_sfen().expect("hand piece");
                    if c == Color::Black {
                        s.push_str(pt_str);
                    } else {
                        s.push_str(&pt_str.to_lowercase());
                    }
                }
            }
        }
        s.push(' ');
        s.push_str(&self.game_ply.to_string());
        s
    }

    /// USI表記の指し手を現局面の合法手と照合して復元する。
    pub fn move_from_usi(&self, s: &str) -> Option<Move> {
        let m16 = Move16::from_usi(s)?;
        let mut list = MoveList::default();
        crate::movegen::generate_legal(self, true, &mut list);
        list.as_slice()
            .iter()
            .copied()
            .find(|m| m.to_move16() == m16)
    }

    // ---- 千日手・優等局面（ADR-0026） ----

    /// 現局面の千日手状態。StateInfoスタックを2plyごとに遡って判定する。
    pub fn repetition_state(&self) -> Repetition {
        let cur = self.states.len() - 1;
        let st = &self.states[cur];
        let us = self.side.index();
        let them = 1 - us;
        let limit = (st.plies_from_null as usize).min(cur);
        let mut i = 4;
        while i <= limit {
            let prev = &self.states[cur - i];
            if prev.board_key == st.board_key {
                if prev.hand_key == st.hand_key {
                    // 連続王手の千日手（相手優先で判定）
                    if st.continuous_check[them] as usize * 2 >= i {
                        return Repetition::Win;
                    }
                    if st.continuous_check[us] as usize * 2 >= i {
                        return Repetition::Lose;
                    }
                    return Repetition::Draw;
                }
                let my_now = Hand((st.hand_key >> (32 * us)) as u32);
                let my_then = Hand((prev.hand_key >> (32 * us)) as u32);
                if my_now.is_superior_or_equal(my_then) {
                    return Repetition::Superior;
                }
                if my_then.is_superior_or_equal(my_now) {
                    return Repetition::Inferior;
                }
                // 交換が混在する場合は判定不能なので走査を続ける
            }
            i += 2;
        }
        Repetition::None
    }

    // ---- 擬似合法性（置換表由来の指し手の検査。ADR-0025） ----

    /// mがこの局面の擬似合法手か（is_legalの前提を満たすか）を検査する。
    pub fn pseudo_legal(&self, m: Move) -> bool {
        if m.is_special() {
            return false;
        }
        let us = self.side;
        let to = m.to();
        if to.is_none() || !m.piece_after().is_empty() && m.piece_after().color() != us {
            return false;
        }
        if m.is_drop() {
            let pt = m.drop_piece_type();
            if pt.0 < 9 || !self.hands[us.index()].has(pt) || !self.piece_on(to).is_empty() {
                return false;
            }
            let rel = to.rank().relative(us).0;
            let placement_ok = match pt {
                PieceType::PAWN => {
                    rel >= 1
                        && (self.pieces(us, PieceType::PAWN) & Bitboard::file(to.file())).is_empty()
                }
                PieceType::LANCE => rel >= 1,
                PieceType::KNIGHT => rel >= 2,
                _ => true,
            };
            if !placement_ok {
                return false;
            }
            // 王手中の駒打ちは合い駒のみ
            if self.in_check() {
                let checkers = self.checkers();
                if checkers.more_than_one()
                    || !between(self.king_sq[us.index()], checkers.lsb()).test(to)
                {
                    return false;
                }
            }
            true
        } else {
            let from = m.from_sq();
            let pc = self.piece_on(from);
            if pc != m.piece_before() || pc.is_empty() || pc.color() != us {
                return false;
            }
            if self.color_bb(us).test(to) || !attacks(pc, from, self.occupied()).test(to) {
                return false;
            }
            let pt = pc.piece_type();
            let zone = Bitboard::promotion_zone(us);
            let rel = to.rank().relative(us).0;
            if m.is_promote() {
                if !pt.can_promote() || !(zone.test(from) || zone.test(to)) {
                    return false;
                }
            } else {
                // 行き所のない駒になる不成は非合法
                let ok = match pt {
                    PieceType::PAWN | PieceType::LANCE => rel >= 1,
                    PieceType::KNIGHT => rel >= 2,
                    _ => true,
                };
                if !ok {
                    return false;
                }
            }
            // 王手中は玉移動か、単王手への合い駒・王手駒の捕獲のみ
            if self.in_check() && pt != PieceType::KING {
                let checkers = self.checkers();
                if checkers.more_than_one() {
                    return false;
                }
                let checker = checkers.lsb();
                if to != checker && !between(self.king_sq[us.index()], checker).test(to) {
                    return false;
                }
            }
            true
        }
    }

    // ---- SEE（静的交換評価。ADR-0025） ----

    /// mの静的交換評価がthreshold以上か。Stockfishのsee_geと同じswap構造。
    /// 駒打ちも解き（ADR-0091）、初手の成りも扱う（ADR-0095）。
    /// 交換の途中で相手が成る筋は見ない簡略版のままである。
    pub fn see_ge(&self, m: Move, threshold: i32) -> bool {
        let to = m.to();
        // 打つ手は移動先が空きマスなので取る駒がない。打った駒が盤上へ
        // 現れるぶん、飛び駒の利きはそこで止まる（occへtoを足す）。
        // 移動する手はfromが空くのでoccから抜く
        let (captured, placed, occ0) = if m.is_drop() {
            (
                0,
                PIECE_VALUE[m.drop_piece_type().index()],
                self.occupied() | Bitboard::from_square(to),
            )
        } else {
            let from = m.from_sq();
            // 成ると駒の価値が上がる（歩90→と金540など）。その差は取り分で
            // あり、取り返されるのは成ったあとの駒である（ADR-0095）
            let before = PIECE_VALUE[self.piece_on(from).piece_type().index()];
            let after = PIECE_VALUE[m.piece_after().piece_type().index()];
            (
                PIECE_VALUE[self.piece_on(to).piece_type().index()] + after - before,
                after,
                self.occupied() ^ Bitboard::from_square(from),
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

        let mut occ = occ0;
        let mut stm = self.side;
        let mut res = true;
        loop {
            stm = stm.flip();
            let stm_attackers = self.attackers_to(stm, to, occ) & occ;
            if stm_attackers.is_empty() {
                break;
            }
            res = !res;
            // 最も安い攻撃駒を選ぶ
            let mut best = Square::NONE;
            let mut best_val = i32::MAX;
            for sq in stm_attackers {
                let v = PIECE_VALUE[self.piece_on(sq).piece_type().index()];
                if v < best_val {
                    best_val = v;
                    best = sq;
                }
            }
            if self.piece_on(best).piece_type() == PieceType::KING {
                // 相手の攻撃が残っていれば玉では取れず、結果は直前のまま
                let occ2 = occ ^ Bitboard::from_square(best);
                if !(self.attackers_to(stm.flip(), to, occ2) & occ2).is_empty() {
                    res = !res;
                }
                break;
            }
            swap = best_val - swap;
            if swap < i32::from(res) {
                break;
            }
            occ ^= Bitboard::from_square(best);
        }
        res
    }

    /// Move16を盤面情報で完全なMoveに復元する（ADR-0012）。
    /// 置換表由来の任意ビット列に耐えるよう全フィールドを検証する。
    /// 復元できても擬似合法とは限らない。pseudo_legalで別途検査する。
    pub fn to_move(&self, m16: Move16) -> Option<Move> {
        let bits = m16.0;
        let to_i = (bits & 0x7F) as u8;
        let from_i = ((bits >> 7) & 0x7F) as u8;
        if to_i >= 81 {
            return None;
        }
        let to = Square::from_index(to_i);
        if bits & 0x8000 != 0 {
            // 駒打ち。fromフィールドは駒種（9〜15）
            if !(9..=15).contains(&from_i) {
                return None;
            }
            Some(Move::new_drop(PieceType(from_i), to, self.side))
        } else {
            if from_i >= 81 || from_i == to_i {
                return None;
            }
            let from = Square::from_index(from_i);
            let pc = self.piece_on(from);
            if pc.is_empty() {
                return None;
            }
            let promote = bits & 0x4000 != 0;
            let after = if promote {
                if !pc.piece_type().can_promote() {
                    return None;
                }
                pc.promote()
            } else {
                pc
            };
            Some(Move::new_move(from, to, promote, after))
        }
    }
}

fn err(msg: &str) -> SfenError {
    SfenError(msg.to_string())
}
