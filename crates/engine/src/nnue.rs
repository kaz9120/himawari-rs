//! NNUE推論の骨格（ADR-0034〜0036）。
//!
//! HalfKPの差分FT（片視点FT_OUT次元×2視点）を連結し、
//! L1_OUT→L2_OUT→1で評価値を出す。本モジュールはスカラー基準実装
//! （正解器）。accumulator差分とSIMDはこの実装との完全一致を要求する
//! 形で後から積む。
//!
//! 次元は `build.rs` が環境変数 `HIMAWARI_ARCH` から生成する（ADR-0127）。

use himawari_core::attacks::{
    attacks, bishop_attacks, gold_attacks, king_attacks, knight_attacks, lance_attacks,
    rook_attacks, silver_attacks,
};
use himawari_core::{Bitboard, Color, PieceType, Position, Square, bonapiece};

use crate::value::Value;

// FT_OUT・L1_OUT・L2_OUT・L1_PAD・ARCH を定義する。
include!(concat!(env!("OUT_DIR"), "/arch.rs"));

/// 隠れ層の入力次元（FT両視点＋利きの両視点）。
///
/// 片視点は `FT_OUT` 次元のaccumulatorを2つに割って要素ごとに掛けるので、
/// 出す活性は `FT_OUT / 2` 次元になる（ADR-0171）。両視点でその2倍。
/// 利きのaccumulator（ADR-0237）を持つ構成では、その後ろへ `EFF_OUT` 次元を
/// 2視点ぶん連結する。`EFF_OUT` が0の既定構成では `FT_OUT` のままである。
/// 玉の近傍の部分モデル（ADR-0246）を持つ構成では、さらに末尾へ両玉の出力が続く。
pub const CONCAT: usize = FT_OUT + 2 * EFF_ACT + crate::submodel::SUB_SLOT;
/// 側のaccumulatorが片視点で出す活性の幅。対の積を掛ける構成（ADR-0245）では半分になる。
pub const EFF_ACT: usize = if EFF_PAIR { EFF_OUT / 2 } else { EFF_OUT };
/// 連結ベクトルのうち、HalfKPの片視点ぶんの幅。
pub const HALF: usize = FT_OUT / 2;
/// 連結ベクトルのうち、利きの活性が始まる位置（ADR-0237）。
pub const EFF_BASE: usize = FT_OUT;
/// 利きの特徴で、升に乗るものの種類。空き・自分の14種・相手の14種（ADR-0237）。
pub const EFF_PIECES: usize = 29;
/// 利き数の段階。0・1・2・3以上の4つ（技巧の3で打ち切り、ADR-0237）。
pub const EFF_LEVELS: usize = 4;
// 側の特徴の総数 `EFF_IN` はbuild.rsが構成名から作る（ADR-0245）。表し方を書かない
// 構成ではF2だけで、升×乗るもの×自分の利き数×相手の利き数になる
const _: () = assert!(EFF_GENERIC || EFF_IN == 81 * EFF_PIECES * EFF_LEVELS * EFF_LEVELS);
/// 評価値スケール（ADR-0036）。
pub const FV_SCALE: i32 = 16;
/// 入力特徴の総数。
pub const FT_IN: usize = 81 * bonapiece::FE_END as usize;

/// FT重みの格納型（ADR-0138）。accumulatorとFTバイアスはi16のままなので、
/// 活性（clipped ReLU）以降の精度は変わらない。
pub type FtWeight = i8;

/// i16で読んだFT重みを格納型へ移す（ADR-0138）。
///
/// 範囲を確かめる。**黙って切り詰めない。** 飽和は
/// 0.055%でも−59.3 Eloになる（ADR-0138のリーグ戦）ので、気づかず
/// 壊れたネットで対局する事故のほうが高くつく。
pub fn ft_w_from_i16(v: Vec<i16>) -> Result<Vec<FtWeight>, String> {
    if let Some(&bad) = v.iter().find(|&&x| !(-128..=127).contains(&i32::from(x))) {
        return Err(format!(
            "FT weight {bad} does not fit in i8; train the net with --ft-clip (ADR-0138)"
        ));
    }
    Ok(v.into_iter().map(|x| x as i8).collect())
}

/// 重み一式。量子化はADR-0036（隠れ層i8）とADR-0138（FT重みi8）。
/// FTバイアスとaccumulatorはi16で持つ。
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
    /// 利きの特徴の重み（ADR-0237）。列優先: `eff_w[feature * EFF_OUT + o]`。
    /// `EFF_OUT` が0の構成では空。
    pub eff_w: Vec<i16>,
    pub eff_b: Vec<i16>,
    /// 玉の近傍の部分モデル（ADR-0246）。持たない構成では空。
    pub sub: crate::submodel::SubNet,
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
        let mut net = NnueNetwork {
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
            eff_w: Vec::new(),
            eff_b: Vec::new(),
            sub: crate::submodel::SubNet::default(),
        };
        // 利きの重みは最後に引く。既定構成（EFF_OUT=0）の乱数列を変えない
        net.eff_w = r.i16v(EFF_IN * EFF_OUT, 16);
        net.eff_b = r.i16v(EFF_OUT, 128);
        net.sub = crate::submodel::SubNet::random(seed ^ 0x5b);
        net.finish()
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

