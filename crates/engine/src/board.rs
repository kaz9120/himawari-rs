//! 探索が手を進め戻す盤面（ADR-0239の手順0）。
//!
//! 探索のワーカーと、盤面操作だけを測る `boardbench` が同じ進め戻しを
//! 呼ぶために1か所へ集める。利きの表のように局面に付いて回る状態を
//! 足すときは、この型に持たせ、`do_move` などへ追従を書く。そうすれば
//! 探索とミクロな計測の両方が同じ費用を払う。
//!
//! 利き数のバイト表（ADR-0239の案B）をここで遅延して維持する（ADR-0240）。局面の読み取りは
//! `Deref` で `Position` のメソッドをそのまま呼べる。進め戻しはこの型の同名のメソッドが先に
//! 解決されるので、包んだ側を素通りしない。`DerefMut` は実装しない。
//! 局面を可変で借りる経路を、この型の進め戻しに限るためである。

use std::cell::{Cell, UnsafeCell};
use std::ops::Deref;

use himawari_core::attacks::ray_attacks;
use himawari_core::{Bitboard, EffectCounts, Move, Piece, PieceType, Position, Square};

/// 表の1段。差分を当てた後の表と、その局面の占有を持つ。
/// どちらも `catch_up` だけが書く。`do_move` は触らない（ADR-0240の4回目のp1）。
#[derive(Clone, Copy)]
struct Table {
    counts: EffectCounts,
    /// この段の局面の占有。次の段の差分を当てるときに指す前の占有として読む
    occ: Bitboard,
}

/// 進めるときに積む1段の材料。手と取られた駒（なければ空）で8バイトになる。
/// 表と別の密な配列に置き、`do_move` が触るキャッシュラインを8段で1本に減らす
/// （ADR-0240の4回目のp1）。指した後の占有は積まず、追いつくときに手から求める。
#[derive(Clone, Copy)]
struct PlyMove {
    mv: Move,
    captured: Piece,
}

/// 探索が進め戻す盤面。`Position` と、それに追従する状態を持つ。
///
/// 利き数の表は遅延して更新する（ADR-0240の1回目）。NNUEのaccumulatorと
/// 同じく、進めるときは手を積むだけにし、表を読むノードで初めて、
/// 正しい最後の段から今の段までの差分を当てる。読まずに戻るノードは費用を払わない。
pub struct SearchBoard {
    pos: Position,
    /// `tables[k]` が根からk手進めた局面の表。`valid` 以下の段だけが正しい
    tables: Vec<UnsafeCell<Table>>,
    /// `moves[k]` が根からk手目に指した手。`moves[0]` は使わない
    moves: Vec<PlyMove>,
    ply: usize,
    /// 0〜valid の段の表が正しい。読むときに今の段まで進める
    valid: Cell<usize>,
}

impl Clone for SearchBoard {
    fn clone(&self) -> SearchBoard {
        SearchBoard {
            pos: self.pos.clone(),
            tables: self
                .tables
                .iter()
                // SAFETY: 複写の間、ほかに表を書く者はいない（`&self` を借りている）
                .map(|t| UnsafeCell::new(unsafe { *t.get() }))
                .collect(),
            moves: self.moves.clone(),
            ply: self.ply,
            valid: self.valid.clone(),
        }
    }
}

impl SearchBoard {
    pub fn new(pos: Position) -> SearchBoard {
        let mut tables = Vec::with_capacity(Self::INITIAL_DEPTH);
        tables.push(UnsafeCell::new(Table {
            counts: EffectCounts::build(&pos),
            occ: pos.occupied(),
        }));
        let mut moves = Vec::with_capacity(Self::INITIAL_DEPTH);
        moves.push(PlyMove {
            mv: Move::NONE,
            captured: Piece::EMPTY,
        });
        SearchBoard {
            pos,
            tables,
            moves,
            ply: 0,
            valid: Cell::new(0),
        }
    }

    /// 最初に確保する段の数。探索の最大の深さより大きくしておけば伸ばさずに済む
    const INITIAL_DEPTH: usize = 256;

