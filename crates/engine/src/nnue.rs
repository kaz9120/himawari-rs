//! NNUE推論の骨格（ADR-0034〜0036）。
//!
//! HalfKPの差分FT（256×2視点）を連結し、32→32→1で評価値を出す。
//! 本モジュールはスカラー基準実装（正解器）。accumulator差分と
//! SIMDはこの実装との完全一致を要求する形で後から積む。

use himawari_core::{Color, PieceType, Position, Square, bonapiece};

use crate::value::Value;

/// FT出力次元（片視点）。
/// FTの出力次元。`ft512` featureで512へ切り替える（ADR-0067）。
#[cfg(not(feature = "ft512"))]
pub const FT_OUT: usize = 256;
#[cfg(feature = "ft512")]
pub const FT_OUT: usize = 512;
/// 隠れ層の入力次元（FT両視点）。
pub const CONCAT: usize = FT_OUT * 2;
pub const HIDDEN: usize = 32;
/// 評価値スケール（ADR-0036）。
pub const FV_SCALE: i32 = 16;
/// HalfKP特徴の総数。
pub const FT_IN: usize = 81 * bonapiece::FE_END as usize;

/// 重み一式。量子化はADR-0036（FT系i16、隠れ層i8）。
pub struct NnueNetwork {
    /// FT重み。列優先: `ft_w[feature * FT_OUT + o]`。
    pub ft_w: Vec<i16>,
    pub ft_b: Vec<i16>,
    /// 隠れ層。行優先: `w2[row * CONCAT + i]`。
    pub w2: Vec<i8>,
    pub b2: Vec<i32>,
    pub w3: Vec<i8>,
    pub b3: Vec<i32>,
    pub w4: Vec<i8>,
    pub b4: i32,
}

impl NnueNetwork {
    /// テスト・開発用の乱数重み（xorshiftで再現可能）。
    pub fn random(seed: u64) -> NnueNetwork {
        struct Rng(u64);
        impl Rng {
            fn next(&mut self) -> i64 {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                self.0 as i64
            }
            fn i16v(&mut self, n: usize, range: i64) -> Vec<i16> {
                (0..n)
                    .map(|_| ((self.next() % range) - range / 2) as i16)
                    .collect()
            }
            fn i8v(&mut self, n: usize) -> Vec<i8> {
                (0..n).map(|_| (self.next() % 64 - 32) as i8).collect()
            }
            fn i32v(&mut self, n: usize) -> Vec<i32> {
                (0..n).map(|_| (self.next() % 4096 - 2048) as i32).collect()
            }
        }
        let mut r = Rng(seed.max(1));
        NnueNetwork {
            ft_w: r.i16v(FT_IN * FT_OUT, 32),
            ft_b: r.i16v(FT_OUT, 128),
            w2: r.i8v(HIDDEN * CONCAT),
            b2: r.i32v(HIDDEN),
            w3: r.i8v(HIDDEN * HIDDEN),
            b3: r.i32v(HIDDEN),
            w4: r.i8v(HIDDEN),
            b4: 0,
        }
    }
}

/// 視点cのHalfKP活性特徴（玉以外の盤上駒＋両者の持ち駒）を列挙する。
pub fn halfkp_active(pos: &Position, c: Color, out: &mut Vec<u32>) {
    out.clear();
    let king = pos.king(c);
    for sq_i in 0..81u8 {
        let sq = Square::from_index(sq_i);
        let pc = pos.piece_on(sq);
        if pc.is_empty() || pc.piece_type() == PieceType::KING {
            continue;
        }
        let bp = bonapiece::board_bona_piece(c, pc, sq);
        out.push(bonapiece::halfkp_index(c, king, bp));
    }
    for owner in [Color::Black, Color::White] {
        let hand = pos.hand(owner);
        for pt in PieceType::HAND_KINDS {
            for i in 1..=hand.count(pt) {
                let bp = bonapiece::hand_bona_piece(c, owner, pt, i);
                out.push(bonapiece::halfkp_index(c, king, bp));
            }
        }
    }
}

#[inline]
fn clip(v: i32) -> u8 {
    v.clamp(0, 127) as u8
}

