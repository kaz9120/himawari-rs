//! 側のaccumulatorの入力を、表し方ごとに作る（ADR-0245）。
//!
//! 構成名の `e<E>` の後ろに書いた種類（`-f2_threat` など）の特徴を、種類の並びの順に
//! 表の行を続けて立てる。評価のたびに盤から作り直し、差分は持たない。学習側（PyO3）も
//! 同じ関数を通すので、学習と推論で特徴の作り方がずれない。
//!
//! 視点ごとに盤を反転し、自他は視点から見る。利きは盤上の生の利きで、玉の利きを含め、
//! ピンは無視する。その升の駒自身の利きは数えない。ADR-0237のF2と同じ数え方である。

use himawari_core::attacks::attacks;
use himawari_core::{Bitboard, Color, Piece, PieceType, Square, bonapiece};

use crate::nnue::{
    CONCAT, EFF_ACT, EFF_BASE, EFF_IN, EFF_KINDS, EFF_MAX_ACTIVE, EFF_OUT, EFF_PAIR, EFF_PIECES,
    NnueNetwork, effect_index, pair_activation,
};
use crate::nnue_acc::EvalBoard;

/// 種類の番号。build.rsの `SIDE_KINDS` の並びと同じにする。
pub const KIND_F2: usize = 0;
pub const KIND_L2: usize = 1;
pub const KIND_THERMO: usize = 2;
pub const KIND_MINATT: usize = 3;
pub const KIND_THREAT: usize = 4;
pub const KIND_ESCAPE: usize = 5;
pub const KIND_DROP: usize = 6;
pub const KIND_MOBILITY: usize = 7;
pub const KIND_KW5: usize = 8;
pub const KIND_HAND: usize = 9;
/// 習甦方式の玉×升×利き（ADR-0245の追記）。升に乗る駒は見ない。
pub const KIND_KE: usize = 10;
/// HalfKP（玉×BonaPiece）を側に入れる（ADR-0245の習甦方式）。主のHalfKPと同じ添字。
pub const KIND_KP: usize = 11;
pub const KINDS: usize = 12;

/// 最小の攻め駒の階級の数。なし・歩・香・桂・銀・金と成り駒・角・飛・馬・龍・玉。
const ATT_CLASSES: usize = 11;
/// 持ち駒の種類（`PieceType::HAND_KINDS` の順）。
const HAND: usize = 7;
/// 1種類の持ち駒で数える枚数の上限。歩の18枚に合わせる。
const HAND_MAX: usize = 18;

/// 種類ごとの表の行数。build.rsの `SIDE_KINDS` と同じ値で、一致はテストで確かめる。
pub const ROWS: [usize; KINDS] = [
    81 * EFF_PIECES * 16,
    81 * EFF_PIECES * 4,
    81 * EFF_PIECES + 81 * 2 * 3,
    81 * EFF_PIECES * ATT_CLASSES * ATT_CLASSES,
    28 * 81 * ATT_CLASSES * 2,
    2 * 8 * 5,
    2 * HAND * 4,
    28 * 8,
    2 * EFF_PIECES * 16 * 25,
    2 * HAND * HAND_MAX,
    81 * 81 * 16,
    81 * bonapiece::FE_END as usize,
];

/// 種類ごとの、1視点で立つ数の上限。
pub const MAX_ACTIVE: [usize; KINDS] = [
    81,
    81,
    81 + 81 * 2 * 3,
    81,
    38,
    16,
    2 * HAND * 4,
    38 * 8,
    50,
    38,
    81,
    38,
];

const _: () = {
    let (mut rows, mut max, mut i) = (0, 0, 0);
    while i < EFF_KINDS.len() {
        rows += ROWS[EFF_KINDS[i]];
        max += MAX_ACTIVE[EFF_KINDS[i]];
        i += 1;
    }
    assert!(rows == EFF_IN, "build.rsの SIDE_KINDS と ROWS が食い違う");
    assert!(
        max == EFF_MAX_ACTIVE,
        "build.rsの SIDE_KINDS と MAX_ACTIVE が食い違う"
    );
};

