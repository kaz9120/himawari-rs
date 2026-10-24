//! 玉の近傍の部分モデル（ADR-0246）。
//!
//! 玉を中心にした5×5の窓を、玉の側から見た向きで25升のトークンにし、小さなMLPに
//! 通す。升のトークンは「駒の状態30通り×自分の利き4段階×相手の利き4段階」の480通りで、
//! 窓の外の情報として相手の持ち駒7種と手番を足す。両玉に同じモデルを当て、出力を
//! 連結ベクトルの末尾（`SUB_BASE` 以降）へ、手番側の玉・非手番側の玉の順に置く。
//!
//! 学習側は埋め込み16次元→並べて400次元→32→`SUB_OUT` の形で持つ。埋め込みと第1層は
//! どちらも線形なので、読み込みで「升の位置×トークン」ごとの32次元の行へ畳み込む。
//! 推論は25行を足すだけになる。重みはf32のまま持ち、差分計算もキャッシュもしない。

use himawari_core::{Color, PieceType, Square};

use crate::nnue::{CONCAT, EFF_ACT, FT_OUT, SUB_OUT};
use crate::nnue_acc::EvalBoard;

/// 窓の升の数（5×5）。
pub const WIN: usize = 25;
/// 升の駒の状態。盤外・空き・自分の14種・相手の14種。
pub const STATES: usize = 30;
/// 升のトークンの数。駒の状態×自分の利き4段階×相手の利き4段階。
pub const TOKENS: usize = STATES * 16;
/// 升の埋め込みの次元（学習側だけが持つ）。
pub const EMB: usize = 16;
/// 窓の外の入力。相手の持ち駒7種と、玉の側の手番か。
pub const EXTRA: usize = 8;
/// 第1層の入力の幅（学習側の形）。
pub const SUB_IN: usize = WIN * EMB + EXTRA;
/// 中間の幅。
pub const HID: usize = 32;
/// 1つの玉について学習側へ渡す整数の数。25升のトークンと窓の外の8つ。
pub const SUB_LEN: usize = WIN + EXTRA;
/// 連結ベクトルの中で部分モデルが占める幅。両玉ぶんを32の倍数へ切り上げ、余りは0のまま置く。
pub const SUB_SLOT: usize = if SUB_OUT == 0 {
    0
} else {
    (2 * SUB_OUT).next_multiple_of(32)
};
/// 連結ベクトルの中で部分モデルの出力が始まる位置。
pub const SUB_BASE: usize = FT_OUT + 2 * EFF_ACT;
/// ファイルに載る重みの数（学習側の形のf32）。
/// 埋め込み・第1層の重みとバイアス・第2層の重みとバイアスの順に並ぶ。
pub const SUB_PARAMS: usize = if SUB_OUT == 0 {
    0
} else {
    TOKENS * EMB + HID * SUB_IN + HID + SUB_OUT * HID + SUB_OUT
};

const _: () = assert!(SUB_BASE + SUB_SLOT == CONCAT);

/// 持ち駒の枚数を割る数（`PieceType::HAND_KINDS` の順）。その駒の最大の枚数で、入力が0〜1に収まる。
pub fn hand_scale(pt: PieceType) -> f32 {
    match pt {
        PieceType::PAWN => 18.0,
        PieceType::BISHOP | PieceType::ROOK => 2.0,
        _ => 4.0,
    }
}

/// 重み一式。`raw` がファイルの形で、残りは読み込みで作る派生表。
#[derive(Clone, Default)]
pub struct SubNet {
    /// 学習側の形のf32（`SUB_PARAMS` 個）。書き出しと重みのハッシュに使う。
    pub raw: Vec<f32>,
    /// 埋め込みを第1層へ畳み込んだ表。`table[(pos * TOKENS + tok) * HID + j]`。
    table: Vec<f32>,
    /// 窓の外の入力の重み。`wx[e * HID + j]`。
    wx: Vec<f32>,
    b1: Vec<f32>,
    /// 第2層。`w2[o * HID + j]`。
    w2: Vec<f32>,
    b2: Vec<f32>,
}