/// 升ごとの利き数を色別に数える（ADR-0237）。添字は `[色][升]` で、升は盤の
/// 向きのまま。玉の利きを含め、ピンは無視する。その升の駒自身は数えない
/// （駒は自分の升へ利かない）。値は打ち切らない。
///
/// 盤上の駒を1周して利き先へ足す。`effect_labels` と同じ数え方で、
/// `attackers_to` との一致はテストで確かめる。
pub fn effect_counts(pos: &Position) -> [[u8; 81]; 2] {
    let occ = pos.occupied();
    let mut n = [[0u8; 81]; 2];
    for sq in occ {
        let pc = pos.piece_on(sq);
        let side = &mut n[pc.color().index()];
        for to in attacks(pc, sq, occ) {
            side[to.index()] += 1;
        }
    }
    n
}

/// 利きの特徴のindex（ADR-0237のF2）。`sq` は視点から見た升、`pc` は
/// 0（空き）・1〜14（自分の駒）・15〜28（相手の駒）、`own`・`opp` は
/// 自分と相手の利き数（打ち切り前）。
#[inline]
pub fn effect_index(sq: usize, pc: usize, own: u8, opp: u8) -> u32 {
    let cap = |n: u8| usize::from(n).min(EFF_LEVELS - 1);
    (((sq * EFF_PIECES + pc) * EFF_LEVELS + cap(own)) * EFF_LEVELS + cap(opp)) as u32
}

/// 視点cの利きの特徴を81個書く（ADR-0237のF2）。升ごとに1つ立つ。
///
/// 視点ごとに盤を反転し、自他は視点から見る。HalfKPと同じ扱いなので、
/// 180度回して先後を入れ替えた局面は同じ特徴になる。`counts` は
/// `effect_counts` の結果を渡す。両視点で使い回すために外へ出してある。
pub fn effect_active(pos: &Position, c: Color, counts: &[[u8; 81]; 2], out: &mut [u32; 81]) {
    let (own, opp) = (&counts[c.index()], &counts[c.flip().index()]);
    for i in 0..81u8 {
        let sq = Square::from_index(i);
        let pc = pos.piece_on(sq);
        let code = if pc.is_empty() {
            0
        } else {
            // 駒種は2〜15の14種。自分を1〜14、相手を15〜28へ写す
            let kind = usize::from(pc.piece_type().0) - 1;
            if pc.color() == c { kind } else { kind + 14 }
        };
        let view = if c == Color::Black { sq } else { sq.inv() };
        out[view.index()] = effect_index(view.index(), code, own[sq.index()], opp[sq.index()]);
    }
}

/// 利きのaccumulatorを特徴から全計算する（ADR-0237のD3）。i32で足すので、
/// 81行を足しても桁あふれしない。差分の経路（D2）はこの値との一致を求める。
pub fn effect_accumulate(net: &NnueNetwork, features: &[u32; 81]) -> [i32; EFF_OUT] {
    let mut acc = [0i32; EFF_OUT];
    for (a, &b) in acc.iter_mut().zip(&net.eff_b) {
        *a = i32::from(b);
    }
    for &f in features {
        effect_row_add(&mut acc, effect_row(net, f));
    }
    acc
}

/// 利きのaccumulatorが取りうる値の上限（絶対値）を、重みから見積もる。
///
/// 出力ごとに、バイアスと升ごとの重みの最大の絶対値を足す。1升に立つ特徴は
/// 1つなので、どの局面でもaccの絶対値はこれを超えない。i16に収まれば、
/// 差分の経路（D2）をi16の巡回加算で持っても、i32で足す全計算と一致する。
pub fn effect_acc_bound(net: &NnueNetwork) -> i64 {
    let per = EFF_PIECES * EFF_LEVELS * EFF_LEVELS;
    // 利きを持たない構成では重みが空で、下のループは回らない。chunksに0を
    // 渡せないので、そのときだけ幅を1と置く
    const WIDTH: usize = if EFF_OUT == 0 { 1 } else { EFF_OUT };
    let width = WIDTH;
    let mut sum: Vec<i64> = net.eff_b.iter().map(|&b| i64::from(b).abs()).collect();
    for rows in net.eff_w.chunks(per * width) {
        let mut most = vec![0i64; EFF_OUT];
        for row in rows.chunks(width) {
            for (m, &w) in most.iter_mut().zip(row) {
                *m = (*m).max(i64::from(w).abs());
            }
        }
        for (s, m) in sum.iter_mut().zip(&most) {
            *s += m;
        }
    }
    sum.into_iter().max().unwrap_or(0)
}

/// 利きの特徴 `f` の重みの行。
#[inline]
pub fn effect_row(net: &NnueNetwork, f: u32) -> &[i16] {
    let at = f as usize * EFF_OUT;
    &net.eff_w[at..at + EFF_OUT]
}

#[inline]
pub fn effect_row_add(acc: &mut [i32; EFF_OUT], row: &[i16]) {
    for (a, &w) in acc.iter_mut().zip(row) {
        *a += i32::from(w);
    }
}

