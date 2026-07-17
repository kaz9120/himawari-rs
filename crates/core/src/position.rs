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

    // ---- do/undo（ADR-0014） ----

    pub fn do_move(&mut self, m: Move) {
        debug_assert!(!m.is_special());
        let us = self.side;
        let them = us.flip();
        let prev = self.state();
        let mut st = StateInfo {
            captured: Piece::EMPTY,
            board_key: prev.board_key,
            hand_key: 0,
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
        pos.states.push(StateInfo {
            board_key,
            hand_key: u64::from(pos.hands[0].0) | (u64::from(pos.hands[1].0) << 32),
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
}

fn err(msg: &str) -> SfenError {
    SfenError(msg.to_string())
}
