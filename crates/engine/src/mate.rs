//! 1手詰め判定（ADR-0029）。
//!
//! 敵玉近傍の候補手だけを列挙し、各候補は**駒を動かさずに**検証する。
//! `do_move` / `undo_move` は使わない。移動後の利きは、移動元の駒を
//! マスクで外し、移動先の駒の利きを別に足して求める（ADR-0109、
//! 出典はやねうら王 mate/mate1ply_without_effect.cpp）。
//!
//! 検証を通った手しか返さないため誤検出はない。合駒可能な遠距離の
//! 王手は安全側に倒すので見逃しはあり得る（健全・不完全）。
//! 案A（全合法手からの王手列挙・検証）はテスト用オラクル。
//! 歩打ちは打ち歩詰めで常に非合法なので候補にしない。

use himawari_core::{
    Bitboard, Color, Move, MoveList, Piece, PieceType, Position, Square, attacks, generate_legal,
};

/// 手を指した後の盤面で、手番側の駒がsqへ利いている集合。
///
/// 盤面は変えない。移動元fromの駒は移動済みなのでマスクで外し、
/// 移動先toの駒pcの利きは別に足す。occは移動後の占有を渡す。
/// 出典はやねうら王 mate1ply_without_effect.cpp:537（AttacksAroundKingInAvoiding）。
#[inline]
fn our_attackers(
    pos: &Position,
    us: Color,
    sq: Square,
    occ: Bitboard,
    from_bb: Bitboard,
    to: Square,
    pc: Piece,
) -> Bitboard {
    let mut a = pos.attackers_to(us, sq, occ) & !from_bb;
    if attacks::attacks(pc, to, occ).test(sq) {
        a |= Bitboard::from_square(to);
    }
    a
}

/// 手mが詰みかを、駒を動かさずに判定する。
///
/// trueを返すなら確実に詰み（偽陽性なし）。合駒が成立しうる局面は
/// 安全側に倒してfalseを返すため、見逃しはあり得る。
fn is_mate_move(pos: &Position, m: Move) -> bool {
    let us = pos.side_to_move();
    let them = us.flip();
    let ksq = pos.king(them);
    let ksq_bb = Bitboard::from_square(ksq);
    let to = m.to();
    let to_bb = Bitboard::from_square(to);
    let pc = m.piece_after();
    let occ = pos.occupied();
    let from_bb = if m.is_drop() {
        Bitboard::EMPTY
    } else {
        Bitboard::from_square(m.from_sq())
    };
    // 移動後の占有。toは駒を取った場合も占有のままなのでORでよい
    let occ_after = (occ ^ from_bb) | to_bb;
    // 玉を除いた占有。玉が動いた後の利きを見るのに使う
    let occ_no_king = occ_after ^ ksq_bb;

    // 1. 移動先の駒を玉に取られるなら詰みではない。いちばん効く枝刈りなので
    //    合法性の検査より前に置く。取られる駒は自分自身へ利かないので、
    //    移動先の利きを足す必要はない
    if attacks::king_attacks(ksq).test(to)
        && (pos.attackers_to(us, to, occ_no_king) & !from_bb).is_empty()
    {
        return false;
    }
    if !pos.pseudo_legal(m) || !pos.is_legal(m) {
        return false;
    }

    // 2. 王手になっているか。開き王手もここで拾える
    let checkers = our_attackers(pos, us, ksq, occ_after, from_bb, to, pc);
    if checkers.is_empty() {
        return false;
    }

    // 3. 玉の逃げ場。移動先toへ逃げる手は1で処理済み。
    //    自駒のあるマスは取って逃げる手なので候補に残す
    let their_after = pos.color_bb(them) & !to_bb;
    for s in attacks::king_attacks(ksq) & !their_after & !to_bb {
        let o = occ_no_king | Bitboard::from_square(s);
        if our_attackers(pos, us, s, o, from_bb, to, pc).is_empty() {
            return false;
        }
    }

    // 両王手は玉が動くしかない。取っても合駒しても片方が残る
    if checkers.more_than_one() {
        return true;
    }
    let csq = checkers.lsb();
    let csq_bb = Bitboard::from_square(csq);

    // 4. 玉以外の駒で王手駒を取る。取った後も王手が残るなら回避にならない
    for d in pos.attackers_to(them, csq, occ_after) & !to_bb & !ksq_bb {
        let o = occ_after ^ Bitboard::from_square(d);
        // csqにあった駒は取られたのでマスクで外す
        if (our_attackers(pos, us, ksq, o, from_bb, to, pc) & !csq_bb).is_empty() {
            return false;
        }
    }

    // 5. 合駒。玉と王手駒の間があるのは開き王手の場合だけ
    let inter = attacks::between(ksq, csq);
    if !inter.is_empty() {
        // 持ち駒があれば遮れるとみなす（安全側）
        if !pos.hand(them).is_empty() {
            return false;
        }
        for s in inter {
            let s_bb = Bitboard::from_square(s);
            // 将棋の駒は利くマスへ動けるので、逆利きの集合が移動元になる
            for d in pos.attackers_to(them, s, occ_after) & !to_bb & !ksq_bb {
                let o = (occ_after ^ Bitboard::from_square(d)) | s_bb;
                if our_attackers(pos, us, ksq, o, from_bb, to, pc).is_empty() {
                    return false;
                }
            }
        }
    }
    true
}