/// 利きのaccumulatorを0〜127へクリップして書く。
#[inline]
pub fn effect_clip(acc: &[i32; EFF_OUT], out: &mut [u8]) {
    debug_assert_eq!(out.len(), EFF_OUT);
    for (o, &a) in out.iter_mut().zip(acc) {
        *o = clip(a);
    }
}

/// 差分で持つ側のaccの要素の型。F2だけの構成はi16（ADR-0237のD2）、
/// F2と最小の攻め駒の組（`SIDE_MINATT_INC`）はi32で持つ（ADR-0249）。
#[cfg(not(eff_acc32))]
pub type EffAcc = i16;
/// 差分で持つ側のaccの要素の型。F2だけの構成はi16（ADR-0237のD2）、
/// F2と最小の攻め駒の組（`SIDE_MINATT_INC`）はi32で持つ（ADR-0249）。
#[cfg(eff_acc32)]
pub type EffAcc = i32;

/// 差分で持つ側のaccを0〜127へクリップして書く（D2）。
#[cfg(not(eff_acc32))]
#[inline]
pub fn effect_clip_acc(acc: &[EffAcc; EFF_OUT], out: &mut [u8]) {
    debug_assert_eq!(out.len(), EFF_OUT);
    for (o, &a) in out.iter_mut().zip(acc) {
        *o = a.clamp(0, 127) as u8;
    }
}

/// 差分で持つ側のaccを0〜127へクリップして書く（ADR-0249）。
#[cfg(eff_acc32)]
#[inline]
pub fn effect_clip_acc(acc: &[EffAcc; EFF_OUT], out: &mut [u8]) {
    effect_clip(acc, out);
}

/// 差分で持つ側のaccへバイアスを写す。
#[cfg(not(eff_acc32))]
#[inline]
pub fn effect_acc_bias(net: &NnueNetwork, acc: &mut [EffAcc; EFF_OUT]) {
    acc.copy_from_slice(&net.eff_b[..EFF_OUT]);
}

/// 差分で持つ側のaccへバイアスを写す（i32へ広げる。ADR-0249）。
#[cfg(eff_acc32)]
#[inline]
pub fn effect_acc_bias(net: &NnueNetwork, acc: &mut [EffAcc; EFF_OUT]) {
    for (a, &b) in acc.iter_mut().zip(&net.eff_b[..EFF_OUT]) {
        *a = i32::from(b);
    }
}

/// 利きのaccumulatorを特徴から作り直し、0〜127へクリップして書く（ADR-0237のD3）。
///
/// 評価のたびに81行を足す。差分は持たない。対の積は掛けない。
pub fn effect_transform(net: &NnueNetwork, features: &[u32; 81], out: &mut [u8]) {
    effect_clip(&effect_accumulate(net, features), out);
}

/// 色ごとの利き数を、1枚以上・2枚以上・3枚以上の3枚の集合で持つ（ADR-0237のD2）。
///
/// 特徴は利き数を3で打ち切るので、升ごとの数を持たなくても段階は決まる。
/// 駒ごとに利き先の集合を飽和カウンタへ足すので、升を1つずつ数えずに済む。
/// 親の段の値と排他的論理和を取れば、段階が変わった升の集合になる。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EffectLevels {
    /// `ge[色][k]` は、その色の利きが k+1 枚以上ある升
    pub ge: [[Bitboard; 3]; 2],
}

impl EffectLevels {
    pub const EMPTY: EffectLevels = EffectLevels {
        ge: [[Bitboard::EMPTY; 3]; 2],
    };

    /// 局面から作る。数え方は `effect_counts` と同じで、一致はテストで確かめる。
    ///
    /// 駒種ごとに集合から引く（ADR-0242の段3）。駒ごとに `attacks` の分岐を
    /// 通らないので、駒種の分岐の外れが減る。
    pub fn build(pos: &Position) -> EffectLevels {
        let occ = pos.occupied();
        let mut l = EffectLevels::EMPTY;
        for c in [Color::Black, Color::White] {
            // 歩の利き先は駒ごとに別の升なので、集合のまま1段ずらして1回で足せる
            let pawns = pos.pieces(c, PieceType::PAWN);
            l.add(
                c,
                match c {
                    Color::Black => pawns.shift_up(),
                    Color::White => pawns.shift_down(),
                },
            );
            for sq in pos.pieces(c, PieceType::KNIGHT) {
                l.add(c, knight_attacks(c, sq));
            }
            for sq in pos.pieces(c, PieceType::SILVER) {
                l.add(c, silver_attacks(c, sq));
            }
            for sq in pos.golds(c) {
                l.add(c, gold_attacks(c, sq));
            }
            // 玉の無い局面（詰将棋など）もあるので、集合から引く
            for sq in pos.pieces(c, PieceType::KING) {
                l.add(c, king_attacks(sq));
            }
            for sq in pos.pieces(c, PieceType::LANCE) {
                l.add(c, lance_attacks(c, sq, occ));
            }
            for sq in pos.pieces(c, PieceType::BISHOP) {
                l.add(c, bishop_attacks(sq, occ));
            }
            for sq in pos.pieces(c, PieceType::ROOK) {
                l.add(c, rook_attacks(sq, occ));
            }
            for sq in pos.pieces(c, PieceType::HORSE) {
                l.add(c, bishop_attacks(sq, occ) | king_attacks(sq));
            }
            for sq in pos.pieces(c, PieceType::DRAGON) {
                l.add(c, rook_attacks(sq, occ) | king_attacks(sq));
            }
        }
        l
    }