impl SubNet {
    /// ファイルの形の重みから派生表を作る。部分モデルを持たない構成では空を返す。
    pub fn from_raw(raw: Vec<f32>) -> SubNet {
        assert_eq!(raw.len(), SUB_PARAMS, "部分モデルの重みの数が合わない");
        if SUB_OUT == 0 {
            return SubNet::default();
        }
        let (emb, rest) = raw.split_at(TOKENS * EMB);
        let (w1, rest) = rest.split_at(HID * SUB_IN);
        let (b1, rest) = rest.split_at(HID);
        let (w2, b2) = rest.split_at(SUB_OUT * HID);
        let mut table = vec![0f32; WIN * TOKENS * HID];
        for pos in 0..WIN {
            for tok in 0..TOKENS {
                let e = &emb[tok * EMB..(tok + 1) * EMB];
                let row = &mut table[(pos * TOKENS + tok) * HID..(pos * TOKENS + tok + 1) * HID];
                for (j, t) in row.iter_mut().enumerate() {
                    let w = &w1[j * SUB_IN + pos * EMB..j * SUB_IN + (pos + 1) * EMB];
                    *t = w.iter().zip(e).map(|(a, b)| a * b).sum();
                }
            }
        }
        let mut wx = vec![0f32; EXTRA * HID];
        for e in 0..EXTRA {
            for j in 0..HID {
                wx[e * HID + j] = w1[j * SUB_IN + WIN * EMB + e];
            }
        }
        SubNet {
            table,
            wx,
            b1: b1.to_vec(),
            w2: w2.to_vec(),
            b2: b2.to_vec(),
            raw,
        }
    }

    /// テスト用の乱数重み。値は学習の初期値と同じくらいの大きさにする。
    pub fn random(seed: u64) -> SubNet {
        let mut s = seed.max(1);
        let raw = (0..SUB_PARAMS)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                ((s % 2001) as f32 - 1000.0) / 4000.0
            })
            .collect();
        SubNet::from_raw(raw)
    }
}

/// 色kcの玉の窓を、玉の側から見た向きで `SUB_LEN` 個の整数にする。
///
/// 前の25個が升のトークンで、玉の側から見て段の小さい（敵陣の）側の筋から順に並ぶ。
/// 後手の玉は盤を180度回してから読むので、先後で同じ形が同じ並びになる。
/// 残りの8個は相手の持ち駒の枚数7種と、玉の側の手番なら1。玉の無い局面は25升とも盤外にする。
pub fn inputs<B: EvalBoard + ?Sized>(b: &B, kc: Color, out: &mut [u16; SUB_LEN]) {
    let levels = b.effect_levels();
    let ks = b.king(kc);
    let view = |sq: Square| if kc == Color::Black { sq } else { sq.inv() };
    for (i, t) in out[..WIN].iter_mut().enumerate() {
        *t = 0;
        if ks.is_none() {
            continue;
        }
        let vk = view(ks);
        let (df, dr) = ((i / 5) as i32 - 2, (i % 5) as i32 - 2);
        let (f, r) = (i32::from(vk.file().0) + df, i32::from(vk.rank().0) + dr);
        if !(0..9).contains(&f) || !(0..9).contains(&r) {
            continue;
        }
        let sq = view(Square::from_index((f * 9 + r) as u8));
        let pc = b.piece_on(sq);
        // 空きは1、駒は側の入力と同じ番号（自分の14種を1〜14、相手を15〜28）に1を足す
        let state = if pc.is_empty() {
            1
        } else {
            let kind = usize::from(pc.piece_type().0) - 1;
            1 + if pc.color() == kc { kind } else { kind + 14 }
        };
        let own = usize::from(levels.level(kc, sq));
        let opp = usize::from(levels.level(kc.flip(), sq));
        *t = ((state * 4 + own) * 4 + opp) as u16;
    }
    for (hi, &pt) in PieceType::HAND_KINDS.iter().enumerate() {
        out[WIN + hi] = b.hand_count(kc.flip(), pt).min(u32::from(u16::MAX)) as u16;
    }
    out[WIN + 7] = u16::from(b.side_to_move() == kc);
}

/// 1つの玉の窓から、0〜127の出力を `SUB_OUT` 個作る。
pub fn forward(net: &SubNet, inp: &[u16; SUB_LEN], out: &mut [u8]) {
    debug_assert_eq!(out.len(), SUB_OUT);
    let mut h = [0f32; HID];
    h.copy_from_slice(&net.b1);
    for (pos, &tok) in inp[..WIN].iter().enumerate() {
        let at = (pos * TOKENS + usize::from(tok)) * HID;
        for (a, &w) in h.iter_mut().zip(&net.table[at..at + HID]) {
            *a += w;
        }
    }
    for (e, &pt) in PieceType::HAND_KINDS.iter().enumerate() {
        let x = f32::from(inp[WIN + e]) / hand_scale(pt);
        for (a, &w) in h.iter_mut().zip(&net.wx[e * HID..(e + 1) * HID]) {
            *a += w * x;
        }
    }
    let x = f32::from(inp[WIN + 7]);
    for (a, &w) in h.iter_mut().zip(&net.wx[7 * HID..8 * HID]) {
        *a += w * x;
    }
    for a in &mut h {
        *a = a.clamp(0.0, 1.0);
    }
    for (o, y) in out.iter_mut().enumerate() {
        let w = &net.w2[o * HID..(o + 1) * HID];
        let v = net.b2[o] + w.iter().zip(&h).map(|(a, b)| a * b).sum::<f32>();
        *y = (v.clamp(0.0, 1.0) * 127.0).round() as u8;
    }
}

