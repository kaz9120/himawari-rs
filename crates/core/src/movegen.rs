//! 指し手生成（ADR-0017）。
//!
//! 生成分類はCaptures（歩成含む）／Quiets／NonEvasions／Evasions。
//! `all = false`（Normal）は無意味な不成を生成せず、`all = true` は
//! 全合法手（perft・検証用）を生成する。

use crate::attacks::{attacks, between, king_attacks};
use crate::bitboard::Bitboard;
use crate::moves::{Move, MoveList};
use crate::piece::{Piece, PieceType};
use crate::position::Position;
use crate::types::{Color, Rank, Square};

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum GenType {
    Captures,
    Quiets,
    NonEvasions,
    Evasions,
}

/// 成り・不成の変種を積む（ADR-0017の成り規約）。
fn push_variants(us: Color, from: Square, to: Square, pc: Piece, all: bool, list: &mut MoveList) {
    let pt = pc.piece_type();
    let zone = Bitboard::promotion_zone(us);
    let can_pro = pt.can_promote() && (zone.test(from) || zone.test(to));
    // 手番から見た移動先の段（0 = 一段目）
    let rel = to.rank().relative(us).0;

    if can_pro {
        list.push(Move::new_move(from, to, true, pc.promote()));
        let non_promote = match pt {
            // 歩・香は1段目、桂は1〜2段目に不成で入れない
            PieceType::PAWN => {
                if all {
                    rel >= 1
                } else {
                    false
                }
            }
            PieceType::LANCE => {
                if all {
                    rel >= 1
                } else {
                    rel == 1
                }
            }
            PieceType::KNIGHT => all && rel >= 2,
            PieceType::SILVER => true,
            PieceType::BISHOP | PieceType::ROOK => all,
            _ => unreachable!("can_promote is limited to 6 kinds"),
        };
        if non_promote {
            list.push(Move::new_move(from, to, false, pc));
        }
    } else {
        // 成れない移動。行き所のない駒になる手は生成しない
        let ok = match pt {
            PieceType::PAWN | PieceType::LANCE => rel >= 1,
            PieceType::KNIGHT => rel >= 2,
            _ => true,
        };
        if ok {
            list.push(Move::new_move(from, to, false, pc));
        }
    }
}

/// 玉以外の盤上の駒による、targetへの移動を生成する。
fn generate_board_moves(pos: &Position, target: Bitboard, all: bool, list: &mut MoveList) {
    let us = pos.side_to_move();
    let occ = pos.occupied();
    let movers = pos.color_bb(us) & !pos.pieces(us, PieceType::KING);
    for from in movers {
        let pc = pos.piece_on(from);
        let att = attacks(pc, from, occ) & target;
        for to in att {
            push_variants(us, from, to, pc, all, list);
        }
    }
}

/// 玉の移動を生成する。
fn generate_king_moves(pos: &Position, target: Bitboard, list: &mut MoveList) {
    let us = pos.side_to_move();
    let from = pos.king(us);
    let pc = pos.piece_on(from);
    for to in king_attacks(from) & target {
        list.push(Move::new_move(from, to, false, pc));
    }
}

/// 駒打ちを生成する。targetは空きマスの部分集合であること。
fn generate_drops(pos: &Position, target: Bitboard, list: &mut MoveList) {
    let us = pos.side_to_move();
    let hand = pos.hand(us);
    if hand.is_empty() {
        return;
    }
    let r1 = Bitboard::rank(Rank(0).relative(us));
    let r2 = Bitboard::rank(Rank(1).relative(us));
    for pt in PieceType::HAND_KINDS {
        if !hand.has(pt) {
            continue;
        }
        let mask = match pt {
            PieceType::PAWN => {
                // 二歩と1段目を除外。歩のいる筋はfill_filesで一括して求める
                let nifu = pos.pieces(us, PieceType::PAWN).fill_files();
                target & !r1 & !nifu
            }
            PieceType::LANCE => target & !r1,
            PieceType::KNIGHT => target & !(r1 | r2),
            _ => target,
        };
        for to in mask {
            list.push(Move::new_drop(pt, to, us));
        }
    }
}