/// 駒打ちの詰み候補。合駒不能な近接打ち（玉の隣接＋桂）に限る。
fn drop_mates(pos: &Position, us: Color, ksq: Square) -> Option<Move> {
    let hand = pos.hand(us);
    if hand.is_empty() {
        return None;
    }
    let them = us.flip();
    let empty = !pos.occupied() & Bitboard::ALL;
    // ptを打ってksqに王手できるマス。逆視点の利きで求める。
    // 飛角は全升占有の利き＝隣接4マスだけを使う（遠打ちは合駒可能）
    for pt in [
        PieceType::GOLD,
        PieceType::SILVER,
        PieceType::KNIGHT,
        PieceType::LANCE,
        PieceType::ROOK,
        PieceType::BISHOP,
    ] {
        if hand.count(pt) == 0 {
            continue;
        }
        let targets = match pt {
            PieceType::GOLD => attacks::gold_attacks(them, ksq),
            PieceType::SILVER => attacks::silver_attacks(them, ksq),
            PieceType::KNIGHT => attacks::knight_attacks(them, ksq),
            PieceType::LANCE => attacks::pawn_attacks(them, ksq),
            PieceType::ROOK => attacks::rook_attacks(ksq, Bitboard::ALL),
            _ => attacks::bishop_attacks(ksq, Bitboard::ALL),
        } & empty;
        for to in targets {
            let m = Move::new_drop(pt, to, us);
            if is_mate_move(pos, m) {
                return Some(m);
            }
        }
    }
    None
}

/// 盤上の駒の詰み候補。移動先を玉の隣接8マスと桂の王手マスに限る。
/// 王手になるかの事前判定はせず、検証側に任せる（間接王手も拾える）。
fn move_mates(pos: &Position, us: Color, ksq: Square) -> Option<Move> {
    let them = us.flip();
    let occ = pos.occupied();
    let near = attacks::king_attacks(ksq) & !pos.color_bb(us);
    let knight_to = attacks::knight_attacks(them, ksq) & !pos.color_bb(us);
    let zone = Bitboard::promotion_zone(us);
    for from in pos.color_bb(us) {
        let pc = pos.piece_on(from);
        let pt = pc.piece_type();
        if pt == PieceType::KING {
            continue;
        }
        let targets = attacks::attacks(pc, from, occ)
            & if pt == PieceType::KNIGHT {
                knight_to
            } else {
                near
            };
        for to in targets {
            if pt.can_promote() && (zone.test(from) || zone.test(to)) {
                let m = Move::new_move(from, to, true, pc.promote());
                if is_mate_move(pos, m) {
                    return Some(m);
                }
            }
            let m = Move::new_move(from, to, false, pc);
            if is_mate_move(pos, m) {
                return Some(m);
            }
        }
    }
    None
}

/// 1手詰めがあればその手を返す。王手されていない局面で呼ぶこと。
/// 返した手は詰みであることが保証される（見逃しはあり得る）。
pub fn mate_1ply(pos: &Position) -> Option<Move> {
    debug_assert!(!pos.in_check());
    let us = pos.side_to_move();
    let ksq = pos.king(us.flip());
    move_mates(pos, us, ksq).or_else(|| drop_mates(pos, us, ksq))
}

/// テスト用オラクル（案A）: 全合法手から王手を選び、回避不能なら詰み。
/// 遅いが完全。mate_1plyの照合にのみ使う。
pub fn mate_1ply_oracle(pos: &mut Position) -> Option<Move> {
    let mut list = MoveList::default();
    generate_legal(pos, true, &mut list);
    let moves: Vec<Move> = list.as_slice().to_vec();
    for m in moves {
        if !pos.gives_check(m) {
            continue;
        }
        pos.do_move(m);
        let mate = {
            let mut ev = MoveList::default();
            generate_legal(pos, true, &mut ev);
            ev.is_empty()
        };
        pos.undo_move(m);
        if mate {
            return Some(m);
        }
    }
    None
}
