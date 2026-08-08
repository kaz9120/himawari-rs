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

use crate::attacks::attacks;
use crate::bitboard::Bitboard;
use crate::moves::Move;
use crate::piece::Piece;
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
    pub fn rebuild(&mut self, pos: &Position) {
        self.to_sq = [Bitboard::EMPTY; Square::NB];
        self.from_sq = [Bitboard::EMPTY; Square::NB];
        let occ = pos.occupied();
        for from in occ {
            let pc = pos.piece_on(from);
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
    /// 短い利きの駒は占有が変わっても利きが動かないが、選り分けずに
    /// 数え直す。判定を挟むより素直で、誤りが入りにくい。
    pub fn update(&mut self, pos: &Position, m: Move) {
        let to = m.to();
        let mut touched = Bitboard::from_square(to) | self.to_sq[to.index()];
        if !m.is_drop() {
            let from = m.from_sq();
            touched |= Bitboard::from_square(from) | self.to_sq[from.index()];
        }

        // 先に古い利きを全部落とす。落としてから足さないと、同じ駒を
        // 2度数える
        for sq in touched {
            for t in self.from_sq[sq.index()] {
                self.to_sq[t.index()].clear(sq);
            }
            self.from_sq[sq.index()] = Bitboard::EMPTY;
        }

        let occ = pos.occupied();
        for sq in touched {
            let pc = pos.piece_on(sq);
            if pc == Piece::EMPTY {
                continue;
            }
            let att = attacks(pc, sq, occ);
            self.from_sq[sq.index()] = att;
            for t in att {
                self.to_sq[t.index()].set(sq);
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SFEN_STARTPOS;

    /// 全マスについて、差分なしの `attackers_to` と一致するか。
    fn assert_matches_attackers_to(pos: &Position) {
        let mut t = EffectTable::new();
        t.rebuild(pos);
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
        t.rebuild(&pos);
        // 先手の歩は7七にあり、7六へ利く。7六に立つ先手の駒は歩1枚だけ
        let target = Square::from_usi("7f").expect("7f");
        assert_eq!(t.count(&pos, target, Color::Black), 1);
        assert_eq!(t.count(&pos, target, Color::White), 0);
    }

    #[test]
    fn attacks_from_is_empty_on_an_empty_square() {
        let pos = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
        let mut t = EffectTable::new();
        t.rebuild(&pos);
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
            diff.update(&next, m);
            let mut full = EffectTable::new();
            full.rebuild(&next);
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
        t.rebuild(&pos);
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
        t.rebuild(&pos);
        walk_and_compare(&mut pos, &t, 2);
    }

    /// 逆向きの索引が食い違っていないか。from_sq に入っているなら
    /// to_sq にも入っていて、その逆も成り立つ。
    #[test]
    fn both_indices_agree() {
        let sfen = "l6nl/5+P1gk/2np1S3/p1p4Pp/3P2Sp1/1PPb2P1P/P5GS1/R8/LN4bKL w GR5pnsg 1";
        let pos = Position::from_sfen(sfen).expect("sfen");
        let mut t = EffectTable::new();
        t.rebuild(&pos);
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
}