/// 構成の表の中で、HalfKP（`KIND_KP`）が始まる行。含まない構成では `None`。
/// 学習側は、この区画にだけ玉位置に依らない駒の成分（factorizer、ADR-0066）を足す。
pub const KP_BASE: Option<usize> = {
    let (mut base, mut i, mut found) = (0, 0, None);
    while i < EFF_KINDS.len() {
        if EFF_KINDS[i] == KIND_KP {
            found = Some(base);
        }
        base += ROWS[EFF_KINDS[i]];
        i += 1;
    }
    found
};

/// 特徴を作るのに要る盤の情報。升ごとの利き数と最小の攻め駒を、色ごとに1回だけ数える。
pub struct SideBoard {
    pc: [Piece; 81],
    hand: [[u8; HAND]; 2],
    king: [Square; 2],
    occ: Bitboard,
    color_bb: [Bitboard; 2],
    cnt: [[u8; 81]; 2],
    minatt: [[u8; 81]; 2],
}

/// 最小の攻め駒の階級。0は駒でない。
fn att_class(pt: PieceType) -> u8 {
    match pt {
        PieceType::PAWN => 1,
        PieceType::LANCE => 2,
        PieceType::KNIGHT => 3,
        PieceType::SILVER => 4,
        PieceType::BISHOP => 6,
        PieceType::ROOK => 7,
        PieceType::HORSE => 8,
        PieceType::DRAGON => 9,
        PieceType::KING => 10,
        pt if pt.is_gold_like() => 5,
        _ => 0,
    }
}

/// 視点cから見た升。後手の視点は盤を180度回す。逆写像も同じ式になる。
#[inline]
fn view(sq: Square, c: Color) -> Square {
    if c == Color::Black { sq } else { sq.inv() }
}

/// 視点cから見た駒。0は空き、自分の14種を1〜14、相手を15〜28へ写す（`effect_active` と同じ）。
#[inline]
fn code(pc: Piece, c: Color) -> usize {
    if pc.is_empty() {
        return 0;
    }
    let kind = usize::from(pc.piece_type().0) - 1;
    if pc.color() == c { kind } else { kind + 14 }
}

impl SideBoard {
    pub fn new<B: EvalBoard + ?Sized>(b: &B) -> SideBoard {
        let mut s = SideBoard {
            pc: [Piece::EMPTY; 81],
            hand: [[0; HAND]; 2],
            king: [b.king(Color::Black), b.king(Color::White)],
            occ: Bitboard::EMPTY,
            color_bb: [Bitboard::EMPTY; 2],
            cnt: [[0; 81]; 2],
            minatt: [[0; 81]; 2],
        };
        for i in 0..81u8 {
            let sq = Square::from_index(i);
            let pc = b.piece_on(sq);
            s.pc[sq.index()] = pc;
            if !pc.is_empty() {
                s.occ.set(sq);
                s.color_bb[pc.color().index()].set(sq);
            }
        }
        for c in [Color::Black, Color::White] {
            for (hi, &pt) in PieceType::HAND_KINDS.iter().enumerate() {
                s.hand[c.index()][hi] = b.hand_count(c, pt).min(u32::from(u8::MAX)) as u8;
            }
        }
        for sq in s.occ {
            let pc = s.pc[sq.index()];
            let ci = pc.color().index();
            let cl = att_class(pc.piece_type());
            for to in attacks(pc, sq, s.occ) {
                s.cnt[ci][to.index()] += 1;
                let m = &mut s.minatt[ci][to.index()];
                if *m == 0 || cl < *m {
                    *m = cl;
                }
            }
        }
        s
    }

    /// 構成の種類（`EFF_KINDS`）の特徴を、視点cで `out` へ足す。添字は表全体の通し番号。
    pub fn active(&self, c: Color, out: &mut Vec<u32>) {
        let mut base = 0u32;
        for &k in EFF_KINDS {
            self.kind(k, c, base, out);
            base += ROWS[k] as u32;
        }
    }

