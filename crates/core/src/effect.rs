//! 盤面の利きを保持する（ADR-0148）。
//!
//! `attackers_to` は呼ばれるたびに数え上げる。盤面全体の利きが要る用途
//! （EffectBucket・SEE・詰み判定）では、81マス分を呼ぶことになる。
//! ここでは両方向の索引を持ち、差分で維持できる形にする。
//!
//! - `to_sq[sq]`   … sq へ利きを持つ駒の位置
//! - `from_sq[sq]` … sq にある駒が利いているマス
//!
//! 逆向きの索引（`from_sq`）を持つのは差分更新のためである。駒の利きを
//! 消すとき、その駒がどこへ利いていたかを計算し直さずに済む。
//!
//! 利き数は `count(pos, sq, c)` で取る。駒の色は盤面から引くので、
//! ここでは色ごとの表を持たない。

use crate::attacks::{
    attacks, bishop_attacks, gold_attacks, king_attacks, knight_attacks, lance_attacks,
    rook_attacks, silver_attacks,
};
use crate::bitboard::Bitboard;
use crate::moves::Move;
use crate::piece::{Piece, PieceType};
use crate::position::Position;
use crate::types::{Color, Square};

/// 盤面の利き。局面と対で持ち、`do_move` に追従させる。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EffectTable {
    /// sq へ利きを持つ駒の位置
    to_sq: [Bitboard; Square::NB],
    /// sq にある駒が利いているマス
    from_sq: [Bitboard; Square::NB],
}

impl Default for EffectTable {
    fn default() -> Self {
        Self::new()
    }
}

impl EffectTable {
    pub fn new() -> Self {
        EffectTable {
            to_sq: [Bitboard::EMPTY; Square::NB],
            from_sq: [Bitboard::EMPTY; Square::NB],
        }
    }

    /// 盤面から全部作り直す。差分更新の正しさは、これとの一致で確かめる。
    ///
    /// `Position` ではなく盤面と占有を受けるのは、`Position` の中から
    /// 呼ぶためである。フィールドを個別に渡せば、`effect` を可変で借りつつ
    /// `board` を不変で借りられる。
    pub fn rebuild(&mut self, board: &[Piece; Square::NB], occ: Bitboard) {
        self.to_sq = [Bitboard::EMPTY; Square::NB];
        self.from_sq = [Bitboard::EMPTY; Square::NB];
        for from in occ {
            let pc = board[from.index()];
            let att = attacks(pc, from, occ);
            self.from_sq[from.index()] = att;
            for to in att {
                self.to_sq[to.index()] |= Bitboard::from_square(from);
            }
        }
    }

    /// 1手ぶん進めた盤面へ追従する。`pos` は `do_move` を終えた局面、
    /// `m` はそのときの指し手である。
    ///
    /// 利きが変わるのは3種類ある。動いた駒、取られた駒、そして占有が
    /// 変わったマスを通る飛び駒である。3つ目を拾うために `to_sq` の逆引きを
    /// 使う。占有が変わるのは移動元と移動先の2マスだけなので、そこへ利きを
    /// 持つ駒を集めれば漏れない（ADR-0148）。
    ///
    /// 数え直すのは動いたマスと、そこへ利きを持つ**飛び駒**だけでよい。
    /// 短い利きの駒は占有が変わっても利きが動かない。全部数え直す実装から
    /// 絞ったところ、NPSの低下が14.96%から縮んだ（ADR-0148）。
    ///
    /// `sliders` は香・角・飛・馬・竜の位置である。呼び出し側が渡すのは、
    /// `Position` の中から呼ぶときにフィールドを個別に借りるためである。
    pub fn update(
        &mut self,
        board: &[Piece; Square::NB],
        occ: Bitboard,
        sliders: Bitboard,
        m: Move,
    ) {
        let to = m.to();
        let mut touched = Bitboard::from_square(to) | (self.to_sq[to.index()] & sliders);
        if !m.is_drop() {
            let from = m.from_sq();
            touched |= Bitboard::from_square(from) | (self.to_sq[from.index()] & sliders);
        }

        // 触るのは増えた分と減った分だけにする。飛び駒が1マス伸びても、
        // 全部消して全部足すと利きマスの数（飛車なら16回）を払う。差分に
        // すると数回で済む（ADR-0148）
        for sq in touched {
            let pc = board[sq.index()];
            let new = if pc == Piece::EMPTY {
                Bitboard::EMPTY
            } else {
                attacks(pc, sq, occ)
            };
            let old = self.from_sq[sq.index()];
            if old == new {
                continue;
            }
            for t in old & !new {
                self.to_sq[t.index()].clear(sq);
            }
            for t in new & !old {
                self.to_sq[t.index()].set(sq);
            }
            self.from_sq[sq.index()] = new;
        }
    }