    /// 色ごとの「k+1枚以上」の升の集合（81升の通し番号のビット）から作る。
    /// 配列盤（ADR-0241の段階6）が升ごとの利き数から作った集合を受け取る口である。
    #[inline]
    pub fn from_bits(bits: [[u128; 3]; 2]) -> EffectLevels {
        let mut l = EffectLevels::EMPTY;
        for (c, g) in bits.iter().enumerate() {
            for (k, &b) in g.iter().enumerate() {
                l.ge[c][k] = Bitboard::from_bits(b);
            }
        }
        l
    }

    #[inline(always)]
    fn add(&mut self, c: Color, x: Bitboard) {
        let g = &mut self.ge[c.index()];
        g[2] |= g[1] & x;
        g[1] |= g[0] & x;
        g[0] |= x;
    }

    /// 升sqへの色cの利き数を、3で打ち切って返す。
    #[inline]
    pub fn level(&self, c: Color, sq: Square) -> u8 {
        let g = &self.ge[c.index()];
        u8::from(g[0].test(sq)) + u8::from(g[1].test(sq)) + u8::from(g[2].test(sq))
    }

    /// 2つの局面で、どちらかの色の段階が違う升。
    #[inline]
    pub fn diff(&self, other: &EffectLevels) -> Bitboard {
        let mut d = Bitboard::EMPTY;
        for c in 0..2 {
            for k in 0..3 {
                d |= self.ge[c][k] ^ other.ge[c][k];
            }
        }
        d
    }
}

/// 升の中身と両色の利き数を、視点に依らない形で1語に詰める（ADR-0237のD2）。
///
/// 上位が駒（0は空き、先手の14種を1〜14、後手を15〜28）、下位4ビットが
/// 先手と後手の利き数（各2ビット、3で打ち切り）である。視点ごとの特徴の
/// indexは `effect_code_index` で戻す。
#[inline]
pub fn effect_code<B: crate::nnue_acc::EvalBoard + ?Sized>(
    pos: &B,
    levels: &EffectLevels,
    sq: Square,
) -> u16 {
    let pc = pos.piece_on(sq);
    let p = if pc.is_empty() {
        0
    } else {
        let kind = u16::from(pc.piece_type().0) - 1;
        if pc.color() == Color::Black {
            kind
        } else {
            kind + 14
        }
    };
    p << 4
        | u16::from(levels.level(Color::Black, sq)) << 2
        | u16::from(levels.level(Color::White, sq))
}

/// `effect_code` の語を、視点cの特徴のindexへ戻す。`effect_active` と同じ値になる。
///
/// 語は `駒 << 4 | 先手の段階 << 2 | 後手の段階` で、`effect_index` の升より下の桁と
/// 同じ並びになる。先手の視点は升の起点に語を足すだけで済む。後手の視点は、
/// 自他を入れ替えた語を表で引く（ADR-0241の第15回）。
#[inline(always)]
pub fn effect_code_index(c: Color, sq: Square, code: u16) -> u32 {
    const PER_SQ: u32 = (EFF_PIECES * EFF_LEVELS * EFF_LEVELS) as u32;
    match c {
        Color::Black => sq.index() as u32 * PER_SQ + u32::from(code),
        Color::White => {
            sq.inv().index() as u32 * PER_SQ + u32::from(WHITE_CODE[usize::from(code) & 511])
        }
    }
}

/// 後手の視点の語。駒の先後と、両色の段階を入れ替える。
static WHITE_CODE: [u16; 512] = {
    let mut t = [0u16; 512];
    let mut code = 0;
    while code < 512 {
        let p = code >> 4;
        let (b, w) = ((code >> 2) & 3, code & 3);
        let p = if p == 0 {
            0
        } else if p <= 14 {
            p + 14
        } else {
            p - 14
        };
        t[code] = ((p << 4) | (w << 2) | b) as u16;
        code += 1;
    }
    t
};

/// 側の入力がF2と最小の攻め駒の組（`-f2_minatt`）で、両方を升の語の差分で持つ構成か（ADR-0248）。
///
/// 升の語は `nnue_acc::EvalBoard::side_codes` が作る16ビットで、下位9ビットがF2の語
/// （`effect_code`）、上位7ビットが `先手の階級 * 11 + 後手の階級` である。対の積を掛ける構成は含めない。
pub const SIDE_MINATT_INC: bool = EFF_GENERIC
    && !EFF_PAIR
    && EFF_KINDS.len() == 2
    && EFF_KINDS[0] == crate::side_feat::KIND_F2
    && EFF_KINDS[1] == crate::side_feat::KIND_MINATT;