/// 連結ベクトルの部分モデルの区画を埋める。手番側の玉が先に並ぶ。持たない構成では何もしない。
pub fn fill<B: EvalBoard + ?Sized>(net: &SubNet, b: &B, concat: &mut [u8; CONCAT]) {
    if SUB_OUT == 0 {
        return;
    }
    let stm = b.side_to_move();
    let mut inp = [0u16; SUB_LEN];
    for (k, c) in [(0usize, stm), (1, stm.flip())] {
        inputs(b, c, &mut inp);
        let at = SUB_BASE + k * SUB_OUT;
        forward(net, &inp, &mut concat[at..at + SUB_OUT]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use himawari_core::{MoveList, Position, generate_legal};

    fn positions() -> Vec<Position> {
        let mut out = Vec::new();
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for game in 0..8 {
            let mut pos = Position::from_sfen(himawari_core::SFEN_STARTPOS).unwrap();
            for ply in 0..(40 + game * 15) {
                let mut moves = MoveList::default();
                generate_legal(&pos, true, &mut moves);
                if moves.is_empty() {
                    break;
                }
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                pos.do_move(moves.as_slice()[(seed % moves.len() as u64) as usize]);
                if ply % 9 == 0 {
                    out.push(pos.clone());
                }
            }
        }
        out
    }

    /// 番号の端の駒（相手の飛車）が窓に入っても、トークンは表の範囲に収まる。
    #[test]
    fn tokens_stay_in_the_table() {
        let pos = Position::from_sfen("4k4/9/9/9/9/9/9/4r4/4K4 b - 1").unwrap();
        let mut a = [0u16; SUB_LEN];
        inputs(&pos, Color::Black, &mut a);
        let top = a[..WIN].iter().map(|&t| usize::from(t)).max().unwrap();
        assert_eq!(top / 16, STATES - 1, "相手の飛車が最後の状態にならない");
    }

    /// 180度回して先後を入れ替えた局面では、玉ごとの入力が入れ替わるだけになる。
    #[test]
    fn inputs_follow_the_king_owner() {
        for pos in positions() {
            let flipped = Position::from_sfen(&crate::side_feat::tests::flip_sfen(&pos.to_sfen()))
                .expect("反転した局面");
            for c in [Color::Black, Color::White] {
                let (mut a, mut b) = ([0u16; SUB_LEN], [0u16; SUB_LEN]);
                inputs(&pos, c, &mut a);
                inputs(&flipped, c.flip(), &mut b);
                assert_eq!(a, b, "{}", pos.to_sfen());
                assert!(a[..WIN].iter().all(|&t| usize::from(t) < TOKENS));
                // 窓の中央は自玉で、自分の利きは数えない
                assert_eq!(
                    usize::from(a[12]) / 16,
                    usize::from(PieceType::KING.0),
                    "中央が自玉でない"
                );
            }
        }
    }

    /// 畳み込んだ表で計算した値が、学習側の形（埋め込みを並べて第1層に通す）と一致する。
    #[test]
    fn folded_forward_matches_the_unfolded_form() {
        if SUB_OUT == 0 {
            return;
        }
        let net = SubNet::random(7);
        let raw = &net.raw;
        let (emb, rest) = raw.split_at(TOKENS * EMB);
        let (w1, rest) = rest.split_at(HID * SUB_IN);
        let (b1, rest) = rest.split_at(HID);
        let (w2, b2) = rest.split_at(SUB_OUT * HID);
        for pos in positions() {
            let mut inp = [0u16; SUB_LEN];
            inputs(&pos, pos.side_to_move(), &mut inp);
            let mut x = vec![0f32; SUB_IN];
            for p in 0..WIN {
                let t = usize::from(inp[p]);
                x[p * EMB..(p + 1) * EMB].copy_from_slice(&emb[t * EMB..(t + 1) * EMB]);
            }
            for (e, &pt) in PieceType::HAND_KINDS.iter().enumerate() {
                x[WIN * EMB + e] = f32::from(inp[WIN + e]) / hand_scale(pt);
            }
            x[WIN * EMB + 7] = f32::from(inp[WIN + 7]);
            let h: Vec<f32> = (0..HID)
                .map(|j| {
                    (b1[j] + (0..SUB_IN).map(|i| w1[j * SUB_IN + i] * x[i]).sum::<f32>())
                        .clamp(0.0, 1.0)
                })
                .collect();
            let mut got = vec![0u8; SUB_OUT];
            forward(&net, &inp, &mut got);
            for (o, &g) in got.iter().enumerate() {
                let v = b2[o] + (0..HID).map(|j| w2[o * HID + j] * h[j]).sum::<f32>();
                let want = (v.clamp(0.0, 1.0) * 127.0).round() as i32;
                assert!((i32::from(g) - want).abs() <= 1, "{o}: {g} {want}");
            }
        }
    }
}