    /// 種類kの特徴を、視点cで、表の起点baseから足す。
    pub fn kind(&self, k: usize, c: Color, base: u32, out: &mut Vec<u32>) {
        let (ci, oi) = (c.index(), c.flip().index());
        let mut push = |i: usize| out.push(base + i as u32);
        match k {
            KIND_F2 => {
                for i in 0..81 {
                    let v = view(Square::from_index(i as u8), c).index();
                    push(
                        effect_index(v, code(self.pc[i], c), self.cnt[ci][i], self.cnt[oi][i])
                            as usize,
                    );
                }
            }
            KIND_L2 => {
                for i in 0..81 {
                    let v = view(Square::from_index(i as u8), c).index();
                    let (a, b) = (
                        usize::from(self.cnt[ci][i] > 0),
                        usize::from(self.cnt[oi][i] > 0),
                    );
                    push(((v * EFF_PIECES + code(self.pc[i], c)) * 2 + a) * 2 + b);
                }
            }
            KIND_THERMO => {
                for i in 0..81 {
                    let v = view(Square::from_index(i as u8), c).index();
                    push(v * EFF_PIECES + code(self.pc[i], c));
                    for (side, n) in [(0, self.cnt[ci][i]), (1, self.cnt[oi][i])] {
                        for t in 1..=n.min(3) {
                            push(81 * EFF_PIECES + (v * 2 + side) * 3 + usize::from(t - 1));
                        }
                    }
                }
            }
            KIND_MINATT => {
                for i in 0..81 {
                    let v = view(Square::from_index(i as u8), c).index();
                    let (a, b) = (
                        usize::from(self.minatt[ci][i]),
                        usize::from(self.minatt[oi][i]),
                    );
                    push(
                        ((v * EFF_PIECES + code(self.pc[i], c)) * ATT_CLASSES + a) * ATT_CLASSES
                            + b,
                    );
                }
            }
            KIND_THREAT => {
                for sq in self.occ {
                    let pc = self.pc[sq.index()];
                    if pc.piece_type() == PieceType::KING {
                        continue;
                    }
                    let (me, them) = (pc.color().index(), pc.color().flip().index());
                    let att = usize::from(self.minatt[them][sq.index()]);
                    if att == 0 {
                        continue;
                    }
                    let def = usize::from(self.cnt[me][sq.index()] > 0);
                    let v = view(sq, c).index();
                    push((((code(pc, c) - 1) * 81 + v) * ATT_CLASSES + att) * 2 + def);
                }
            }
            KIND_ESCAPE => {
                for (tag, kc) in [(0, c), (1, c.flip())] {
                    let ks = self.king[kc.index()];
                    if ks.is_none() {
                        continue;
                    }
                    let vk = view(ks, c);
                    let (kf, kr) = (i32::from(vk.file().0), i32::from(vk.rank().0));
                    let mut d = 0;
                    for df in -1..=1 {
                        for dr in -1..=1 {
                            if df == 0 && dr == 0 {
                                continue;
                            }
                            let (f, r) = (kf + df, kr + dr);
                            let state = if !(0..9).contains(&f) || !(0..9).contains(&r) {
                                0
                            } else {
                                let s = view(Square::from_index((f * 9 + r) as u8), c).index();
                                let pc = self.pc[s];
                                if !pc.is_empty() && pc.color() == kc {
                                    1
                                } else if !pc.is_empty() {
                                    2
                                } else if self.cnt[kc.flip().index()][s] > 0 {
                                    3
                                } else {
                                    4
                                }
                            };
                            push((tag * 8 + d) * 5 + state);
                            d += 1;
                        }
                    }
                }
            }
            KIND_DROP => {
                for (tag, side) in [(0, c), (1, c.flip())] {
                    let ek = self.king[side.flip().index()];
                    if ek.is_none() {
                        continue;
                    }
                    let (kf, kr) = (i32::from(ek.file().0), i32::from(ek.rank().0));
                    let mut safe = 0;
                    for f in (kf - 2).max(0)..=(kf + 2).min(8) {
                        for r in (kr - 2).max(0)..=(kr + 2).min(8) {
                            let s = (f * 9 + r) as usize;
                            if self.pc[s].is_empty() && self.cnt[side.flip().index()][s] == 0 {
                                safe += 1;
                            }
                        }
                    }
                    for hi in 0..HAND {
                        if self.hand[side.index()][hi] == 0 {
                            continue;
                        }
                        for t in 0..safe.min(4) {
                            push((tag * HAND + hi) * 4 + t);
                        }
                    }
                }
            }
            KIND_MOBILITY => {
                for sq in self.occ {
                    let pc = self.pc[sq.index()];
                    let pt = pc.piece_type();
                    if pt == PieceType::PAWN || pt == PieceType::KING {
                        continue;
                    }
                    let n =
                        (attacks(pc, sq, self.occ) & !self.color_bb[pc.color().index()]).count();
                    for t in 0..n.min(8) as usize {
                        push((code(pc, c) - 1) * 8 + t);
                    }
                }
            }
            KIND_KW5 => {
                for (tag, kc) in [(0, c), (1, c.flip())] {
                    let ks = self.king[kc.index()];
                    if ks.is_none() {
                        continue;
                    }
                    let vk = view(ks, c);
                    let (kf, kr) = (i32::from(vk.file().0), i32::from(vk.rank().0));
                    for df in -2..=2i32 {
                        for dr in -2..=2i32 {
                            let (f, r) = (kf + df, kr + dr);
                            if !(0..9).contains(&f) || !(0..9).contains(&r) {
                                continue;
                            }
                            let s = view(Square::from_index((f * 9 + r) as u8), c).index();
                            let own = usize::from(self.cnt[ci][s].min(3));
                            let opp = usize::from(self.cnt[oi][s].min(3));
                            let rel = ((df + 2) * 5 + (dr + 2)) as usize;
                            push(
                                (((tag * EFF_PIECES + code(self.pc[s], c)) * 4 + own) * 4 + opp)
                                    * 25
                                    + rel,
                            );
                        }
                    }
                }
            }
            KIND_HAND => {
                for (tag, side) in [(0, c), (1, c.flip())] {
                    for hi in 0..HAND {
                        for i in 0..usize::from(self.hand[side.index()][hi]).min(HAND_MAX) {
                            push((tag * HAND + hi) * HAND_MAX + i);
                        }
                    }
                }
            }
            KIND_KE => {
                // 視点側の玉の升と、升ごとの自分の利き・相手の利き（3で打ち切り）の組
                let ks = self.king[ci];
                if ks.is_none() {
                    return;
                }
                let kv = view(ks, c).index();
                for i in 0..81 {
                    let v = view(Square::from_index(i as u8), c).index();
                    let own = usize::from(self.cnt[ci][i].min(3));
                    let opp = usize::from(self.cnt[oi][i].min(3));
                    push(((kv * 81 + v) * 4 + own) * 4 + opp);
                }
            }
            KIND_KP => {
                // 主のHalfKPと同じ添字（`halfkp_index`）。玉の無い局面では立てない
                let ks = self.king[ci];
                if ks.is_none() {
                    return;
                }
                for sq in self.occ {
                    let pc = self.pc[sq.index()];
                    if pc.piece_type() == PieceType::KING {
                        continue;
                    }
                    push(
                        bonapiece::halfkp_index(c, ks, bonapiece::board_bona_piece(c, pc, sq))
                            as usize,
                    );
                }
                for owner in [Color::Black, Color::White] {
                    for (hi, &pt) in PieceType::HAND_KINDS.iter().enumerate() {
                        for i in 1..=u32::from(self.hand[owner.index()][hi]) {
                            push(bonapiece::halfkp_index(
                                c,
                                ks,
                                bonapiece::hand_bona_piece(c, owner, pt, i),
                            ) as usize);
                        }
                    }
                }
            }
            _ => unreachable!("側の入力の種類が不明: {k}"),
        }
    }
}

