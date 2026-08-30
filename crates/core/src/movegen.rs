//! 指し手生成（ADR-0017）。
//!
//! 生成分類はCaptures（歩成含む）／Quiets／NonEvasions／Evasions。
//! `all = false`（Normal）は無意味な不成を生成せず、`all = true` は
//! 全合法手（perft・検証用）を生成する。

use crate::attacks::{aligned, attacks, between, king_attacks, pawn_attacks};
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
            // 香の不成は3段目以降で生成する（ADR-0176）。香の値打ちは前方へ
            // 貫通する利きで、成香は金の動きなので前1マスに縮む。3段目で
            // 不成にすれば貫通が残る。2段目の不成は1段目へしか行けず、横と
            // 後ろへ動ける成香に劣るので Normal では生成しない
            PieceType::LANCE => {
                if all {
                    rel >= 1
                } else {
                    rel >= 2
                }
            }
            // 桂の3段目への不成は Normal でも生成する（ADR-0173）。
            // 成桂は金の動きなので、桂の王手とは利きが違う。「成ると王手に
            // ならないが、不成なら王手になる」形が実戦で詰みの決め手になり、
            // floodgateの敗戦11局がこれだった。参照実装（やねうら王・Apery）も
            // 桂の不成だけは全生成フラグで条件付けていない。
            // 1〜2段目は不成だと行き所のない駒になるので `rel >= 2` は残す
            PieceType::KNIGHT => rel >= 2,
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

/// 玉以外の盤上の駒の合法手を、targetへ直接生成する。
///
/// pinされた駒（blockers_for_king）だけ、移動先を玉との整列で絞る。
/// pinされていない駒の移動はis_legalが常にtrueを返す集合なので、
/// 検査せずに積む。判定が手ごとから駒ごとに減る。
fn generate_board_moves_legal(pos: &Position, target: Bitboard, all: bool, list: &mut MoveList) {
    let us = pos.side_to_move();
    let occ = pos.occupied();
    let ksq = pos.king(us);
    let blockers = pos.blockers_for_king(us);
    let movers = pos.color_bb(us) & !pos.pieces(us, PieceType::KING);
    for from in movers {
        let pc = pos.piece_on(from);
        let att = attacks(pc, from, occ) & target;
        if blockers.test(from) {
            for to in att {
                if aligned(from, to, ksq) {
                    push_variants(us, from, to, pc, all, list);
                }
            }
        } else {
            for to in att {
                push_variants(us, from, to, pc, all, list);
            }
        }
    }
}

/// 玉の合法手をtargetへ直接生成する。
///
/// 移動先に敵の利きがなければ合法である。利きは玉自身を除いた占有で
/// 数える。玉が王手の線に沿って退く手を残さないためで、is_legalの
/// 玉の分岐と同じ判定になる。
fn generate_king_moves_legal(pos: &Position, target: Bitboard, list: &mut MoveList) {
    let us = pos.side_to_move();
    let them = us.flip();
    let from = pos.king(us);
    let pc = pos.piece_on(from);
    let occ_wo_king = pos.occupied() ^ Bitboard::from_square(from);
    for to in king_attacks(from) & target {
        if pos.attackers_to(them, to, occ_wo_king).is_empty() {
            list.push(Move::new_move(from, to, false, pc));
        }
    }
}

/// 駒打ちの合法手をtargetへ直接生成する。targetは空きマスの部分集合で
/// あること。
///
/// 不合法になりうる駒打ちは打ち歩詰めだけで、それは敵玉へ王手となる
/// 歩打ち、つまり敵玉の1つ手前への歩打ちに限られる。そのマスを含む
/// ときだけis_legalへ回し、他の駒打ちは検査せずに積む。
fn generate_drops_legal(pos: &Position, target: Bitboard, list: &mut MoveList) {
    let us = pos.side_to_move();
    let hand = pos.hand(us);
    if hand.is_empty() {
        return;
    }
    let them = us.flip();
    // 敵玉の1つ手前は、敵から見た歩の利き先として求まる
    let pawn_check = pawn_attacks(them, pos.king(them));
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
        if pt == PieceType::PAWN && !(mask & pawn_check).is_empty() {
            for to in mask {
                let m = Move::new_drop(pt, to, us);
                if !pawn_check.test(to) || pos.is_legal(m) {
                    list.push(m);
                }
            }
        } else {
            for to in mask {
                list.push(Move::new_drop(pt, to, us));
            }
        }
    }
}

/// 合法手を生成する（王手の有無で自動分岐）。
///
/// 擬似合法手を全数生成してis_legalで濾し直す二段構えは使わない。
/// 二段目の検査とコピーが生成時間の4〜7割を占めていたためで、合法性は
/// カテゴリ別の生成へ折り込む（issue #435）。盤上の駒はpinされた駒だけ
/// 整列検査、玉はattackers_to、駒打ちは王手となる歩打ちだけ打ち歩詰め
/// 検査を通す。
///
/// 生成順は従来の「generate→is_legalフィルタ」と一致する。一致は
/// perft既知値と、新旧生成列を突き合わせるテストで守る
/// （crates/core/tests/integration.rsのgenerate_legal_matches_filtered_pseudo）。
pub fn generate_legal(pos: &Position, all: bool, list: &mut MoveList) {
    let us = pos.side_to_move();
    if pos.in_check() {
        let checkers = pos.checkers();
        generate_king_moves_legal(pos, !pos.color_bb(us), list);
        if checkers.more_than_one() {
            return;
        }
        let checker = checkers.lsb();
        let block = between(pos.king(us), checker);
        generate_board_moves_legal(pos, block | checkers, all, list);
        generate_drops_legal(pos, block, list);
    } else {
        let target = !pos.color_bb(us);
        generate_board_moves_legal(pos, target, all, list);
        generate_king_moves_legal(pos, target, list);
        generate_drops_legal(pos, !pos.occupied(), list);
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
