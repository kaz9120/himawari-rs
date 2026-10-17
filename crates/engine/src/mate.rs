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
    Bitboard, Color, EffectCounts, EffectMap, Move, MoveList, Piece, PieceType, Position, Rank,
    Square, attacks, generate_legal,
};

/// 玉の近傍へ移動できる駒の位置（出典はやねうら王
/// mate1ply_without_effect.cpp:48のCHECK_CAND_BB）。
///
/// 占有なしの最大到達で作るので、どの占有に対しても上位集合になる。
struct CheckCand {
    /// 駒種ごとの移動元候補。金の動きをする5種はGOLDの1本で代表する
    by_pt: [Bitboard; PieceType::NB],
}

/// 移動で王手をかけうる駒種。成金4種は動きが金と同じなのでGOLDに寄せる。
/// 玉は候補にしない。
const MOVERS: [PieceType; 9] = [
    PieceType::GOLD,
    PieceType::PAWN,
    PieceType::LANCE,
    PieceType::KNIGHT,
    PieceType::SILVER,
    PieceType::BISHOP,
    PieceType::ROOK,
    PieceType::HORSE,
    PieceType::DRAGON,
];

/// MOVERSのうち、盤上の駒種と1対1で対応するもの（金の動きをする駒を除く）。
const PLAIN_MOVERS: [PieceType; 8] = [
    PieceType::PAWN,
    PieceType::LANCE,
    PieceType::KNIGHT,
    PieceType::SILVER,
    PieceType::BISHOP,
    PieceType::ROOK,
    PieceType::HORSE,
    PieceType::DRAGON,
];

static CHECK_CAND: std::sync::OnceLock<Vec<CheckCand>> = std::sync::OnceLock::new();

/// 色usの駒が、玉ksqの近傍（桂は桂の王手マス）へ動ける移動元の候補。
#[inline]
fn check_cand(us: Color, ksq: Square) -> &'static CheckCand {
    let t = CHECK_CAND.get_or_init(|| {
        let mut v = Vec::with_capacity(2 * 81);
        for c in [Color::Black, Color::White] {
            for ki in 0..81u8 {
                let ksq = Square::from_index(ki);
                // move_matesの移動先と同じ集合。占有によるマスクは掛けない
                let near = attacks::king_attacks(ksq);
                let knight_to = attacks::knight_attacks(c.flip(), ksq);
                let mut e = CheckCand {
                    by_pt: [Bitboard::EMPTY; PieceType::NB],
                };
                for pt in MOVERS {
                    let target = if pt == PieceType::KNIGHT {
                        knight_to
                    } else {
                        near
                    };
                    let pc = Piece::new(c, pt);
                    let mut bb = Bitboard::EMPTY;
                    for fi in 0..81u8 {
                        let from = Square::from_index(fi);
                        if from == ksq {
                            continue;
                        }
                        if !(attacks::attacks(pc, from, Bitboard::EMPTY) & target).is_empty() {
                            bb.set(from);
                        }
                    }
                    e.by_pt[pt.index()] = bb;
                }
                v.push(e);
            }
        }
        v
    });
    &t[us.index() * 81 + ksq.index()]
}

/// 升の周り5×5。玉の近傍3×3へ隣から利く駒は、ここに収まる。
static NEAR5: std::sync::OnceLock<[Bitboard; 81]> = std::sync::OnceLock::new();

#[inline]
fn near5(sq: Square) -> Bitboard {
    NEAR5.get_or_init(|| {
        std::array::from_fn(|i| {
            let center = Square::from_index(i as u8);
            let mut b = attacks::king_attacks(center) | Bitboard::from_square(center);
            for s in b {
                b |= attacks::king_attacks(s);
            }
            b
        })
    })[sq.index()]
}

/// 候補を利きの地図だけで落とすための前計算（ADR-0239の案E）。
///
/// 地図は指す前の手番側の利きである。指した後に手番側の利きが増えるのは、
/// 動かした駒の利きと、移動元が空いて伸びる飛び利きの2つしかない。玉を
/// 除いて伸びる利きは、指す前に王手がかかっていないので生まれない。
/// 移動元に飛び利きが当たっている手は地図で決めず、従来の判定へ回す。
/// ここで落とすのは詰みでないと決まった候補だけなので、返す手は変わらない。
struct MateMap {
    map: EffectMap,
    /// 玉の近傍で、相手の駒がなく手番側の利きもない升（地図で空いている逃げ場）
    escapes: Bitboard,
}

