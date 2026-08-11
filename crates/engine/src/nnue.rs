//! NNUE推論の骨格（ADR-0034〜0036）。
//!
//! HalfKPの差分FT（片視点FT_OUT次元×2視点）を連結し、
//! L1_OUT→L2_OUT→1で評価値を出す。本モジュールはスカラー基準実装
//! （正解器）。accumulator差分とSIMDはこの実装との完全一致を要求する
//! 形で後から積む。
//!
//! 次元は `build.rs` が環境変数 `HIMAWARI_ARCH` から生成する（ADR-0127）。

use himawari_core::attacks::{attacks, king_attacks};
use himawari_core::{Bitboard, Color, PieceType, Position, Square, bonapiece};

use crate::value::Value;

// FT_OUT・L1_OUT・L2_OUT・L1_PAD・ARCH を定義する。
include!(concat!(env!("OUT_DIR"), "/arch.rs"));

/// 隠れ層の入力次元（FT両視点）。
pub const CONCAT: usize = FT_OUT * 2;
/// 評価値スケール（ADR-0036）。
pub const FV_SCALE: i32 = 16;
/// HalfKP特徴の総数。
pub const FT_IN: usize = 81 * bonapiece::FE_END as usize;

/// FT重みの格納型（ADR-0036、ADR-0138）。既定はi16で、`HIMAWARI_FT_I8=1`
/// でビルドするとi8になる。accumulatorとFTバイアスはどちらの場合もi16の
/// ままなので、活性（clipped ReLU）以降の精度は変わらない。
#[cfg(not(ft_i8))]
pub type FtWeight = i16;
#[cfg(ft_i8)]
pub type FtWeight = i8;

/// i16で読んだFT重みを格納型へ移す（ADR-0138）。
///
/// i8のビルドでは範囲を確かめる。**黙って切り詰めない。** 飽和は
/// 0.055%でも−59.3 Eloになる（ADR-0138のリーグ戦）ので、気づかず
/// 壊れたネットで対局する事故のほうが高くつく。
pub fn ft_w_from_i16(v: Vec<i16>) -> Result<Vec<FtWeight>, String> {
    #[cfg(not(ft_i8))]
    {
        Ok(v)
    }
    #[cfg(ft_i8)]
    {
        if let Some(&bad) = v.iter().find(|&&x| !(-128..=127).contains(&i32::from(x))) {
            return Err(format!(
                "FT重み{bad}がi8に収まらない。--ft-clipを付けて学習したネットが要る（ADR-0138）"
            ));
        }
        Ok(v.into_iter().map(|x| x as i8).collect())
    }
}

