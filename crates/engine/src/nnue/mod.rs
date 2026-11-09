//! NNUE推論の骨格。
//!
//! HalfKPの差分FT（片視点FT_OUT次元×2視点）を連結し、
//! L1_OUT→L2_OUT→1で評価値を出す。本モジュールはスカラー基準実装
//! （正解器）。accumulator差分とSIMDはこの実装との完全一致を要求する
//! 形で後から積む。
//!
//! 次元は `build.rs` が環境変数 `HIMAWARI_ARCH` から生成する。

use himawari_core::attacks::{
    attacks, bishop_attacks, gold_attacks, king_attacks, knight_attacks, lance_attacks,
    rook_attacks, silver_attacks,
};
use himawari_core::{Bitboard, Color, PieceType, Position, Square, bonapiece};

use crate::value::Value;

mod effect;
mod effect_code;
mod halfkp;
mod labels;
mod reference;

pub use effect::*;
pub use effect_code::*;
pub use halfkp::*;
pub use labels::*;
pub use reference::*;

// FT_OUT・L1_OUT・L2_OUT・L1_PAD・ARCH を定義する。
include!(concat!(env!("OUT_DIR"), "/arch.rs"));

/// 隠れ層の入力次元（FT両視点＋利きの両視点）。
///
/// 片視点は `FT_OUT` 次元のaccumulatorを2つに割って要素ごとに掛けるので、
/// 出す活性は `FT_OUT / 2` 次元になる。両視点でその2倍。
/// 利きのaccumulatorを持つ構成では、その後ろへ `EFF_OUT` 次元を
/// 2視点ぶん連結する。`EFF_OUT` が0の構成では `FT_OUT` のままである。
pub const CONCAT: usize = FT_OUT + 2 * EFF_OUT;
/// 連結ベクトルのうち、HalfKPの片視点ぶんの幅。
pub const HALF: usize = FT_OUT / 2;
/// 連結ベクトルのうち、利きの活性が始まる位置。
pub const EFF_BASE: usize = FT_OUT;
/// 利きの特徴で、升に乗るものの種類。空き・自分の14種・相手の14種。
pub const EFF_PIECES: usize = 29;
/// 利き数の段階。0・1・2・3以上の4つ（技巧の3で打ち切り）。
pub const EFF_LEVELS: usize = 4;
// 側の特徴の総数 `EFF_IN` はbuild.rsが構成名から作る。種類を書かない
// 構成ではF2だけで、升×乗るもの×自分の利き数×相手の利き数になる
const _: () = assert!(EFF_GENERIC || EFF_IN == 81 * EFF_PIECES * EFF_LEVELS * EFF_LEVELS);
/// 評価値スケール。
pub const FV_SCALE: i32 = 16;
/// 入力特徴の総数。
pub const FT_IN: usize = 81 * bonapiece::FE_END as usize;

/// FT重みの格納型。accumulatorとFTバイアスはi16のままなので、
/// 活性（clipped ReLU）以降の精度は変わらない。
pub type FtWeight = i8;

/// i16で読んだFT重みを格納型へ移す。
///
/// 範囲を確かめる。**黙って切り詰めない。** 飽和は
/// 0.055%でも−59.3 Eloになるので、気づかず
/// 壊れたネットで対局する事故のほうが高くつく。
pub fn ft_w_from_i16(v: Vec<i16>) -> Result<Vec<FtWeight>, String> {
    if let Some(&bad) = v.iter().find(|&&x| !(-128..=127).contains(&i32::from(x))) {
        return Err(format!(
            "FT weight {bad} does not fit in i8; train the net with --ft-clip "
        ));
    }
    Ok(v.into_iter().map(|x| x as i8).collect())
}

/// 重み一式。隠れ層とFT重みはi8に量子化する。
/// FTバイアスとaccumulatorはi16で持つ。
pub struct NnueNetwork {
    /// FT重み。列優先: `ft_w[feature * FT_OUT + o]`。
    pub ft_w: Vec<FtWeight>,
    pub ft_b: Vec<i16>,
    /// 隠れ層1。行優先: `w2[row * CONCAT + i]`。
    pub w2: Vec<i8>,
    /// 同じ重みを4列チャンク単位で並べ替えた表。
    /// `w2` から機械的に作る派生表で、列駆動の推論だけが読む。
    /// **`w2` を差し替えたら `finish` を呼び直す。**
    pub w2_sparse: Vec<i8>,
    pub b2: Vec<i32>,
    /// 隠れ層2。行優先だが列幅は `L1_PAD` で、`L1_OUT` 以降の列は
    /// 常にゼロにする（入力側もゼロ埋めなので値には影響しない）。
    pub w3: Vec<i8>,
    pub b3: Vec<i32>,
    /// 出力層。入力は最後の隠れ層（隠れ層2つなら`L2_OUT`、1つなら`L1_OUT`）。
    pub w_out: Vec<i8>,
    pub b_out: i32,
    /// 利きの特徴の重み。列優先: `eff_w[feature * EFF_OUT + o]`。
    /// `EFF_OUT` が0の構成では空。
    pub eff_w: Vec<i16>,
    pub eff_b: Vec<i16>,
}

