//! NNUE推論の骨格（ADR-0034〜0036）。
//!
//! HalfKPの差分FT（片視点FT_OUT次元×2視点）を連結し、
//! L1_OUT→L2_OUT→1で評価値を出す。本モジュールはスカラー基準実装
//! （正解器）。accumulator差分とSIMDはこの実装との完全一致を要求する
//! 形で後から積む。
//!
//! 次元は `build.rs` が環境変数 `HIMAWARI_ARCH` から生成する（ADR-0127）。

use himawari_core::{Color, PieceType, Position, Square, bonapiece};

use crate::value::Value;

// FT_OUT・L1_OUT・L2_OUT・L1_PAD・ARCH を定義する。
include!(concat!(env!("OUT_DIR"), "/arch.rs"));

/// 隠れ層の入力次元（FT両視点）。
pub const CONCAT: usize = FT_OUT * 2;
/// 評価値スケール（ADR-0036）。
pub const FV_SCALE: i32 = 16;
/// HalfKP特徴の総数。
pub const FT_IN: usize = 81 * bonapiece::FE_END as usize;

/// 重み一式。量子化はADR-0036（FT系i16、隠れ層i8）。
pub struct NnueNetwork {
    /// FT重み。列優先: `ft_w[feature * FT_OUT + o]`。
    pub ft_w: Vec<i16>,
    pub ft_b: Vec<i16>,
    /// 隠れ層1。行優先: `w2[row * CONCAT + i]`。
    pub w2: Vec<i8>,
    pub b2: Vec<i32>,
    /// 隠れ層2。行優先だが列幅は `L1_PAD` で、`L1_OUT` 以降の列は
    /// 常にゼロにする（入力側もゼロ埋めなので値には影響しない）。
    pub w3: Vec<i8>,
    pub b3: Vec<i32>,
    /// 隠れ層3（`L3_OUT` が0の3層構成では空）。列幅は `L2_PAD`。
    pub w4: Vec<i8>,
    pub b4: Vec<i32>,
    /// 出力層。入力は最後の隠れ層（4層なら`L3_OUT`、3層なら`L2_OUT`）。
    pub w_out: Vec<i8>,
    pub b_out: i32,
}

/// 出力層の入力次元。層を1つ挟むかで変わる。
pub const LAST_HIDDEN: usize = if L3_OUT != 0 { L3_OUT } else { L2_OUT };

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
            /// 行ごとに `used` 列だけ埋め、残りをゼロにした行優先の行列。
            fn i8_rows(&mut self, rows: usize, used: usize, stride: usize) -> Vec<i8> {
                let mut v = vec![0i8; rows * stride];
                for row in 0..rows {
                    for x in &mut v[row * stride..row * stride + used] {
                        *x = (self.next() % 64 - 32) as i8;
                    }
                }
                v
            }
            fn i32v(&mut self, n: usize) -> Vec<i32> {
                (0..n).map(|_| (self.next() % 4096 - 2048) as i32).collect()
            }
        }
        let mut r = Rng(seed.max(1));
        NnueNetwork {
            ft_w: r.i16v(FT_IN * FT_OUT, 32),
            ft_b: r.i16v(FT_OUT, 128),
            w2: r.i8v(L1_OUT * CONCAT),
            b2: r.i32v(L1_OUT),
            w3: r.i8_rows(L2_OUT, L1_OUT, L1_PAD),
            b3: r.i32v(L2_OUT),
            w4: r.i8_rows(L3_OUT, L2_OUT, L2_PAD),
            b4: r.i32v(L3_OUT),
            w_out: r.i8v(LAST_HIDDEN),
            b_out: 0,
        }
    }
}

/// 隠れ層の重みを `行数 × used` から `行数 × stride` へ広げ、余った列を
/// ゼロで埋める。ファイルと学習側はゼロ埋めを持たず、推論だけがSIMDの
/// 都合で広い幅を使う（ADR-0127）。
pub fn pad_rows(rows: &[i8], used: usize, stride: usize) -> Vec<i8> {
    if stride == used || rows.is_empty() {
        return rows.to_vec();
    }
    let count = rows.len() / used;
    let mut v = vec![0i8; count * stride];
    for (dst, src) in v.chunks_mut(stride).zip(rows.chunks_exact(used)) {
        dst[..used].copy_from_slice(src);
    }
    v
}

/// 指し手ラベルの `from` のクラス数。盤上81マス＋打つ駒7種（ADR-0129）。
pub const MOVE_FROM_CLASSES: usize = 81 + 7;
/// 指し手ラベルの `to` のクラス数。
pub const MOVE_TO_CLASSES: usize = 81;
/// ラベルが取れなかったときの値。PyTorchの `ignore_index` に渡す。
pub const MOVE_NONE: i64 = -1;