/// 重み一式。量子化はADR-0036（FT系i16、隠れ層i8）。FT重みだけは
/// ビルドでi8にもできる（ADR-0138）。
pub struct NnueNetwork {
    /// FT重み。列優先: `ft_w[feature * FT_OUT + o]`。
    pub ft_w: Vec<FtWeight>,
    pub ft_b: Vec<i16>,
    /// 隠れ層1。行優先: `w2[row * CONCAT + i]`。
    pub w2: Vec<i8>,
    /// 同じ重みを4列チャンク単位で並べ替えた表（ADR-0151群L）。
    /// `w2` から機械的に作る派生表で、列駆動の推論だけが読む。
    /// **`w2` を差し替えたら `finish` を呼び直す。**
    pub w2_sparse: Vec<i8>,
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
pub const LAST_HIDDEN: usize = if L3_OUT != 0 {
    L3_OUT
} else if L2_OUT != 0 {
    L2_OUT
} else {
    L1_OUT
};

/// 隠れ層1の重みを4列チャンク単位で並べ替える（ADR-0151群L）。
///
/// `out[k * L1_OUT * 4 + o * 4 + j] = w2[o * CONCAT + 4k + j]`。入力列を
/// 4要素ずつのチャンクに区切り、チャンクkについて全出力行の重みを
/// 連続させる。列駆動の推論は、活性が非ゼロのチャンクだけを選んで
/// この16バイト単位の並びを読む。長さは `w2` と同じで、既定構成では16KBになる。
pub fn interleave_w2(w2: &[i8]) -> Vec<i8> {
    debug_assert_eq!(w2.len(), L1_OUT * CONCAT);
    let mut t = vec![0i8; L1_OUT * CONCAT];
    for k in 0..CONCAT / 4 {
        for o in 0..L1_OUT {
            let dst = k * L1_OUT * 4 + o * 4;
            t[dst..dst + 4].copy_from_slice(&w2[o * CONCAT + 4 * k..o * CONCAT + 4 * k + 4]);
        }
    }
    t
}

impl NnueNetwork {
    /// 派生表を作って完成させる。**`w2` を決めた直後に呼ぶ。**
    ///
    /// 呼び忘れても評価値は変わらない（列駆動の経路が使う表が空になり、
    /// 密の経路へ落ちるだけ）が、速度は戻る。
    #[must_use]
    pub fn finish(mut self) -> NnueNetwork {
        self.w2_sparse = interleave_w2(&self.w2);
        self
    }

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
            ft_w: r
                .i16v(FT_IN * FT_OUT, 32)
                .into_iter()
                .map(|v| v as FtWeight)
                .collect(),
            ft_b: r.i16v(FT_OUT, 128),
            w2: r.i8v(L1_OUT * CONCAT),
            w2_sparse: Vec::new(),
            b2: r.i32v(L1_OUT),
            w3: r.i8_rows(L2_OUT, L1_OUT, L1_PAD),
            b3: r.i32v(L2_OUT),
            w4: r.i8_rows(L3_OUT, L2_OUT, L2_PAD),
            b4: r.i32v(L3_OUT),
            w_out: r.i8v(LAST_HIDDEN),
            b_out: 0,
        }
        .finish()
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

/// 利きラベル1本の長さ。手番側81升＋相手側81升（ADR-0133）。
/// 短い利きと長い利きで1本ずつ持つ。
pub const EFFECT_LEN: usize = 81 * 2;

/// 遮りで利きが変わる駒（飛び駒）か。馬・竜は遠隔の部分を持つので含む。
fn is_slider(pt: PieceType) -> bool {
    matches!(
        pt,
        PieceType::LANCE
            | PieceType::BISHOP
            | PieceType::ROOK
            | PieceType::HORSE
            | PieceType::DRAGON
    )
}

/// 盤上の実利きを長短に分けて数える（ADR-0133）。
///
/// 分ける基準は距離ではなく遮り依存性である。飛び駒でない駒（歩・桂・銀・
/// 金相当・玉）は遮りが起こらないので、桂が跳ぶ2升先も含めてすべて短い。
/// 飛び駒（香・角・飛・馬・竜）は隣接升だけがその先の駒に依らず必ず利くので、
/// 隣接を短い、その先を長いとする。馬・竜の `attacks` は
/// `bishop_attacks | king_attacks` の形なので、この規則で自動的に分かれる。
/// 短い＋長いは、常にその駒の利き全体に一致する。
///
/// 添字は `陣営 * 81 + 手番視点の升`。陣営0が手番側、1が相手側で、升は
/// `move_labels` と同じく後手番のとき `80 - idx` へ回す。揃えないと同じ形の
/// 局面が先後で違うラベルになり、学習が割れる。
///
/// 値は利き数であって有無ではない。学習側で二値化も回帰も選べるようにして
/// あり、的を変えるのに抽出をやり直さずに済む。
pub fn effect_labels(pos: &Position) -> ([u8; EFFECT_LEN], [u8; EFFECT_LEN]) {
    let stm = pos.side_to_move();
    let occ = pos.occupied();
    let mut short = [0u8; EFFECT_LEN];
    let mut long = [0u8; EFFECT_LEN];
    // 手番視点へそろえる。盤自体は反転しない
    let view = |sq: Square| -> usize {
        if stm == Color::Black {
            sq.index()
        } else {
            sq.inv().index()
        }
    };
    // 盤上の駒を1周する。81升それぞれで attackers_to を呼ぶと81回になるが、
    // 駒を回れば盤上の枚数（平手で40）で済む
    for sq_i in 0..81u8 {
        let sq = Square::from_index(sq_i);
        let pc = pos.piece_on(sq);
        if pc.is_empty() {
            continue;
        }
        let base = if pc.color() == stm { 0 } else { 81 };
        let att = attacks(pc, sq, occ);
        let (near, far) = if is_slider(pc.piece_type()) {
            let adjacent = king_attacks(sq);
            (att & adjacent, att & !adjacent)
        } else {
            (att, Bitboard::EMPTY)
        };
        for to in near {
            short[base + view(to)] += 1;
        }
        for to in far {
            long[base + view(to)] += 1;
        }
    }
    (short, long)
}

/// 視点cのHalfKP活性特徴（玉以外の盤上駒＋両者の持ち駒）のBonaPieceを
/// 順に渡す。玉位置のオフセットを掛ける前の形で、これを起点に
/// 特徴インデックスにもキャッシュの鍵にもできる（ADR-0156）。
/// 盤上は駒のある升だけを走る。81升を舐めると空升の判定が3分の2を
/// 占め、全計算のほうで列挙が支配的になる（ADR-0156のプロファイル）。
#[inline]
pub fn for_each_bona_piece(pos: &Position, c: Color, mut f: impl FnMut(u16)) {
    let kings = Bitboard::from_square(pos.king(Color::Black))
        | Bitboard::from_square(pos.king(Color::White));
    for sq in pos.occupied() ^ kings {
        f(bonapiece::board_bona_piece(c, pos.piece_on(sq), sq));
    }
    for owner in [Color::Black, Color::White] {
        let hand = pos.hand(owner);
        for pt in PieceType::HAND_KINDS {
            for i in 1..=hand.count(pt) {
                f(bonapiece::hand_bona_piece(c, owner, pt, i));
            }
        }
    }
}

/// 視点cのHalfKP活性特徴（玉以外の盤上駒＋両者の持ち駒）を列挙する。
pub fn halfkp_active(pos: &Position, c: Color, out: &mut Vec<u32>) {
    out.clear();
    let king = pos.king(c);
    for_each_bona_piece(pos, c, |bp| {
        out.push(bonapiece::halfkp_index(c, king, bp));
    });
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
    // 隠れ層は書いたぶんだけ挟む。次元は定数なので使わない分岐は消える
    let mut h3 = [0u8; L2_PAD];
    for (o, h) in h3[..L2_OUT].iter_mut().enumerate() {
        let mut sum = net.b3[o];
        for (i, &x) in h2.iter().enumerate() {
            sum += i32::from(net.w3[o * L1_PAD + i]) * i32::from(x);
        }
        *h = clip(sum >> 6);
    }
    let mut h4 = [0u8; L3_OUT];
    for (o, h) in h4.iter_mut().enumerate() {
        let mut sum = net.b4[o];
        for (i, &x) in h3.iter().enumerate() {
            sum += i32::from(net.w4[o * L2_PAD + i]) * i32::from(x);
        }
        *h = clip(sum >> 6);
    }
    let last: &[u8] = if L3_OUT != 0 {
        &h4
    } else if L2_OUT != 0 {
        &h3[..L2_OUT]
    } else {
        &h2[..L1_OUT]
    };

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

    /// 手番視点の升へ回す。テスト側でも同じ変換を持ち、実装と突き合わせる。
    fn view_index(sq: Square, stm: Color) -> usize {
        if stm == Color::Black {
            sq.index()
        } else {
            sq.inv().index()
        }
    }

    /// 検証用に置いた局面。馬・竜・香を含む。ランダム対局40手では
    /// 成駒がほとんど出ないので、別に用意する
    const EFFECT_SFENS: [&str; 3] = [
        "8k/9/9/9/4+R4/9/4+B4/9/K8 b - 1",
        "8k/9/9/9/4L4/9/4l4/9/K8 b - 1",
        "8k/2n6/9/3s5/4B4/9/2g2N3/9/K8 w - 1",
    ];

    /// 短い利きと長い利きの和は、その升の利き数そのものになる。基準は
    /// 探索が使う `attackers_to` で、実装を共有しない別経路である。
    #[test]
    fn effect_short_plus_long_covers_every_attacker() {
        let mut positions: Vec<Position> = EFFECT_SFENS
            .iter()
            .map(|s| Position::from_sfen(s).expect("検証用sfen"))
            .collect();
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
                    positions.push(Position::from_sfen(&pos.to_sfen()).unwrap());
                }
                let m = list.as_slice()[(rng.next() % list.len() as u64) as usize];
                pos.do_move(m);
            }
        }

        for pos in &positions {
            let (short, long) = effect_labels(pos);
            let stm = pos.side_to_move();
            let occ = pos.occupied();
            for i in 0..81u8 {
                let sq = Square::from_index(i);
                let v = view_index(sq, stm);
                for (base, c) in [(0usize, stm), (81, stm.flip())] {
                    let want = pos.attackers_to(c, sq, occ).count();
                    let got = u32::from(short[base + v]) + u32::from(long[base + v]);
                    assert_eq!(
                        got,
                        want,
                        "利き数が合わない: sq={} c={c:?} {}",
                        sq.to_usi(),
                        pos.to_sfen()
                    );
                }
            }
        }
    }

    /// 遮りは長い利きにだけ効く。香の隣接1升は手前に駒が入っても変わらず、
    /// その先だけが消える。長短を分ける基準が距離ではなく遮り依存性である
    /// ことの確認になる（ADR-0133）。
    #[test]
    fn blocking_changes_only_the_long_effect() {
        // 先手香を5五に置く。空なら5四〜5一へ利く
        let open = Position::from_sfen("8k/9/9/9/4L4/9/9/9/K8 b - 1").unwrap();
        // 5三に先手歩を足す。香の利きは5四・5三で止まる
        let blocked = Position::from_sfen("8k/9/4P4/9/4L4/9/9/9/K8 b - 1").unwrap();
        let (s_open, l_open) = effect_labels(&open);
        let (s_blocked, l_blocked) = effect_labels(&blocked);
        let at = |sq: &str| Square::from_usi(sq).expect("升").index();

        // 隣接升（5四）は遮りに依らず必ず利く
        assert_eq!(s_open[at("5d")], 1);
        assert_eq!(s_blocked[at("5d")], 1);
        // その先は消える。5三は歩に当たって残る（利きは駒の上まで届く）
        assert_eq!(
            (l_open[at("5c")], l_open[at("5b")], l_open[at("5a")]),
            (1, 1, 1)
        );
        assert_eq!(
            (
                l_blocked[at("5c")],
                l_blocked[at("5b")],
                l_blocked[at("5a")]
            ),
            (1, 0, 0)
        );
        // 5二の利き数は歩が肩代わりして1のまま。**利き有無だけを的にすると
        // 遮りが見えない**（ADR-0133がT2を退けた理由）ことがここに出る
        assert_eq!(s_blocked[at("5b")], 1);
        assert_eq!(s_open[at("5b")], 0);
    }

    /// 飛び駒でない駒は長い利きを持たない。桂は2升先へ跳ぶが、遮りが
    /// 起こらないので短い扱いにする（ADR-0133）。
    #[test]
    fn non_sliders_have_no_long_effect() {
        // 先手の桂・銀・金・歩と玉だけを置く
        let pos = Position::from_sfen("8k/9/9/9/4N4/9/2G1S1P2/9/K8 b - 1").unwrap();
        let (short, long) = effect_labels(&pos);
        assert!(
            long.iter().all(|&x| x == 0),
            "飛び駒がなければ長い利きは出ない"
        );
        let at = |sq: &str| Square::from_usi(sq).expect("升").index();
        // 5五の桂が跳ぶ先は2升離れているが、短いほうへ入る
        assert_eq!(short[at("4c")], 1);
        assert_eq!(short[at("6c")], 1);
    }

    /// 利きラベルは手番視点になる。先手の局面と、それを180度回して
    /// 先後を入れ替えた局面は同じラベルへ落ちる。
    #[test]
    fn effect_labels_follow_the_side_to_move() {
        let mut positions: Vec<Position> = EFFECT_SFENS
            .iter()
            .map(|s| Position::from_sfen(s).expect("検証用sfen"))
            .collect();
        let mut rng = Rng(0x1234_5678_9ABC_DEF0);
        let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
        for ply in 0..40 {
            let mut list = MoveList::default();
            generate_legal(&pos, true, &mut list);
            if list.is_empty() {
                break;
            }
            if ply % 3 == 0 {
                positions.push(Position::from_sfen(&pos.to_sfen()).unwrap());
            }
            let m = list.as_slice()[(rng.next() % list.len() as u64) as usize];
            pos.do_move(m);
        }

        for pos in &positions {
            let mirrored = Position::from_sfen(&mirror_sfen(pos)).unwrap();
            assert_eq!(
                effect_labels(pos),
                effect_labels(&mirrored),
                "鏡像でラベルが一致しない: {}",
                pos.to_sfen()
            );
        }
    }

    /// インターリーブ表は `w2` の並べ替えである（ADR-0151群L）。
    /// 全要素を1対1で突き合わせる。列駆動の推論はこの並びに依存する。
    #[test]
    fn interleaved_w2_is_a_permutation_of_w2() {
        let net = NnueNetwork::random(17);
        assert_eq!(net.w2_sparse.len(), net.w2.len());
        for k in 0..CONCAT / 4 {
            for o in 0..L1_OUT {
                for j in 0..4 {
                    assert_eq!(
                        net.w2_sparse[k * L1_OUT * 4 + o * 4 + j],
                        net.w2[o * CONCAT + 4 * k + j],
                        "k={k} o={o} j={j}"
                    );
                }
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