/// 出力層の入力次元。隠れ層2を挟むかで変わる。
pub const LAST_HIDDEN: usize = if L2_OUT != 0 { L2_OUT } else { L1_OUT };

/// 隠れ層1の重みを4列チャンク単位で並べ替える。
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
            w_out: r.i8v(LAST_HIDDEN),
            b_out: 0,
            eff_w: Vec::new(),
            eff_b: Vec::new(),
        };
        // 利きの重みは最後に引く。既定構成（EFF_OUT=0）の乱数列を変えない
        net.eff_w = r.i16v(EFF_IN * EFF_OUT, 16);
        net.eff_b = r.i16v(EFF_OUT, 128);
        net.finish()
    }
}

/// 隠れ層の重みを `行数 × used` から `行数 × stride` へ広げ、余った列を
/// ゼロで埋める。ファイルと学習側はゼロ埋めを持たず、推論だけがSIMDの
/// 都合で広い幅を使う。
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

#[inline]
fn clip(v: i32) -> u8 {
    v.clamp(0, 127) as u8
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
        assert!((81..88).contains(&from), "打つ駒は81以降: {from}");
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

    /// 左右反転で評価値がどれだけ動くかを測る。
    ///
    /// HalfKPは玉位置を81通りそのまま持つので、左右対称性は構造では
    /// 保証されず、学習で獲得するしかない。**獲得できていない量が、
    /// 特徴側でミラーを畳む改造の伸びしろになる。**
    ///
    /// 現行ネットが要るので既定では走らせない。
    /// `EVAL_FILE=... cargo test --release -p himawari-engine -- --ignored --nocapture mirror_lr_gap`
    #[test]
    #[ignore = "現行ネットが要る調査"]
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
        // 歩=90スケール
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

    /// 検証用に置いた局面。馬・竜・香を含む。ランダム対局40手では
    /// 成駒がほとんど出ないので、別に用意する
    const EFFECT_SFENS: [&str; 3] = [
        "8k/9/9/9/4+R4/9/4+B4/9/K8 b - 1",
        "8k/9/9/9/4L4/9/4l4/9/K8 b - 1",
        "8k/2n6/9/3s5/4B4/9/2g2N3/9/K8 w - 1",
    ];

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

    /// 利き数は探索が使う `attackers_to` と一致する。実装を
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
    /// 突き合わせる（F2）。視点の反転と自他の入れ替えもここで見る。
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

    /// 段階の集合は、升ごとに数えた利き数を3で打ち切った値と一致する。
    /// 升の語から戻した特徴のindexも、`effect_active` と一致する。
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
    /// 遮りを無視した利き数は駒の位置の線形和で、FTがすでに表せる。
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
    /// 利きの列を通って評価値に効く。利きを持たない構成では
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
        // 行の和に負けない大きさにする
        net.eff_b.fill(20000);
        fill_effect(&net, &pos, &mut concat);
        assert!(
            concat[EFF_BASE..EFF_BASE + 2 * EFF_OUT]
                .iter()
                .all(|&x| x == 127),
            "バイアスで飽和する"
        );
        // 第1層の利きの列だけを動かすと評価値が変わる。HalfKPの列は触らない
        let base = evaluate_scalar(&net, &pos);
        let mut moved = false;
        for scale in [1i8, -1, 3, -3] {
            for (i, w) in net.w2.iter_mut().enumerate() {
                if (EFF_BASE..EFF_BASE + 2 * EFF_OUT).contains(&(i % CONCAT)) {
                    *w = scale;
                }
            }
            net = net.finish();
            moved |= evaluate_scalar(&net, &pos) != base;
        }
        assert!(moved, "利きの活性が後段へ届いていない");
    }

    /// インターリーブ表は `w2` の並べ替えである。
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