/// 1手詰めの候補を、手番側の利きだけで落とす判定（ADR-0239）。
/// 都度作る地図（`MateMap`）と、維持している利き数の表（`MateTable`）が実装する。
trait MateFilter {
    /// 移動元から飛び利きが伸びうるか。伸びうる手は利きだけで決めない。
    fn opens_line(&self, from: Square) -> bool;
    /// 玉の隣のtoへ動かした駒が、玉に取られるか（`opens_line` でない手に限る）。
    fn king_takes(&self, ksq: Square, to: Square, drop: bool) -> bool;
    /// 動かした駒の利きで埋まらない逃げ場が残るか（`opens_line` でない手に限る）。
    fn escape_left(&self, occ_no_king: Bitboard, to: Square, pc: Piece) -> bool;
}

/// 地図で空いている逃げ場のうち、動かした駒の利きで埋まらない升が残るか。
/// 駒の利きは玉を除いた移動後の占有で引くので、実際の利きの上位集合である。
#[inline(always)]
fn escape_left_of(escapes: Bitboard, occ_no_king: Bitboard, to: Square, pc: Piece) -> bool {
    if escapes.is_empty() {
        return false;
    }
    let covered = attacks::attacks(pc, to, occ_no_king) | Bitboard::from_square(to);
    !(escapes & !covered).is_empty()
}

impl MateFilter for MateMap {
    #[inline]
    fn opens_line(&self, from: Square) -> bool {
        self.map.slider.test(from)
    }

    /// 動かす駒はtoへ利いているので、2枚以上でなければほかに守りがない。
    /// 打つ手は打った駒が数に入っていないので、1枚以上で見る。
    #[inline]
    fn king_takes(&self, ksq: Square, to: Square, drop: bool) -> bool {
        attacks::king_attacks(ksq).test(to)
            && !(if drop { self.map.any } else { self.map.multi }).test(to)
    }

    #[inline]
    fn escape_left(&self, occ_no_king: Bitboard, to: Square, pc: Piece) -> bool {
        escape_left_of(self.escapes, occ_no_king, to, pc)
    }
}

/// 維持している利き数の表から読む版（ADR-0239の案B）。判定は `MateMap` と同じで、
/// 地図を作らずに表の升を直接読む。
struct MateTable<'a> {
    counts: &'a EffectCounts,
    us: Color,
    escapes: Bitboard,
}

impl<'a> MateTable<'a> {
    fn new(pos: &Position, counts: &'a EffectCounts, us: Color, ksq: Square) -> MateTable<'a> {
        let mut escapes = Bitboard::EMPTY;
        for s in attacks::king_attacks(ksq) & !pos.color_bb(us.flip()) {
            if counts.count(us, s) == 0 {
                escapes |= Bitboard::from_square(s);
            }
        }
        MateTable {
            counts,
            us,
            escapes,
        }
    }
}

impl MateFilter for MateTable<'_> {
    #[inline]
    fn opens_line(&self, from: Square) -> bool {
        self.counts.dirs(self.us, from) != 0
    }

    #[inline]
    fn king_takes(&self, ksq: Square, to: Square, drop: bool) -> bool {
        attacks::king_attacks(ksq).test(to)
            && self.counts.count(self.us, to) < if drop { 1 } else { 2 }
    }

    #[inline]
    fn escape_left(&self, occ_no_king: Bitboard, to: Square, pc: Piece) -> bool {
        escape_left_of(self.escapes, occ_no_king, to, pc)
    }
}

impl MateMap {
    /// 地図を使わない経路が渡す空の値。読まれない
    const EMPTY: MateMap = MateMap {
        map: EffectMap {
            any: Bitboard::EMPTY,
            multi: Bitboard::EMPTY,
            slider: Bitboard::EMPTY,
        },
        escapes: Bitboard::EMPTY,
    };

    fn new(pos: &Position, us: Color, ksq: Square) -> MateMap {
        let map = EffectMap::build_near(pos, us, near5(ksq));
        let escapes = attacks::king_attacks(ksq) & !pos.color_bb(us.flip()) & !map.any;
        MateMap { map, escapes }
    }
}

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