/// 王手回避を生成する（ADR-0016）。
fn generate_evasions(pos: &Position, all: bool, list: &mut MoveList) {
    let us = pos.side_to_move();
    let checkers = pos.checkers();
    debug_assert!(!checkers.is_empty());

    generate_king_moves(pos, !pos.color_bb(us), list);
    if checkers.more_than_one() {
        return;
    }
    let checker = checkers.lsb();
    let block = between(pos.king(us), checker);
    generate_board_moves(pos, block | checkers, all, list);
    generate_drops(pos, block, list);
}

/// 擬似合法手を生成する。合法性はis_legalで別途検査する。
pub fn generate(pos: &Position, gt: GenType, all: bool, list: &mut MoveList) {
    let us = pos.side_to_move();
    debug_assert!(gt == GenType::Evasions || !pos.in_check());
    match gt {
        GenType::Evasions => generate_evasions(pos, all, list),
        GenType::NonEvasions => {
            let target = !pos.color_bb(us);
            generate_board_moves(pos, target, all, list);
            generate_king_moves(pos, target, list);
            generate_drops(pos, !pos.occupied(), list);
        }
        GenType::Captures => {
            let target = pos.color_bb(us.flip());
            generate_board_moves(pos, target, all, list);
            generate_king_moves(pos, target, list);
            // 歩成（空きマスへの成り）もCapturesに含める（ADR-0017）
            let zone = Bitboard::promotion_zone(us);
            let empty = !pos.occupied();
            let occ = pos.occupied();
            for from in pos.pieces(us, PieceType::PAWN) {
                let pc = pos.piece_on(from);
                for to in attacks(pc, from, occ) & empty & zone {
                    list.push(Move::new_move(from, to, true, pc.promote()));
                }
            }
        }
        GenType::Quiets => {
            let empty = !pos.occupied();
            let us_c = us;
            let zone = Bitboard::promotion_zone(us_c);
            let occ = pos.occupied();
            // 歩以外の駒 + 歩の非「成り」部分（歩成はCaptures側）
            let movers = pos.color_bb(us)
                & !pos.pieces(us, PieceType::KING)
                & !pos.pieces(us, PieceType::PAWN);
            for from in movers {
                let pc = pos.piece_on(from);
                for to in attacks(pc, from, occ) & empty {
                    push_variants(us, from, to, pc, all, list);
                }
            }
            for from in pos.pieces(us, PieceType::PAWN) {
                let pc = pos.piece_on(from);
                for to in attacks(pc, from, occ) & empty {
                    if zone.test(to) {
                        // 成りはCaptures側。Allモードの不成のみここで積む
                        if all && to.rank().relative(us).0 >= 1 {
                            list.push(Move::new_move(from, to, false, pc));
                        }
                    } else {
                        list.push(Move::new_move(from, to, false, pc));
                    }
                }
            }
            generate_king_moves(pos, empty, list);
            generate_drops(pos, empty, list);
        }
    }
}

/// 合法手を生成する（王手の有無で自動分岐＋is_legalフィルタ）。
pub fn generate_legal(pos: &Position, all: bool, list: &mut MoveList) {
    let mut pseudo = MoveList::default();
    if pos.in_check() {
        generate(pos, GenType::Evasions, all, &mut pseudo);
    } else {
        generate(pos, GenType::NonEvasions, all, &mut pseudo);
    }
    for &m in &pseudo {
        if pos.is_legal(m) {
            list.push(m);
        }
    }
}

/// perft（ADR-0018）。深さ1はbulk counting。
pub fn perft(pos: &mut Position, depth: u32) -> u64 {
    debug_assert!(depth >= 1);
    let mut list = MoveList::default();
    generate_legal(pos, true, &mut list);
    if depth == 1 {
        return list.len() as u64;
    }
    let mut nodes = 0;
    for &m in &list {
        pos.do_move(m);
        nodes += perft(pos, depth - 1);
        pos.undo_move(m);
    }
    nodes
}

/// 素直なperft（--slow検証用）。深さ0まで潜って葉を数える。
pub fn perft_slow(pos: &mut Position, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    let mut list = MoveList::default();
    generate_legal(pos, true, &mut list);
    let mut nodes = 0;
    for &m in &list {
        pos.do_move(m);
        nodes += perft_slow(pos, depth - 1);
        pos.undo_move(m);
    }
    nodes
}