    /// 包んでいる局面。
    #[inline(always)]
    pub fn position(&self) -> &Position {
        &self.pos
    }

    /// 今の局面の利き数の表。まだ当てていない差分があれば、ここで当てる。
    #[inline(always)]
    pub fn counts(&self) -> &EffectCounts {
        if self.valid.get() < self.ply {
            self.catch_up();
        }
        // SAFETY: 今の段の表を書くのは `catch_up` だけで、それは今の段が
        // 正しくないときにしか走らない。返した参照が生きている間に今の段が
        // 正しくなくなるには `do_move`（`&mut self`）が要る
        unsafe { &(*self.tables.get_unchecked(self.ply).get()).counts }
    }

    /// 今の段の表を、まだ当てていなければいま当てる。読む前に先行して
    /// 当てておきたい場所（静止探索の進めの直後）から呼ぶ（ADR-0240の3回目のq1）。
    #[inline(always)]
    pub fn catch_up_now(&self) {
        if self.valid.get() < self.ply {
            self.catch_up();
        }
    }

    /// 正しい最後の段から今の段まで、1段ずつ複写して差分を当てる。
    /// 途中の段も正しくなるので、兄弟のノードが同じ祖先を使い回せる。
    /// `#[cold]` は付けない。静止探索の先行更新（q1）では、この呼び出しがほぼ毎回通る
    /// （ADR-0240の3回目のc1）。
    #[inline(never)]
    fn catch_up(&self) {
        let valid = self.valid.get();
        // SAFETY: valid段は正しい段で、ここでは読むだけである
        let mut occ = unsafe { (*self.tables[valid].get()).occ };
        for k in valid + 1..=self.ply {
            let pm = self.moves[k];
            // 指した後の占有。移動先は必ず埋まり、移動元は空く
            let to = Bitboard::from_square(pm.mv.to());
            let occ_after = if pm.mv.is_drop() {
                occ | to
            } else {
                (occ ^ Bitboard::from_square(pm.mv.from_sq())) | to
            };
            // SAFETY: k-1段とk段は別の要素である。k段は正しくない段なので、
            // それを指す参照は外に出ていない（`counts` は正しい段しか返さない）
            unsafe {
                let src = &(*self.tables[k - 1].get()).counts;
                let dst = &mut *self.tables[k].get();
                dst.counts.copy_from(src);
                dst.counts.update(pm.mv, pm.captured, occ, occ_after);
                dst.occ = occ_after;
            }
            occ = occ_after;
        }
        self.valid.set(self.ply);
        debug_assert_eq!(occ, self.pos.occupied(), "手から求めた占有が局面と食い違う");
        debug_assert_eq!(
            // SAFETY: 書き終えた段を読むだけで、ほかに書く者はいない
            unsafe { (*self.tables[self.ply].get()).counts },
            EffectCounts::build(&self.pos),
            "利き数の表が作り直しと食い違う"
        );
    }

    /// 段を1つ伸ばす。探索の最大の深さを超えて進めたときだけ通る。
    #[cold]
    #[inline(never)]
    fn grow(&mut self) {
        self.tables.push(UnsafeCell::new(Table {
            counts: EffectCounts::default(),
            occ: Bitboard::EMPTY,
        }));
        self.moves.push(PlyMove {
            mv: Move::NONE,
            captured: Piece::EMPTY,
        });
    }

    /// 1手進める。表には触らず、差分に要る手と取られた駒を密な配列へ積む。
    #[inline(always)]
    pub fn do_move(&mut self, m: Move) {
        let captured = if m.is_drop() {
            Piece::EMPTY
        } else {
            self.pos.piece_on(m.to())
        };
        self.pos.do_move(m);
        // 今の段より上の表は、これから書く段の古い値なので正しくない
        if self.valid.get() > self.ply {
            self.valid.set(self.ply);
        }
        let next = self.ply + 1;
        if next == self.moves.len() {
            self.grow();
        }
        // SAFETY: `moves` と `tables` は同じ長さで伸び、next < len を上で保証した
        unsafe { *self.moves.get_unchecked_mut(next) = PlyMove { mv: m, captured } };
        self.ply = next;
    }