/// スカラー全計算の評価（手番視点、歩=90スケール）。
/// 差分計算・SIMDの正解基準（ADR-0035, 0036）。
pub fn evaluate_scalar(net: &NnueNetwork, pos: &Position) -> Value {
    let stm = pos.side_to_move();
    let mut features = Vec::with_capacity(64);
    let mut concat = [0u8; CONCAT];

    // FT: 手番視点 → [0..FT_OUT)、非手番視点 → [FT_OUT..2*FT_OUT)
    for (half, c) in [(0usize, stm), (1, stm.flip())] {
        halfkp_active(pos, c, &mut features);
        let mut acc = [0i32; FT_OUT];
        for (o, a) in acc.iter_mut().enumerate() {
            *a = i32::from(net.ft_b[o]);
        }
        for &f in &features {
            let base = f as usize * FT_OUT;
            for (o, a) in acc.iter_mut().enumerate() {
                *a += i32::from(net.ft_w[base + o]);
            }
        }
        for (o, &a) in acc.iter().enumerate() {
            concat[half * FT_OUT + o] = clip(a);
        }
    }

    forward_hidden(net, &concat)
}

/// 連結ベクトルから評価値まで（隠れ層はi8×u8の積和、ADR-0036）。
pub(crate) fn forward_hidden(net: &NnueNetwork, concat: &[u8; CONCAT]) -> Value {
    let mut h2 = [0u8; HIDDEN];
    for (o, h) in h2.iter_mut().enumerate() {
        let mut sum = net.b2[o];
        for (i, &x) in concat.iter().enumerate() {
            sum += i32::from(net.w2[o * CONCAT + i]) * i32::from(x);
        }
        // 学習時のスケール（2^6）で割るのが標準
        *h = clip(sum >> 6);
    }
    let mut h3 = [0u8; HIDDEN];
    for (o, h) in h3.iter_mut().enumerate() {
        let mut sum = net.b3[o];
        for (i, &x) in h2.iter().enumerate() {
            sum += i32::from(net.w3[o * HIDDEN + i]) * i32::from(x);
        }
        *h = clip(sum >> 6);
    }
    let mut out = net.b4;
    for (i, &x) in h3.iter().enumerate() {
        out += i32::from(net.w4[i]) * i32::from(x);
    }
    out / FV_SCALE
}

#[cfg(test)]
mod tests {
    use super::*;
    use himawari_core::{MoveList, SFEN_STARTPOS, generate_legal};

    /// 局面を180度回転・先後入替した鏡像SFENを作る。
    fn mirror_sfen(pos: &Position) -> String {
        let sfen = pos.to_sfen();
        let parts: Vec<&str> = sfen.split(' ').collect();
        // 盤面: 行を逆順・各行の文字列も逆順、大小文字を入替
        let rows: Vec<String> = parts[0]
            .split('/')
            .rev()
            .map(|row| {
                let mut cells: Vec<String> = Vec::new();
                let mut chars = row.chars().peekable();
                while let Some(ch) = chars.next() {
                    if ch == '+' {
                        let p = chars.next().unwrap();
                        cells.push(format!("+{}", swap_case(p)));
                    } else if ch.is_ascii_digit() {
                        cells.push(ch.to_string());
                    } else {
                        cells.push(swap_case(ch).to_string());
                    }
                }
                cells.reverse();
                cells.concat()
            })
            .collect();
        let side = if parts[1] == "b" { "w" } else { "b" };
        let hands = if parts[2] == "-" {
            "-".to_string()
        } else {
            parts[2].chars().map(swap_case).collect()
        };
        format!("{} {side} {hands} 1", rows.join("/"))
    }

    fn swap_case(c: char) -> char {
        if c.is_ascii_uppercase() {
            c.to_ascii_lowercase()
        } else if c.is_ascii_lowercase() {
            c.to_ascii_uppercase()
        } else {
            c
        }
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
    }

    /// 鏡像局面（180度回転・先後入替・手番反転）は同じ評価値になる。
    /// 特徴抽出の視点対称性が正しいことの自己検証。
    #[test]
    fn mirror_invariance() {
        let net = NnueNetwork::random(42);
        for seed in 1..=8u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
            for ply in 0..40 {
                let mut list = MoveList::default();
                generate_legal(&pos, true, &mut list);
                if list.is_empty() {
                    break;
                }
                if ply % 5 == 0 {
                    let mirrored = Position::from_sfen(&mirror_sfen(&pos)).unwrap();
                    assert_eq!(
                        evaluate_scalar(&net, &pos),
                        evaluate_scalar(&net, &mirrored),
                        "鏡像で評価が一致しない: {}",
                        pos.to_sfen()
                    );
                }
                let m = list.as_slice()[(rng.next() % list.len() as u64) as usize];
                pos.do_move(m);
            }
        }
    }

    /// HalfKP活性特徴数 = 盤上の玉以外の駒 + 持ち駒総数。
    #[test]
    fn halfkp_active_count() {
        let pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
        let mut v = Vec::new();
        halfkp_active(&pos, Color::Black, &mut v);
        assert_eq!(v.len(), 38, "平手は玉以外38枚・持ち駒なし");
        assert!(v.iter().all(|&f| (f as usize) < FT_IN));
    }
}