/// 移動先toの駒が玉に取られてしまうか。取られるなら詰みではない。
///
/// 成り・不成や打つ駒種によらず結果が同じなので、候補の列挙側で1回だけ
/// 判定する。is_mate_moveの玉の逃げ場の検査と同じことを、いちばん効く
/// 枝刈りとして前に出したものである。
#[inline]
fn king_captures_to(pos: &Position, us: Color, ksq: Square, to: Square, from_bb: Bitboard) -> bool {
    if !attacks::king_attacks(ksq).test(to) {
        return false;
    }
    // 玉が動くので占有から玉を抜く。toの駒は自分自身へ利かないので、
    // 移動先の利きを足す必要はない
    let occ = (pos.occupied() ^ from_bb) ^ Bitboard::from_square(ksq);
    (pos.attackers_to(us, to, occ) & !from_bb).is_empty()
}

/// 行き所のない駒になるので不成では入れないマス。
/// 歩・香は最終段、桂は最終2段。成れる場合は成りの手が別に作られる。
#[inline]
fn no_retreat(us: Color, pt: PieceType) -> Bitboard {
    let last = if us == Color::Black { 0 } else { 8 };
    let second = if us == Color::Black { 1 } else { 7 };
    match pt {
        PieceType::PAWN | PieceType::LANCE => Bitboard::rank(Rank(last)),
        PieceType::KNIGHT => Bitboard::rank(Rank(last)) | Bitboard::rank(Rank(second)),
        _ => Bitboard::EMPTY,
    }
}

