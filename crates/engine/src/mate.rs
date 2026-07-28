//! 1手詰め判定（ADR-0029）。
//!
//! 案B: 敵玉近傍の候補手だけを列挙し、各候補は実際に指して
//! 回避手の有無で検証する。検証を通った手しか返さないため
//! 誤検出はない。合駒可能な遠距離の王手は候補にしないので
//! 見逃しはあり得る（健全・不完全）。
//! 案A（全合法手からの王手列挙・検証）はテスト用オラクル。
//! 歩打ちは打ち歩詰めで常に非合法なので候補にしない。

use himawari_core::{
    Bitboard, Color, GenType, Move, MoveList, PieceType, Position, attacks, generate,
    generate_legal,
};

/// 候補手を実際に指し、王手かつ回避不能かを検証する。
///
/// do_moveは重い（NNUE差分の材料づくりと王手情報の更新を伴う）ので、
/// 王手にならない手は指す前に弾く（ADR-0094）。回避手の判定も、1つ
/// 見つけた時点で打ち切る。
fn is_mate_move(pos: &mut Position, m: Move) -> bool {
    if !pos.pseudo_legal(m) || !pos.is_legal(m) {
        return false;
    }
    // 王手でなければ詰みにならない。gives_checkは開き王手も拾うため、
    // 元の実装（指してからin_checkを見る）と同じ手が残る
    if !pos.gives_check(m) {
        return false;
    }
    pos.do_move(m);
    // 回避手が1つでもあれば詰みではない。全部を集めてから数える必要はない
    let mut pseudo = MoveList::default();
    generate(pos, GenType::Evasions, true, &mut pseudo);
    let escapable = pseudo.as_slice().iter().any(|&mv| pos.is_legal(mv));
    pos.undo_move(m);
    !escapable
}

/// 駒打ちの詰み候補。合駒不能な近接打ち（玉の隣接＋桂）に限る。
fn drop_mates(pos: &mut Position, us: Color, ksq: Square) -> Option<Move> {
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
fn move_mates(pos: &mut Position, us: Color, ksq: Square) -> Option<Move> {
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

use himawari_core::Square;

/// 1手詰めがあればその手を返す。王手されていない局面で呼ぶこと。
/// 返した手は詰みであることが保証される（見逃しはあり得る）。
pub fn mate_1ply(pos: &mut Position) -> Option<Move> {
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