/// `move16` を手番視点の (from, to) ラベルへ分解する（ADR-0129）。
///
/// HalfKPの特徴は手番視点で作る（`bonapiece::board_bona_piece` が後手番で
/// `sq.inv()` する）ので、指し手のラベルも同じ向きに揃える。揃えないと、
/// 同じ形の局面が先後で違うラベルになり、学習が割れる。
///
/// `from` は盤上81マスに打つ駒7種を続けた88クラス、`to` は81クラス。
/// **成りは落ちる。** どちらのヘッドにも現れない（ADR-0129で承知のうえ）。
pub fn move_labels(m: u16, stm: Color) -> (i64, i64) {
    if m == 0 {
        return (MOVE_NONE, MOVE_NONE);
    }
    let flip = |sq: usize| -> usize { if stm == Color::Black { sq } else { 80 - sq } };
    let to_raw = (m & 0x7F) as usize;
    if to_raw >= 81 {
        return (MOVE_NONE, MOVE_NONE);
    }
    let to = flip(to_raw) as i64;

    let from_raw = ((m >> 7) & 0x7F) as usize;
    // bit15が立っていれば駒打ち。fromフィールドには駒種が入る
    let from = if m & (1 << 15) != 0 {
        match PieceType::HAND_KINDS
            .iter()
            .position(|pt| pt.0 as usize == from_raw)
        {
            Some(i) => (81 + i) as i64,
            None => return (MOVE_NONE, MOVE_NONE),
        }
    } else if from_raw < 81 {
        flip(from_raw) as i64
    } else {
        return (MOVE_NONE, MOVE_NONE);
    };
    (from, to)
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
    // 隠れ層2の入力はL1_PAD幅で、L1_OUT以降はゼロのまま渡す
    let mut h2 = [0u8; L1_PAD];
    for (o, h) in h2[..L1_OUT].iter_mut().enumerate() {
        let mut sum = net.b2[o];
        for (i, &x) in concat.iter().enumerate() {
            sum += i32::from(net.w2[o * CONCAT + i]) * i32::from(x);
        }
        // 学習時のスケール（2^6）で割るのが標準
        *h = clip(sum >> 6);
    }
    let mut h3 = [0u8; L2_PAD];
    for (o, h) in h3[..L2_OUT].iter_mut().enumerate() {
        let mut sum = net.b3[o];
        for (i, &x) in h2.iter().enumerate() {
            sum += i32::from(net.w3[o * L1_PAD + i]) * i32::from(x);
        }
        *h = clip(sum >> 6);
    }
    // 4層構成でだけ層をもう1つ挟む。L3_OUTは定数なので分岐は消える
    let mut h4 = [0u8; L3_OUT];
    for (o, h) in h4.iter_mut().enumerate() {
        let mut sum = net.b4[o];
        for (i, &x) in h3.iter().enumerate() {
            sum += i32::from(net.w4[o * L2_PAD + i]) * i32::from(x);
        }
        *h = clip(sum >> 6);
    }
    let last: &[u8] = if L3_OUT != 0 { &h4 } else { &h3[..L2_OUT] };

    let mut out = net.b_out;
    for (i, &x) in last.iter().enumerate() {
        out += i32::from(net.w_out[i]) * i32::from(x);
    }
    out / FV_SCALE
}

#[cfg(test)]
mod tests {
    use super::*;
    use himawari_core::moves::Move16;
    use himawari_core::{MoveList, SFEN_STARTPOS, generate_legal};

    /// 指し手ラベルは手番視点になる。先手の7g7fと、その180度回転である
    /// 後手の3c3dは同じラベルへ落ちる。揃えないと同じ形が先後で割れる。
    #[test]
    fn move_labels_follow_the_side_to_move() {
        let black = Move16::from_usi("7g7f").expect("7g7f");
        let white = Move16::from_usi("3c3d").expect("3c3d");
        let b = move_labels(black.0, Color::Black);
        let w = move_labels(white.0, Color::White);
        assert_eq!(b, w, "先手{b:?} 後手{w:?}");
        assert!(b.0 < 81 && b.1 < 81, "盤上の移動はどちらも81未満: {b:?}");
    }

    /// 駒打ちのfromは盤上81マスの後ろへ並ぶ。toは手番視点で回る。
    #[test]
    fn dropped_piece_gets_its_own_class() {
        let (from, to) = move_labels(Move16::from_usi("P*5e").expect("P*5e").0, Color::Black);
        assert!(
            (81..MOVE_FROM_CLASSES as i64).contains(&from),
            "打つ駒は81以降: {from}"
        );
        assert!((0..81).contains(&to), "打つ先は盤上: {to}");

        let (_, to_w) = move_labels(Move16::from_usi("P*5e").expect("P*5e").0, Color::White);
        assert_eq!(to_w, 80 - to, "後手視点では180度回る");
    }

    /// 指し手がないレコード（move16=0）はラベルを持たない。
    #[test]
    fn missing_move_is_ignored() {
        assert_eq!(move_labels(0, Color::Black), (MOVE_NONE, MOVE_NONE));
    }

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