// build.rsが出すcfg `eff_acc32`（側のaccをi32で持つ）は、この構成と一致させる
const _: () = assert!(SIDE_MINATT_INC == cfg!(eff_acc32));

/// i32の側のaccが溢れないこと。1升に立つのはF2と最小の攻め駒の1つずつで、81升の
/// 162行とバイアスをi16の最大の絶対値で足しても、i32に十分収まる。だから読み込みで
/// 重みを検査しなくても、差分の値はi32で作り直す値と一致する（ADR-0249）。
const _: () = assert!((162 + 1) * 32768 < i32::MAX as i64);

/// F2の表の行数。最小の攻め駒の行はこの後ろに続く（`side_feat::ROWS` の順）。
const F2_ROWS: u32 = (81 * EFF_PIECES * EFF_LEVELS * EFF_LEVELS) as u32;
/// 最小の攻め駒の階級の数（`side_feat` と同じ）。
const MINATT_CLASSES: u32 = 11;
/// 最小の攻め駒の表で、1升が持つ行数。
const MINATT_PER_SQ: u32 = EFF_PIECES as u32 * MINATT_CLASSES * MINATT_CLASSES;

/// `side_codes` の語のうちF2の部分を、視点cの特徴のindexへ戻す。
#[inline(always)]
pub fn side_f2_index(c: Color, sq: Square, code: u16) -> u32 {
    effect_code_index(c, sq, code & 0x1FF)
}

/// `side_codes` の語のうち最小の攻め駒の部分を、視点cの特徴のindexへ戻す。
/// `side_feat` の `KIND_MINATT` と同じ値になる。
#[inline(always)]
pub fn side_minatt_index(c: Color, sq: Square, code: u16) -> u32 {
    let k = usize::from(code >> 4);
    match c {
        Color::Black => {
            let (p, m) = (k as u32 & 31, k as u32 >> 5);
            F2_ROWS + sq.index() as u32 * MINATT_PER_SQ + p * MINATT_CLASSES * MINATT_CLASSES + m
        }
        Color::White => {
            F2_ROWS + sq.inv().index() as u32 * MINATT_PER_SQ + u32::from(WHITE_MINATT[k & 4095])
        }
    }
}

/// 後手の視点の、升の中で閉じた最小の攻め駒の行。添字は `語 >> 4`（駒5ビット＋階級の組7ビット）で、
/// 駒の先後と、両者の階級を入れ替える。
static WHITE_MINATT: [u16; 4096] = {
    let mut t = [0u16; 4096];
    let mut k = 0;
    while k < 4096 {
        let (p, m) = (k & 31, k >> 5);
        if p <= 28 && m < 121 {
            let p = if p == 0 {
                0
            } else if p <= 14 {
                p + 14
            } else {
                p - 14
            };
            let (b, w) = (m / 11, m % 11);
            t[k] = (p * 121 + w * 11 + b) as u16;
        }
        k += 1;
    }
    t
};

/// `SIDE_MINATT_INC` の構成で、側のaccumulatorが取りうる値の上限（絶対値）を重みから見積もる。
///
/// 1升に立つのはF2と最小の攻め駒の1つずつなので、升ごとに各表の最大の絶対値を足す。
/// accはi32で持つので読み込みでは使わない。テストで、i16を超えるネットを作れたことを確かめる。
#[cfg(test)]
pub fn side_minatt_acc_bound(net: &NnueNetwork) -> i64 {
    if !SIDE_MINATT_INC {
        unreachable!("f2_minatt の構成でだけ呼ぶ");
    }
    const WIDTH: usize = if EFF_OUT == 0 { 1 } else { EFF_OUT };
    let mut sum: Vec<i64> = net.eff_b.iter().map(|&b| i64::from(b).abs()).collect();
    let f2 = F2_ROWS as usize * WIDTH;
    let blocks = net.eff_w[..f2]
        .chunks(EFF_PIECES * EFF_LEVELS * EFF_LEVELS * WIDTH)
        .chain(net.eff_w[f2..].chunks(MINATT_PER_SQ as usize * WIDTH));
    for rows in blocks {
        let mut most = vec![0i64; EFF_OUT];
        for row in rows.chunks(WIDTH) {
            for (m, &w) in most.iter_mut().zip(row) {
                *m = (*m).max(i64::from(w).abs());
            }
        }
        for (s, m) in sum.iter_mut().zip(&most) {
            *s += m;
        }
    }
    sum.into_iter().max().unwrap_or(0)
}

/// 連結ベクトルの利きの部分（両視点）を埋める（ADR-0237）。手番視点が先に並ぶ。
/// `EFF_OUT` が0の構成では何もしない。
#[inline]
pub fn fill_effect(net: &NnueNetwork, pos: &Position, concat: &mut [u8; CONCAT]) {
    if EFF_OUT == 0 {
        return;
    }
    if EFF_GENERIC {
        crate::side_feat::fill(net, pos, concat);
        return;
    }
    let counts = effect_counts(pos);
    let stm = pos.side_to_move();
    let mut feats = [0u32; 81];
    for (k, c) in [(0usize, stm), (1, stm.flip())] {
        effect_active(pos, c, &counts, &mut feats);
        let at = EFF_BASE + k * EFF_OUT;
        effect_transform(net, &feats, &mut concat[at..at + EFF_OUT]);
    }
}