    /// sq へ利きを持つ駒の位置。
    pub fn attackers(&self, sq: Square) -> Bitboard {
        self.to_sq[sq.index()]
    }

    /// sq にある駒が利いているマス。駒がなければ空。
    pub fn attacks_from(&self, sq: Square) -> Bitboard {
        self.from_sq[sq.index()]
    }

    /// sq へ利いている色cの駒数。玉の利きも数え、sq にある駒自身は数えない。
    ///
    /// ピンは見ない。盤の生の利きである（ADR-0148）。EffectBucketの
    /// バケット決定はこの値を使う。
    pub fn count(&self, pos: &Position, sq: Square, c: Color) -> u32 {
        (self.to_sq[sq.index()] & pos.color_bb(c)).count()
    }
}

/// 色cの利きの地図（ADR-0239の案E）。
///
/// 維持はせず、使うノードで作る。`EffectTable` と違って利いている駒の位置は
/// 持たず、1枚以上・2枚以上利いている升の集合だけを持つ。玉の利きも数える。
/// 1手詰めの候補を落とす、玉の行き先が安全かを見る、といった用途では、
/// 位置まで要らずにこれで決まる場合が大半である。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct EffectMap {
    /// cの駒が1枚以上利いている升
    pub any: Bitboard,
    /// cの駒が2枚以上利いている升
    pub multi: Bitboard,
    /// cの飛び駒（香・角・飛と、馬・竜の飛び利き）が利いている升。
    /// この升の駒が動くと、その奥へ飛び利きが伸びうる
    pub slider: Bitboard,
}

impl EffectMap {
    #[inline(always)]
    fn add(&mut self, x: Bitboard) {
        self.multi |= self.any & x;
        self.any |= x;
    }

    #[inline(always)]
    fn add_slider(&mut self, x: Bitboard) {
        self.add(x);
        self.slider |= x;
    }

    /// 盤全体の地図を作る。
    pub fn build(pos: &Position, c: Color) -> EffectMap {
        Self::build_near(pos, c, Bitboard::ALL)
    }