/// 学習側の口。視点cの特徴を、`EFF_MAX_ACTIVE` 個へ `EFF_IN`（詰め物の行）で埋めて返す。
pub fn active_padded<B: EvalBoard + ?Sized>(b: &B, c: Color) -> Vec<u32> {
    let mut v = Vec::with_capacity(EFF_MAX_ACTIVE);
    SideBoard::new(b).active(c, &mut v);
    debug_assert!(v.len() <= EFF_MAX_ACTIVE);
    v.resize(EFF_MAX_ACTIVE, EFF_IN as u32);
    v
}

/// 側のaccumulatorを特徴から作り直し、連結ベクトルの側の部分（両視点、手番視点が先）を書く。
pub fn fill<B: EvalBoard + ?Sized>(net: &NnueNetwork, b: &B, concat: &mut [u8; CONCAT]) {
    let sb = SideBoard::new(b);
    let stm = b.side_to_move();
    let mut feats = Vec::with_capacity(EFF_MAX_ACTIVE);
    for (k, c) in [(0usize, stm), (1, stm.flip())] {
        feats.clear();
        sb.active(c, &mut feats);
        let acc = accumulate(net, &feats);
        let out = &mut concat[EFF_BASE + k * EFF_ACT..EFF_BASE + (k + 1) * EFF_ACT];
        if EFF_PAIR {
            let h = EFF_OUT / 2;
            for (o, x) in out.iter_mut().enumerate() {
                *x = pair_activation(acc[o], acc[o + h]);
            }
        } else {
            for (x, &a) in out.iter_mut().zip(&acc) {
                *x = a.clamp(0, 127) as u8;
            }
        }
    }
}