    /// `do_move` を戻す。表は段を下げるだけで、差分を当て直さない。
    #[inline(always)]
    pub fn undo_move(&mut self, m: Move) {
        self.pos.undo_move(m);
        self.ply -= 1;
    }

    /// `Position::see_ge` と同じ値を返す。取り合いなしの判定を、地図の代わりに
    /// 表から読む（ADR-0239の案B）。
    #[inline(always)]
    pub fn see_ge(&self, m: Move, threshold: i32) -> bool {
        self.pos.see_ge_by(m, threshold, || {
            let t = self.counts();
            let them = self.pos.side_to_move().flip();
            t.count(them, m.to()) == 0 && (m.is_drop() || t.dirs(them, m.from_sq()) == 0)
        })
    }

    /// `Position::is_legal` と同じ値を返す。玉の行き先を表から読む。
    #[inline(always)]
    pub fn is_legal(&self, m: Move) -> bool {
        if self.pos.in_check() && !m.is_drop() && m.piece_before().piece_type() == PieceType::KING {
            return self.king_evasion_safe(m.to());
        }
        self.pos.is_legal_by(m, |to| {
            self.counts().count(self.pos.side_to_move().flip(), to) == 0
        })
    }

    /// 王手がかかっているときに、玉が隣の升toへ逃げて安全か（ADR-0240の3回目）。
    /// 玉を除くと伸びる相手の利きは、玉へ届いている飛び利き（王手の飛び駒）だけで、
    /// 玉の奥の1升へ延びる。表の利き数がゼロで、その1升でもなければ安全である。
    /// `Position::is_legal` の `attackers_to`（玉を除いた占有）と同じ値を返す。
    #[inline(always)]
    fn king_evasion_safe(&self, to: Square) -> bool {
        let t = self.counts();
        let them = self.pos.side_to_move().flip();
        if t.count(them, to) != 0 {
            return false;
        }
        let ksq = self.pos.king(self.pos.side_to_move());
        let mut dirs = t.dirs(them, ksq);
        while dirs != 0 {
            let d = dirs.trailing_zeros() as usize;
            dirs &= dirs - 1;
            // 方向dへ進む飛び利きは、玉の奥の1升（全升を占有にしたときの最初の升）へ延びる
            if ray_attacks(d, ksq, Bitboard::ALL).test(to) {
                return false;
            }
        }
        true
    }

    /// `mate_1ply` と同じ手を返す。候補を表から読んで落とす。
    #[inline(always)]
    pub fn mate_1ply(&self) -> Option<Move> {
        crate::mate::mate_1ply_with_counts(&self.pos, self.counts())
    }

    /// null moveで1手進める。盤面が変わらないので表はそのまま使う。
    #[inline(always)]
    pub fn do_null_move(&mut self) {
        self.pos.do_null_move();
    }

    /// `do_null_move` を戻す。
    #[inline(always)]
    pub fn undo_null_move(&mut self) {
        self.pos.undo_null_move();
    }
}

impl Deref for SearchBoard {
    type Target = Position;