    /// 銀・金・玉（金の動きをする成駒を含む）は、`near` にある駒だけを数える。
    /// 歩・桂・飛び駒は全部数える。
    ///
    /// 銀・金・玉の利きは隣の升にしか届かないので、ある升の周りの3×3だけを
    /// 正しく知りたいときは、5×5を `near` に渡せば足りる。`near` の外の駒が
    /// 利く升では、`any` と `multi` が実際より少なくなる。
    pub fn build_near(pos: &Position, c: Color, near: Bitboard) -> EffectMap {
        let occ = pos.occupied();
        let mut m = EffectMap::default();
        // 歩の利きは駒ごとに別の升なので、集合のまま1段ずらせば重ならない
        let pawns = pos.pieces(c, PieceType::PAWN);
        m.add(match c {
            Color::Black => pawns.shift_up(),
            Color::White => pawns.shift_down(),
        });
        for sq in pos.pieces(c, PieceType::KNIGHT) {
            m.add(knight_attacks(c, sq));
        }
        for sq in pos.pieces(c, PieceType::SILVER) & near {
            m.add(silver_attacks(c, sq));
        }
        for sq in pos.golds(c) & near {
            m.add(gold_attacks(c, sq));
        }
        let ksq = pos.king(c);
        if near.test(ksq) && pos.pieces(c, PieceType::KING).test(ksq) {
            m.add(king_attacks(ksq));
        }
        for sq in pos.pieces(c, PieceType::LANCE) {
            m.add_slider(lance_attacks(c, sq, occ));
        }
        for sq in pos.pieces(c, PieceType::BISHOP) {
            m.add_slider(bishop_attacks(sq, occ));
        }
        for sq in pos.pieces(c, PieceType::ROOK) {
            m.add_slider(rook_attacks(sq, occ));
        }
        // 馬・竜は飛び利きと1升の利きを分けて足す。同じ駒の利きなので、
        // 2つが重なる升はない
        for sq in pos.pieces(c, PieceType::HORSE) {
            let ray = bishop_attacks(sq, occ);
            m.add(ray | (king_attacks(sq) & !ray));
            m.slider |= ray;
        }
        for sq in pos.pieces(c, PieceType::DRAGON) {
            let ray = rook_attacks(sq, occ);
            m.add(ray | (king_attacks(sq) & !ray));
            m.slider |= ray;
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SFEN_STARTPOS;

    /// 全マスについて、差分なしの `attackers_to` と一致するか。
    fn assert_matches_attackers_to(pos: &Position) {
        let mut t = EffectTable::new();
        t.rebuild(pos.board_ref(), pos.occupied());
        let occ = pos.occupied();
        for i in 0..Square::NB {
            let sq = Square::from_index(i as u8);
            let want =
                pos.attackers_to(Color::Black, sq, occ) | pos.attackers_to(Color::White, sq, occ);
            assert_eq!(
                t.attackers(sq),
                want,
                "利きが食い違う: {sq:?}\n{}",
                pos.to_sfen()
            );
        }
    }

    #[test]
    fn rebuild_matches_attackers_to_at_startpos() {
        let pos = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
        assert_matches_attackers_to(&pos);
    }

    #[test]
    fn rebuild_matches_attackers_to_with_sliders() {
        // 飛・角・香の利きが開いている中盤の局面
        let sfen = "l6nl/5+P1gk/2np1S3/p1p4Pp/3P2Sp1/1PPb2P1P/P5GS1/R8/LN4bKL w GR5pnsg 1";
        let pos = Position::from_sfen(sfen).expect("sfen");
        assert_matches_attackers_to(&pos);
    }

    #[test]
    fn count_excludes_the_piece_on_the_square() {
        let pos = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
        let mut t = EffectTable::new();
        t.rebuild(pos.board_ref(), pos.occupied());
        // 先手の歩は7七にあり、7六へ利く。7六に立つ先手の駒は歩1枚だけ
        let target = Square::from_usi("7f").expect("7f");
        assert_eq!(t.count(&pos, target, Color::Black), 1);
        assert_eq!(t.count(&pos, target, Color::White), 0);
    }

    #[test]
    fn attacks_from_is_empty_on_an_empty_square() {
        let pos = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
        let mut t = EffectTable::new();
        t.rebuild(pos.board_ref(), pos.occupied());
        let empty = Square::from_usi("5e").expect("5e");
        assert!(t.attacks_from(empty).is_empty());
    }

    /// 指し手を進めるたびに、差分更新と作り直しが一致するか。
    /// 全合法手を深さdまで辿り、各ノードで突き合わせる。
    fn walk_and_compare(pos: &mut Position, table: &EffectTable, depth: u32) -> u64 {
        if depth == 0 {
            return 1;
        }
        let mut list = crate::moves::MoveList::default();
        crate::movegen::generate_legal(pos, true, &mut list);
        let mut n = 0u64;
        for &m in &list {
            let mut next = pos.clone();
            next.do_move(m);
            let mut diff = table.clone();
            diff.update(next.board_ref(), next.occupied(), next.sliders(), m);
            let mut full = EffectTable::new();
            full.rebuild(next.board_ref(), next.occupied());
            assert_eq!(
                diff,
                full,
                "差分更新が作り直しと食い違う: 手={m:?}\n{}",
                next.to_sfen()
            );
            n += walk_and_compare(&mut next, &diff, depth - 1);
        }
        n
    }

    #[test]
    fn incremental_update_matches_rebuild_from_startpos() {
        let mut pos = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
        let mut t = EffectTable::new();
        t.rebuild(pos.board_ref(), pos.occupied());
        // 深さ3で約2.5万局面。飛び駒の伸縮と駒取りが十分に現れる
        let n = walk_and_compare(&mut pos, &t, 3);
        assert!(n > 20000, "局面数が少なすぎる: {n}");
    }

    #[test]
    fn incremental_update_matches_rebuild_in_midgame() {
        // 飛・角・香の利きが交差し、駒打ちも成りも出る局面
        let sfen = "l6nl/5+P1gk/2np1S3/p1p4Pp/3P2Sp1/1PPb2P1P/P5GS1/R8/LN4bKL w GR5pnsg 1";
        let mut pos = Position::from_sfen(sfen).expect("sfen");
        let mut t = EffectTable::new();
        t.rebuild(pos.board_ref(), pos.occupied());
        walk_and_compare(&mut pos, &t, 2);
    }

    /// 逆向きの索引が食い違っていないか。from_sq に入っているなら
    /// to_sq にも入っていて、その逆も成り立つ。
    #[test]
    fn both_indices_agree() {
        let sfen = "l6nl/5+P1gk/2np1S3/p1p4Pp/3P2Sp1/1PPb2P1P/P5GS1/R8/LN4bKL w GR5pnsg 1";
        let pos = Position::from_sfen(sfen).expect("sfen");
        let mut t = EffectTable::new();
        t.rebuild(pos.board_ref(), pos.occupied());
        for i in 0..Square::NB {
            let from = Square::from_index(i as u8);
            for to in t.attacks_from(from) {
                assert!(
                    t.attackers(to).test(from),
                    "from_sq にあって to_sq にない: {from:?} -> {to:?}"
                );
            }
            for from2 in t.attackers(from) {
                assert!(
                    t.attacks_from(from2).test(from),
                    "to_sq にあって from_sq にない: {from2:?} -> {from:?}"
                );
            }
        }
    }

    /// 地図が全升で `attackers_to` の数と一致するか（ADR-0239）。
    fn assert_map_matches(pos: &Position) {
        let occ = pos.occupied();
        for c in [Color::Black, Color::White] {
            let m = EffectMap::build(pos, c);
            let mut slider = Bitboard::EMPTY;
            for sq in pos.color_bb(c) {
                let pt = pos.piece_on(sq).piece_type();
                slider |= match pt {
                    PieceType::LANCE => lance_attacks(c, sq, occ),
                    PieceType::BISHOP | PieceType::HORSE => bishop_attacks(sq, occ),
                    PieceType::ROOK | PieceType::DRAGON => rook_attacks(sq, occ),
                    _ => Bitboard::EMPTY,
                };
            }
            assert_eq!(m.slider, slider, "飛び利きが食い違う\n{}", pos.to_sfen());
            for i in 0..Square::NB {
                let sq = Square::from_index(i as u8);
                let n = pos.attackers_to(c, sq, occ).count();
                assert_eq!(m.any.test(sq), n >= 1, "1枚以上: {sq:?}\n{}", pos.to_sfen());
                assert_eq!(
                    m.multi.test(sq),
                    n >= 2,
                    "2枚以上: {sq:?}\n{}",
                    pos.to_sfen()
                );
            }
        }
    }

    fn walk_maps(pos: &mut Position, depth: u32) -> u64 {
        assert_map_matches(pos);
        if depth == 0 {
            return 1;
        }
        let mut list = crate::moves::MoveList::default();
        crate::movegen::generate_legal(pos, true, &mut list);
        let mut n = 0;
        for &m in &list {
            pos.do_move(m);
            n += walk_maps(pos, depth - 1);
            pos.undo_move(m);
        }
        n
    }

    #[test]
    fn effect_map_matches_attackers_to() {
        let mut pos = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
        walk_maps(&mut pos, 2);
        let sfen = "l6nl/5+P1gk/2np1S3/p1p4Pp/3P2Sp1/1PPb2P1P/P5GS1/R8/LN4bKL w GR5pnsg 1";
        let mut pos = Position::from_sfen(sfen).expect("sfen");
        let n = walk_maps(&mut pos, 2);
        assert!(n > 1000, "局面数が少なすぎる: {n}");
    }

    /// `build_near` は、5×5を渡した升の周り3×3で全体の地図と一致する。
    #[test]
    fn effect_map_near_is_exact_around_the_center() {
        let sfen = "l6nl/5+P1gk/2np1S3/p1p4Pp/3P2Sp1/1PPb2P1P/P5GS1/R8/LN4bKL w GR5pnsg 1";
        let pos = Position::from_sfen(sfen).expect("sfen");
        for c in [Color::Black, Color::White] {
            let full = EffectMap::build(&pos, c);
            for i in 0..Square::NB {
                let center = Square::from_index(i as u8);
                let zone = king_attacks(center) | Bitboard::from_square(center);
                let mut box5 = zone;
                for s in zone {
                    box5 |= king_attacks(s);
                }
                let near = EffectMap::build_near(&pos, c, box5);
                assert_eq!(near.any & zone, full.any & zone);
                assert_eq!(near.multi & zone, full.multi & zone);
                assert_eq!(near.slider, full.slider);
            }
        }
    }
}