#[inline]
fn clip(v: i32) -> u8 {
    v.clamp(0, 127) as u8
}

/// accumulatorの前半と後半を要素ごとに掛けて活性にする（ADR-0171）。
///
/// `(clip(a) * clip(b) + 64) >> 7` の最大は 126 で、127には届かない。
/// **学習側が同じ127/128を掛けているので、これで縮尺が合う。**掛けずに
/// 127で割ると量子化の係数を後段まで直す羽目になる。
///
/// 64を足すのは切り捨てでなく四捨五入にするためである。足さないと活性が
/// 平均0.48ぶん小さく出る。1024次元すべてに同じ向きの偏りが乗るので、
/// バイアスでは吸収されない（足すと平均誤差が0.49から0.19へ下がる）。
#[inline]
pub(crate) fn pair_activation(a: i32, b: i32) -> u8 {
    ((i32::from(clip(a)) * i32::from(clip(b)) + 64) >> 7) as u8
}

/// スカラー全計算の評価（手番視点、歩=90スケール）。
/// 差分計算・SIMDの正解基準（ADR-0035, 0036）。
pub fn evaluate_scalar(net: &NnueNetwork, pos: &Position) -> Value {
    let stm = pos.side_to_move();
    let mut features = Vec::with_capacity(64);
    let mut concat = [0u8; CONCAT];

    // FT: 手番視点 → [0..HALF)、非手番視点 → [HALF..CONCAT)
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
        // accの前半と後半を対にして掛ける（ADR-0171）
        for o in 0..HALF {
            concat[half * HALF + o] = pair_activation(acc[o], acc[o + HALF]);
        }
    }
    fill_effect(net, pos, &mut concat);
    crate::submodel::fill(&net.sub, pos, &mut concat);

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

    /// 左右反転したsfen。行の順序も手番も手駒も変えず、各行の升だけ
    /// 逆順にする。将棋の駒の動きは左右対称なので、この像は元の局面と
    /// 同じ強さのはずである。
    fn mirror_lr_sfen(pos: &Position) -> String {
        let sfen = pos.to_sfen();
        let parts: Vec<&str> = sfen.split(' ').collect();
        let rows: Vec<String> = parts[0]
            .split('/')
            .map(|row| {
                let mut cells: Vec<String> = Vec::new();
                let mut chars = row.chars();
                while let Some(ch) = chars.next() {
                    if ch == '+' {
                        let p = chars.next().expect("成駒の駒種がない");
                        cells.push(format!("+{p}"));
                    } else {
                        cells.push(ch.to_string());
                    }
                }
                cells.reverse();
                cells.concat()
            })
            .collect();
        format!("{} {} {} {}", rows.join("/"), parts[1], parts[2], parts[3])
    }

    /// 左右反転で評価値がどれだけ動くかを測る（ADR-0157の先行指標）。
    ///
    /// HalfKPは玉位置を81通りそのまま持つので、左右対称性は構造では
    /// 保証されず、学習で獲得するしかない。**獲得できていない量が、
    /// 特徴側でミラーを畳む改造の伸びしろになる。**
    ///
    /// 現行ネットが要るので既定では走らせない。
    /// `EVAL_FILE=... cargo test --release -p himawari-engine -- --ignored --nocapture mirror_lr_gap`
    #[test]
    #[ignore = "現行ネットが要る調査（ADR-0157）"]
    fn mirror_lr_gap() {
        let path = std::env::var("EVAL_FILE").expect("EVAL_FILE を指定する");
        let f = std::fs::File::open(&path).expect("EVAL_FILEを開けない");
        let mut r = std::io::BufReader::new(f);
        let (net, _) = crate::nnue_io::load(&mut r).expect("ネットを読めない");

        // SFEN_FILEを渡すとその局面集で測る。渡さないとランダムに進めた
        // 局面で測る。教師データの分布から離れるほど評価は荒れるので、
        // 実戦的な局面集での値を正とする
        let sfens: Vec<String> = match std::env::var("SFEN_FILE") {
            Ok(p) => std::fs::read_to_string(&p)
                .expect("SFEN_FILEを読めない")
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.trim_start_matches("sfen ").to_string())
                .collect(),
            Err(_) => Vec::new(),
        };

        let mut rng = Rng(0xDEAD_BEEF_1234_5678);
        let mut gaps: Vec<i32> = Vec::new();
        let mut scores: Vec<i32> = Vec::new();
        let count = if sfens.is_empty() {
            2000
        } else {
            sfens.len().min(20000)
        };
        for i in 0..count {
            let pos = if sfens.is_empty() {
                let mut pos = Position::from_sfen(SFEN_STARTPOS).expect("初期局面");
                let plies = 20 + rng.next() % 80;
                for _ in 0..plies {
                    let mut list = MoveList::default();
                    generate_legal(&pos, true, &mut list);
                    if list.is_empty() {
                        break;
                    }
                    let m = list.as_slice()[(rng.next() % list.len() as u64) as usize];
                    pos.do_move(m);
                }
                pos
            } else {
                Position::from_sfen(&sfens[i]).expect("SFEN_FILEの局面が壊れている")
            };
            let mirrored =
                Position::from_sfen(&mirror_lr_sfen(&pos)).expect("鏡像のsfenが壊れている");
            let a = evaluate_scalar(&net, &pos);
            let b = evaluate_scalar(&net, &mirrored);
            gaps.push((a - b).abs());
            scores.push(a.abs());
        }
        gaps.sort_unstable();
        scores.sort_unstable();
        let pick = |v: &[i32], q: f64| v[((v.len() - 1) as f64 * q) as usize];
        let mean = |v: &[i32]| v.iter().map(|&x| i64::from(x)).sum::<i64>() / v.len() as i64;
        // 歩=90スケール（ADR-0036）
        println!(
            "左右ミラーの評価差（歩=90）: 平均{} 中央{} p90={} p99={} 最大{}",
            mean(&gaps),
            pick(&gaps, 0.5),
            pick(&gaps, 0.9),
            pick(&gaps, 0.99),
            gaps[gaps.len() - 1]
        );
        println!(
            "比較用の評価値の規模: 平均|score|={} 中央={}",
            mean(&scores),
            pick(&scores, 0.5)
        );
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

    /// 利きの特徴を確かめる局面。検証用の3局面と、ランダムに進めた局面。
    fn effect_positions() -> Vec<Position> {
        let mut positions: Vec<Position> = EFFECT_SFENS
            .iter()
            .map(|s| Position::from_sfen(s).expect("検証用sfen"))
            .collect();
        for seed in 1..=8u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
            for ply in 0..80 {
                let mut list = MoveList::default();
                generate_legal(&pos, true, &mut list);
                if list.is_empty() {
                    break;
                }
                if ply % 7 == 0 {
                    positions.push(Position::from_sfen(&pos.to_sfen()).unwrap());
                }
                let m = list.as_slice()[(rng.next() % list.len() as u64) as usize];
                pos.do_move(m);
            }
        }
        positions
    }

    /// 利き数は探索が使う `attackers_to` と一致する（ADR-0237）。実装を
    /// 共有しない別経路で、遮り・玉の利き・駒自身を数えないことをまとめて確かめる。
    #[test]
    fn effect_counts_match_attackers_to() {
        for pos in &effect_positions() {
            let n = effect_counts(pos);
            let occ = pos.occupied();
            for i in 0..81u8 {
                let sq = Square::from_index(i);
                for c in [Color::Black, Color::White] {
                    assert_eq!(
                        u32::from(n[c.index()][sq.index()]),
                        pos.attackers_to(c, sq, occ).count(),
                        "sq={} c={c:?} {}",
                        sq.to_usi(),
                        pos.to_sfen()
                    );
                }
            }
        }
    }

    /// 特徴のindexを升・駒・自他の利き数へ戻し、局面から直接数えた値と
    /// 突き合わせる（ADR-0237のF2）。視点の反転と自他の入れ替えもここで見る。
    #[test]
    fn effect_features_decode_to_the_board() {
        for pos in &effect_positions() {
            let occ = pos.occupied();
            for c in [Color::Black, Color::White] {
                let counts = effect_counts(pos);
                let mut f = [0u32; 81];
                effect_active(pos, c, &counts, &mut f);
                for (view, &idx) in f.iter().enumerate() {
                    let idx = idx as usize;
                    assert!(idx < EFF_IN, "範囲外: {idx}");
                    let opp = idx % EFF_LEVELS;
                    let own = idx / EFF_LEVELS % EFF_LEVELS;
                    let pc = idx / (EFF_LEVELS * EFF_LEVELS) % EFF_PIECES;
                    assert_eq!(
                        idx / (EFF_LEVELS * EFF_LEVELS * EFF_PIECES),
                        view,
                        "升の位置"
                    );
                    let sq = if c == Color::Black {
                        Square::from_index(view as u8)
                    } else {
                        Square::from_index(view as u8).inv()
                    };
                    let cap = |n: u32| (n as usize).min(EFF_LEVELS - 1);
                    assert_eq!(own, cap(pos.attackers_to(c, sq, occ).count()), "自分の利き");
                    assert_eq!(
                        opp,
                        cap(pos.attackers_to(c.flip(), sq, occ).count()),
                        "相手の利き"
                    );
                    let p = pos.piece_on(sq);
                    let want = if p.is_empty() {
                        0
                    } else {
                        let side = if p.color() == c { 0 } else { 14 };
                        usize::from(p.piece_type().0) - 1 + side
                    };
                    assert_eq!(pc, want, "乗っている駒 sq={}", sq.to_usi());
                }
            }
        }
    }

    /// 利きの特徴は手番視点で作るので、180度回して先後を入れ替えた局面と
    /// 同じ並びになる。HalfKPと同じ性質で、学習が先後で割れない。
    #[test]
    fn effect_features_follow_the_perspective() {
        for pos in &effect_positions() {
            let mirrored = Position::from_sfen(&mirror_sfen(pos)).unwrap();
            let (a, b) = (effect_counts(pos), effect_counts(&mirrored));
            for c in [Color::Black, Color::White] {
                let mut fa = [0u32; 81];
                let mut fb = [0u32; 81];
                effect_active(pos, c, &a, &mut fa);
                effect_active(&mirrored, c.flip(), &b, &mut fb);
                assert_eq!(fa, fb, "鏡像で特徴が一致しない: {}", pos.to_sfen());
            }
        }
    }

    /// 段階の集合（D2）は、升ごとに数えた利き数を3で打ち切った値と一致する。
    /// 升の語から戻した特徴のindexも、`effect_active` と一致する（ADR-0237）。
    #[test]
    fn effect_levels_and_codes_match_counts() {
        for pos in &effect_positions() {
            let counts = effect_counts(pos);
            let levels = EffectLevels::build(pos);
            for i in 0..81u8 {
                let sq = Square::from_index(i);
                for c in [Color::Black, Color::White] {
                    assert_eq!(
                        levels.level(c, sq),
                        counts[c.index()][sq.index()].min(3),
                        "sq={} c={c:?} {}",
                        sq.to_usi(),
                        pos.to_sfen()
                    );
                }
            }
            for c in [Color::Black, Color::White] {
                let mut want = [0u32; 81];
                effect_active(pos, c, &counts, &mut want);
                let mut got = [0u32; 81];
                for i in 0..81u8 {
                    let sq = Square::from_index(i);
                    let view = if c == Color::Black { sq } else { sq.inv() };
                    got[view.index()] = effect_code_index(c, sq, effect_code(pos, &levels, sq));
                }
                assert_eq!(got, want, "視点{c:?} {}", pos.to_sfen());
            }
        }
    }

    /// 遮りが特徴に入る。香の前に駒を置くと、その先の升の利き数が落ちる。
    /// 遮りを無視した利き数は駒の位置の線形和で、FTがすでに表せる（ADR-0237）。
    #[test]
    fn effect_features_see_blocking() {
        let open = Position::from_sfen("8k/9/9/9/4L4/9/9/9/K8 b - 1").unwrap();
        let blocked = Position::from_sfen("8k/9/4P4/9/4L4/9/9/9/K8 b - 1").unwrap();
        let at = Square::from_usi("5b").expect("升").index();
        let (o, b) = (effect_counts(&open), effect_counts(&blocked));
        assert_eq!(o[Color::Black.index()][at], 1);
        assert_eq!(b[Color::Black.index()][at], 1, "5二は歩が肩代わりする");
        let at = Square::from_usi("5a").expect("升").index();
        assert_eq!(o[Color::Black.index()][at], 1);
        assert_eq!(b[Color::Black.index()][at], 0, "5一は香の利きが切れる");
    }

    /// 利きの活性は連結ベクトルの後ろ（`EFF_BASE` 以降）に入り、第1層の
    /// 利きの列を通って評価値に効く（ADR-0237）。利きを持たない構成では
    /// 何も書かない。
    #[test]
    fn effect_activations_reach_the_output() {
        let pos = Position::from_sfen(EFFECT_SFENS[2]).unwrap();
        let mut net = NnueNetwork::random(11);
        assert_eq!(net.eff_w.len(), EFF_IN * EFF_OUT);
        let mut concat = [0u8; CONCAT];
        fill_effect(&net, &pos, &mut concat);
        assert!(
            concat[..EFF_BASE].iter().all(|&x| x == 0),
            "HalfKPの位置へ書かない"
        );
        if EFF_OUT == 0 {
            return;
        }
        // 表し方を替えた構成（ADR-0245）は数百の行が立つので、行の和に負けない大きさにする
        net.eff_b.fill(20000);
        fill_effect(&net, &pos, &mut concat);
        // 対の積は127×127/128を四捨五入して126が上限になる（`pair_activation`）
        let top = if EFF_PAIR { 126 } else { 127 };
        assert!(
            concat[EFF_BASE..EFF_BASE + 2 * EFF_ACT]
                .iter()
                .all(|&x| x == top),
            "バイアスで飽和する"
        );
        // 第1層の利きの列だけを動かすと評価値が変わる。HalfKPの列は触らない
        let base = evaluate_scalar(&net, &pos);
        let mut moved = false;
        for scale in [1i8, -1, 3, -3] {
            for (i, w) in net.w2.iter_mut().enumerate() {
                if (EFF_BASE..EFF_BASE + 2 * EFF_ACT).contains(&(i % CONCAT)) {
                    *w = scale;
                }
            }
            net = net.finish();
            moved |= evaluate_scalar(&net, &pos) != base;
        }
        assert!(moved, "利きの活性が後段へ届いていない");
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

    /// 活性特徴数 = 盤上の玉以外の駒 + 持ち駒総数。
    #[test]
    fn halfkp_active_count() {
        let pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
        let mut v = Vec::new();
        halfkp_active(&pos, Color::Black, &mut v);
        assert_eq!(v.len(), 38, "平手は玉以外38枚・持ち駒なし");
        assert!(v.iter().all(|&f| (f as usize) < FT_IN));
    }
}