    #[inline(always)]
    fn deref(&self) -> &Position {
        &self.pos
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use himawari_core::{GenType, MoveList, SFEN_STARTPOS, generate, generate_legal};

    /// 表から読む `see_ge`・`is_legal`・`mate_1ply` が、`Position` 側と同じ値を返す。
    fn walk(b: &mut SearchBoard, depth: u32) {
        if !b.in_check() {
            assert_eq!(b.mate_1ply(), crate::mate::mate_1ply(b.position()));
            let mut pseudo = MoveList::default();
            generate(b, GenType::NonEvasions, false, &mut pseudo);
            for &m in pseudo.as_slice() {
                assert_eq!(b.is_legal(m), b.position().is_legal(m), "{}", m.to_usi());
            }
        }
        let mut list = MoveList::default();
        generate_legal(b, true, &mut list);
        for &m in list.as_slice() {
            for th in [-90, 0, 90] {
                assert_eq!(
                    b.see_ge(m, th),
                    b.position().see_ge(m, th),
                    "{}",
                    m.to_usi()
                );
            }
            if depth > 0 {
                b.do_move(m);
                walk(b, depth - 1);
                b.undo_move(m);
            }
        }
    }

    /// 葉と一部のノードでだけ表を読む。読まない段が何段か続いてから読むと、
    /// 積んだ差分をまとめて当てる経路を通る（ADR-0240の遅延更新）。
    fn walk_sparse(b: &mut SearchBoard, depth: u32, n: &mut u64) {
        *n += 1;
        if depth == 0 || (*n).is_multiple_of(7) {
            assert_eq!(*b.counts(), EffectCounts::build(b.position()));
        }
        if depth == 0 {
            return;
        }
        let mut list = MoveList::default();
        generate_legal(b, true, &mut list);
        for &m in list.as_slice().iter().take(12) {
            b.do_move(m);
            walk_sparse(b, depth - 1, n);
            b.undo_move(m);
        }
    }

    #[test]
    fn lazy_counts_match_rebuild() {
        for sfen in [
            SFEN_STARTPOS,
            "l6nl/5+P1gk/2np1S3/p1p4Pp/3P2Sp1/1PPb2P1P/P5GS1/R8/LN4bKL w GR5pnsg 1",
            "+R1G4nl/1g4+Ss1/1kspp2p1/ppp2pS1p/4n4/P4Gp1P/1P1PP1P2/1+n2K2R1/7NL w G2P2b2lp 24",
        ] {
            let mut b = SearchBoard::new(Position::from_sfen(sfen).expect("sfen"));
            let mut n = 0;
            walk_sparse(&mut b, 4, &mut n);
            assert!(n > 1000, "局面数が少なすぎる: {n}");
        }
    }

    /// 王手のかかった局面で、表から読む玉の逃げ先の判定が `Position` 側と一致する。
    fn walk_evasions(b: &mut SearchBoard, depth: u32, checked: &mut u64) {
        if b.in_check() {
            *checked += 1;
            let mut pseudo = MoveList::default();
            generate(b, GenType::Evasions, false, &mut pseudo);
            for &m in pseudo.as_slice() {
                assert_eq!(
                    b.is_legal(m),
                    b.position().is_legal(m),
                    "{} {}",
                    m.to_usi(),
                    b.position().to_sfen()
                );
            }
        }
        if depth == 0 {
            return;
        }
        let mut list = MoveList::default();
        generate_legal(b, true, &mut list);
        for &m in list.as_slice() {
            b.do_move(m);
            walk_evasions(b, depth - 1, checked);
            b.undo_move(m);
        }
    }

    #[test]
    fn evasion_reads_match_position() {
        let mut checked = 0;
        for sfen in [
            "l6nl/5+P1gk/2np1S3/p1p4Pp/3P2Sp1/1PPb2P1P/P5GS1/R8/LN4bKL w GR5pnsg 1",
            "+R1G4nl/1g4+Ss1/1kspp2p1/ppp2pS1p/4n4/P4Gp1P/1P1PP1P2/1+n2K2R1/7NL w G2P2b2lp 24",
        ] {
            let mut b = SearchBoard::new(Position::from_sfen(sfen).expect("sfen"));
            walk_evasions(&mut b, 2, &mut checked);
        }
        assert!(checked > 100, "王手の局面が少なすぎる: {checked}");
    }

    #[test]
    fn table_reads_match_position() {
        for sfen in [
            SFEN_STARTPOS,
            "l6nl/5+P1gk/2np1S3/p1p4Pp/3P2Sp1/1PPb2P1P/P5GS1/R8/LN4bKL w GR5pnsg 1",
            "+R1G4nl/1g4+Ss1/1kspp2p1/ppp2pS1p/4n4/P4Gp1P/1P1PP1P2/1+n2K2R1/7NL w G2P2b2lp 24",
        ] {
            let mut b = SearchBoard::new(Position::from_sfen(sfen).expect("sfen"));
            walk(&mut b, 1);
        }
    }
}