/// バイアスに特徴の行を足す。i32で足すので桁あふれしない。
pub fn accumulate(net: &NnueNetwork, feats: &[u32]) -> [i32; EFF_OUT] {
    let mut acc = [0i32; EFF_OUT];
    for (a, &b) in acc.iter_mut().zip(&net.eff_b) {
        *a = i32::from(b);
    }
    for &f in feats {
        let at = f as usize * EFF_OUT;
        for (a, &w) in acc.iter_mut().zip(&net.eff_w[at..at + EFF_OUT]) {
            *a += i32::from(w);
        }
    }
    acc
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use himawari_core::{MoveList, Position, generate_legal};

    /// 開始局面から乱数で指し進めた局面。終盤寄りの局面も入るよう長めに進める。
    fn positions() -> Vec<Position> {
        let mut out = Vec::new();
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for game in 0..12 {
            let mut pos = Position::from_sfen(himawari_core::SFEN_STARTPOS).unwrap();
            for ply in 0..(40 + game * 12) {
                let mut moves = MoveList::default();
                generate_legal(&pos, true, &mut moves);
                if moves.is_empty() {
                    break;
                }
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let m = moves.as_slice()[(seed % moves.len() as u64) as usize];
                pos.do_move(m);
                if ply % 7 == 0 {
                    out.push(pos.clone());
                }
            }
        }
        out
    }

    #[test]
    fn every_kind_stays_in_its_rows_and_cap() {
        for pos in positions() {
            let sb = SideBoard::new(&pos);
            for k in 0..KINDS {
                for c in [Color::Black, Color::White] {
                    let mut v = Vec::new();
                    sb.kind(k, c, 0, &mut v);
                    assert!(
                        v.len() <= MAX_ACTIVE[k],
                        "種類{k}が上限を超えた: {} {}",
                        v.len(),
                        pos.to_sfen()
                    );
                    for &i in &v {
                        assert!(
                            (i as usize) < ROWS[k],
                            "種類{k}の添字{i}が行数{}を超えた",
                            ROWS[k]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn f2_kind_matches_effect_active() {
        for pos in positions() {
            let counts = crate::nnue::effect_counts(&pos);
            let sb = SideBoard::new(&pos);
            for c in [Color::Black, Color::White] {
                let mut want = [0u32; 81];
                crate::nnue::effect_active(&pos, c, &counts, &mut want);
                let mut got = Vec::new();
                sb.kind(KIND_F2, c, 0, &mut got);
                let mut want = want.to_vec();
                want.sort_unstable();
                got.sort_unstable();
                assert_eq!(got, want, "{}", pos.to_sfen());
            }
        }
    }

    #[test]
    fn kp_kind_matches_halfkp_active() {
        for pos in positions() {
            let sb = SideBoard::new(&pos);
            for c in [Color::Black, Color::White] {
                let mut want = Vec::new();
                crate::nnue::halfkp_active(&pos, c, &mut want);
                let mut got = Vec::new();
                sb.kind(KIND_KP, c, 0, &mut got);
                want.sort_unstable();
                got.sort_unstable();
                assert_eq!(got, want, "{}", pos.to_sfen());
            }
        }
    }

    /// 180度回して先後を入れ替えた局面は、視点を入れ替えれば同じ特徴になる。
    #[test]
    fn every_kind_is_symmetric_under_color_flip() {
        for pos in positions() {
            let flipped = Position::from_sfen(&flip_sfen(&pos.to_sfen())).unwrap();
            let (a, b) = (SideBoard::new(&pos), SideBoard::new(&flipped));
            for k in 0..KINDS {
                for c in [Color::Black, Color::White] {
                    let (mut x, mut y) = (Vec::new(), Vec::new());
                    a.kind(k, c, 0, &mut x);
                    b.kind(k, c.flip(), 0, &mut y);
                    x.sort_unstable();
                    y.sort_unstable();
                    assert_eq!(x, y, "種類{k}が先後の入れ替えで変わる: {}", pos.to_sfen());
                }
            }
        }
    }

    /// SFENの盤を180度回し、駒の先後と持ち駒の先後、手番を入れ替える。
    /// 盤を180度回して先後を入れ替えたSFEN。部分モデルのテストも使う。
    pub(crate) fn flip_sfen(sfen: &str) -> String {
        let parts: Vec<&str> = sfen.split_whitespace().collect();
        let swap = |ch: char| {
            if ch.is_ascii_uppercase() {
                ch.to_ascii_lowercase()
            } else if ch.is_ascii_lowercase() {
                ch.to_ascii_uppercase()
            } else {
                ch
            }
        };
        // 盤: 段の順と段の中の並びを逆にする。成りの印 '+' は駒の前に付け直す
        let rows: Vec<String> = parts[0]
            .split('/')
            .rev()
            .map(|row| {
                let mut tokens: Vec<String> = Vec::new();
                let mut promo = false;
                for ch in row.chars() {
                    if ch == '+' {
                        promo = true;
                        continue;
                    }
                    let t = if promo {
                        format!("+{}", swap(ch))
                    } else {
                        swap(ch).to_string()
                    };
                    promo = false;
                    tokens.push(t);
                }
                tokens.reverse();
                tokens.concat()
            })
            .collect();
        let side = if parts[1] == "b" { "w" } else { "b" };
        let hand: String = if parts[2] == "-" {
            "-".to_string()
        } else {
            parts[2].chars().map(swap).collect()
        };
        format!(
            "{} {} {} {}",
            rows.join("/"),
            side,
            hand,
            parts.get(3).unwrap_or(&"1")
        )
    }

    #[test]
    fn fill_matches_naive_sum() {
        let net = NnueNetwork::random(7);
        for pos in positions() {
            let mut concat = [0u8; CONCAT];
            fill(&net, &pos, &mut concat);
            let stm = pos.side_to_move();
            for (k, c) in [(0usize, stm), (1, stm.flip())] {
                let mut feats = Vec::new();
                SideBoard::new(&pos).active(c, &mut feats);
                let mut acc: Vec<i32> = net.eff_b.iter().map(|&b| i32::from(b)).collect();
                for &f in &feats {
                    let row = &net.eff_w[f as usize * EFF_OUT..(f as usize + 1) * EFF_OUT];
                    for (a, &w) in acc.iter_mut().zip(row) {
                        *a += i32::from(w);
                    }
                }
                for o in 0..EFF_ACT {
                    let want = if EFF_PAIR {
                        let (x, y) = (acc[o].clamp(0, 127), acc[o + EFF_OUT / 2].clamp(0, 127));
                        ((x * y + 64) >> 7) as u8
                    } else {
                        acc[o].clamp(0, 127) as u8
                    };
                    assert_eq!(concat[EFF_BASE + k * EFF_ACT + o], want);
                }
            }
        }
    }
}