/// 手mが詰みかを、駒を動かさずに判定する。
///
/// trueを返すなら確実に詰み（偽陽性なし）。合駒が成立しうる局面は
/// 安全側に倒してfalseを返すため、見逃しはあり得る。
///
/// mは擬似合法であることが前提。候補の列挙側が利きから作るので、
/// `pseudo_legal` の再検査はしない（行き所のない駒だけ列挙側で除く）。
fn is_mate_move(pos: &Position, m: Move) -> bool {
    debug_assert!(pos.pseudo_legal(m), "擬似合法でない手: {}", m.to_usi());
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

    if !pos.is_legal(m) {
        return false;
    }

    // 1. 王手になっているか。開き王手もここで拾える
    let checkers = our_attackers(pos, us, ksq, occ_after, from_bb, to, pc);
    if checkers.is_empty() {
        return false;
    }

    // 2. 玉の逃げ場。自駒のあるマスは取って逃げる手なので候補に残す。
    //    移動先toも（隣接していれば）ここで検査される
    let their_after = pos.color_bb(them) & !to_bb;
    for s in attacks::king_attacks(ksq) & !their_after {
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

    // 3. 玉以外の駒で王手駒を取る。取った後も王手が残るなら回避にならない
    for d in pos.attackers_to(them, csq, occ_after) & !to_bb & !ksq_bb {
        let o = occ_after ^ Bitboard::from_square(d);
        // csqにあった駒は取られたのでマスクで外す
        if (our_attackers(pos, us, ksq, o, from_bb, to, pc) & !csq_bb).is_empty() {
            return false;
        }
    }

    // 4. 合駒。玉と王手駒の間があるのは開き王手の場合だけ
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
fn drop_mates<const MAP: bool, F: MateFilter>(
    pos: &Position,
    us: Color,
    ksq: Square,
    mm: &F,
) -> Option<Move> {
    let hand = pos.hand(us);
    if hand.is_empty() {
        return None;
    }
    let them = us.flip();
    let empty = !pos.occupied() & Bitboard::ALL;
    let ksq_bb = Bitboard::from_square(ksq);
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
        } & empty
            & !no_retreat(us, pt);
        for to in targets {
            // 打つ手は移動元がないので、地図だけで決まる。玉に取られるかの
            // 判定は `king_captures_to` と同じ値になる
            if !MAP {
                if king_captures_to(pos, us, ksq, to, Bitboard::EMPTY) {
                    continue;
                }
            } else {
                if mm.king_takes(ksq, to, true) {
                    continue;
                }
                let occ_no_king = (pos.occupied() | Bitboard::from_square(to)) ^ ksq_bb;
                if mm.escape_left(occ_no_king, to, Piece::new(us, pt)) {
                    continue;
                }
            }
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
fn move_mates<const MAP: bool, F: MateFilter>(
    pos: &Position,
    us: Color,
    ksq: Square,
    froms: Bitboard,
    mm: &F,
) -> Option<Move> {
    let them = us.flip();
    let ksq_bb = Bitboard::from_square(ksq);
    let occ = pos.occupied();
    let ours = pos.color_bb(us);
    let near = attacks::king_attacks(ksq) & !ours;
    let knight_to = attacks::knight_attacks(them, ksq) & !ours;
    let zone = Bitboard::promotion_zone(us);
    for from in froms {
        let pc = pos.piece_on(from);
        let pt = pc.piece_type();
        let from_bb = Bitboard::from_square(from);
        let targets = attacks::attacks(pc, from, occ)
            & if pt == PieceType::KNIGHT {
                knight_to
            } else {
                near
            };
        // 不成では入れないマス。成りの手は別に作る
        let blocked = no_retreat(us, pt);
        // 移動元に飛び利きが当たっていなければ、候補を地図で落とせる
        let by_map = MAP && !mm.opens_line(from);
        for to in targets {
            if by_map {
                if mm.king_takes(ksq, to, false) {
                    continue;
                }
            } else if king_captures_to(pos, us, ksq, to, from_bb) {
                continue;
            }
            let occ_no_king = ((occ ^ from_bb) | Bitboard::from_square(to)) ^ ksq_bb;
            if pt.can_promote() && (zone.test(from) || zone.test(to)) {
                let m = Move::new_move(from, to, true, pc.promote());
                if !(by_map && mm.escape_left(occ_no_king, to, m.piece_after()))
                    && is_mate_move(pos, m)
                {
                    return Some(m);
                }
            }
            if !blocked.test(to) {
                let m = Move::new_move(from, to, false, pc);
                if !(by_map && mm.escape_left(occ_no_king, to, pc)) && is_mate_move(pos, m) {
                    return Some(m);
                }
            }
        }
    }
    None
}

/// 1手詰めがあればその手を返す。王手されていない局面で呼ぶこと。
/// 返した手は詰みであることが保証される（見逃しはあり得る）。
pub fn mate_1ply(pos: &Position) -> Option<Move> {
    let (us, ksq, froms) = candidates(pos)?;
    let mm = MateMap::new(pos, us, ksq);
    search_candidates::<true, _>(pos, us, ksq, froms, &mm)
}

/// `mate_1ply` の、維持している利き数の表から読む版（ADR-0239の案B）。
/// `counts` は `pos` の表であること。返す手は `mate_1ply` と同じ。
pub fn mate_1ply_with_counts(pos: &Position, counts: &EffectCounts) -> Option<Move> {
    let (us, ksq, froms) = candidates(pos)?;
    let mm = MateTable::new(pos, counts, us, ksq);
    search_candidates::<true, _>(pos, us, ksq, froms, &mm)
}

/// 利きの地図を使わない `mate_1ply`（ADR-0239より前の実装）。返す手が
/// `mate_1ply` と一致することをテストで確かめるために残す。
#[doc(hidden)]
pub fn mate_1ply_without_map(pos: &Position) -> Option<Move> {
    let (us, ksq, froms) = candidates(pos)?;
    search_candidates::<false, _>(pos, us, ksq, froms, &MateMap::EMPTY)
}

/// 手番・相手玉の升・玉の近傍へ届く駒の位置を返す。候補が1つもなければNone。
#[inline(always)]
fn candidates(pos: &Position) -> Option<(Color, Square, Bitboard)> {
    debug_assert!(!pos.in_check());
    let us = pos.side_to_move();
    let ksq = pos.king(us.flip());
    // 玉の近傍へ届かない駒は利きを作らずに捨てる。駒種ごとの候補との積を
    // 取るので、玉と、届かない駒はここで全部落ちる
    let cand = check_cand(us, ksq);
    let mut froms = pos.golds(us) & cand.by_pt[PieceType::GOLD.index()];
    for pt in PLAIN_MOVERS {
        froms |= pos.pieces(us, pt) & cand.by_pt[pt.index()];
    }
    if froms.is_empty() && pos.hand(us).is_empty() {
        return None;
    }
    Some((us, ksq, froms))
}

/// `MAP` が真なら、候補を利きで先に落とす。落とすのは詰みでないと
/// 決まった候補だけで、残りを調べる順は変えないので、返す手は同じになる。
#[inline(always)]
fn search_candidates<const MAP: bool, F: MateFilter>(
    pos: &Position,
    us: Color,
    ksq: Square,
    froms: Bitboard,
    mm: &F,
) -> Option<Move> {
    if !froms.is_empty()
        && let Some(m) = move_mates::<MAP, F>(pos, us, ksq, froms, mm)
    {
        return Some(m);
    }
    drop_mates::<MAP, F>(pos, us, ksq, mm)
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
