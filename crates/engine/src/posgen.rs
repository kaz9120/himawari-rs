//! 事前学習用の局面生成（ADR-0133）。
//!
//! 利き予測は決定的な構造タスクで、的は局面の分布に依存しない。だから
//! 教師データファイルが要らず、局面をその場で作れる。作った局面を使い捨てに
//! すれば同じ局面は二度と出ず、訓練損失がそのまま未見データの損失になる。
//! 過学習が構造的に起こらない。
//!
//! 局面は初期局面から合法手をランダムに指して作り、**1手ごとに1局面**拾う。
//! 1回のplayoutで `max_plies` 個取れるので、指し手生成の費用が割り勘になる。
//! 1局面ごとに初期局面から指し直すと桁で遅くなる。
//!
//! 千日手は判定しない。局面として正しければ利きの的も正しい。
//!
//! 推論側は使わない。学習の抽出パスにしかない。

use himawari_core::{MoveList, Position, SFEN_STARTPOS, generate_legal};

use crate::nnue::{EFFECT_LEN, effect_labels, halfkp_active};

/// 黄金比の奇数。SplitMix64の増分に使う定数で、種の導出にも同じものを使う。
const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

/// SplitMix64の攪拌。外部の乱数クレートを足さずに済ませる。
///
/// 種から種を導くときにも使う。塊の番号をそのまま種にすると隣り合う塊の
/// 乱数列が相関するので、必ずこの関数を通す。
const fn mix(z: u64) -> u64 {
    let z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// SplitMix64。状態は64ビットだけで、同じ種からは必ず同じ列が出る。
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GAMMA);
        mix(self.state)
    }

    /// `0..n` の一様乱数。128ビット乗算の上位を取る（Lemireの縮約）。
    /// 剰余と違い `n` が2の冪でなくても偏りが実用上出ない。
    fn below(&mut self, n: usize) -> usize {
        debug_assert!(n > 0);
        ((u128::from(self.next_u64()) * n as u128) >> 64) as usize
    }
}

/// 生成した1局面。`extract_batch` が返すものと同じ中身を持つ。
/// 評価値の的はないので、教師信号は呼び出し側が埋める。
pub struct GeneratedSample {
    /// 手番視点のHalfKP活性特徴。
    pub stm: Vec<u32>,
    /// 相手視点のHalfKP活性特徴。
    pub opp: Vec<u32>,
    /// 短い利き（遮りが起こらない利き）の升ごとの数。
    pub short: [u8; EFFECT_LEN],
    /// 長い利き（飛び駒の隣接より先）の升ごとの数。
    pub long: [u8; EFFECT_LEN],
}

/// 並列化の単位。1塊を1スレッドが順に作る。
///
/// 塊を固定長にすると、分割の仕方がスレッド数や実行時のスケジュールに
/// 依らなくなる。**同じ種なら同じ局面列が出る**のはこのためである。
pub const GEN_CHUNK: usize = 1024;

/// `count` 個を塊へ割り、塊ごとの (種, 個数) を返す。
///
/// 種は `seed` と塊の番号から決定的に導く。呼び出し側はこの順序を保って
/// 連結すれば、並列に作っても結果が変わらない。
pub fn chunk_specs(count: usize, seed: u64) -> Vec<(u64, usize)> {
    (0..count.div_ceil(GEN_CHUNK))
        .map(|i| {
            let rest = count - i * GEN_CHUNK;
            (
                mix(seed ^ (i as u64).wrapping_mul(GAMMA)),
                rest.min(GEN_CHUNK),
            )
        })
        .collect()
}

/// 1塊ぶんの局面を作る。同じ引数からは必ず同じ結果が出る。
///
/// `max_plies` は1回のplayoutで進める上限手数。合法手が尽きたら（詰み・
/// ステイルメイト）その場で打ち切り、次のplayoutを始める。
///
/// # Panics
///
/// `max_plies` が0だと1局面も作れず、`count` を満たせない。
pub fn generate_chunk(seed: u64, count: usize, max_plies: u16) -> Vec<GeneratedSample> {
    let mut out = Vec::with_capacity(count);
    let mut stm = Vec::new();
    let mut opp = Vec::new();
    visit_positions(seed, count, max_plies, |pos| {
        let side = pos.side_to_move();
        halfkp_active(pos, side, &mut stm);
        halfkp_active(pos, side.flip(), &mut opp);
        let (short, long) = effect_labels(pos);
        out.push(GeneratedSample {
            stm: stm.clone(),
            opp: opp.clone(),
            short,
            long,
        });
    });
    out
}

/// 局面を `count` 個作り、1つずつ `f` へ渡す。
///
/// playoutを回し、1手指すごとに現れた局面を渡す。合法手が尽きたら
/// 打ち切って次のplayoutへ移る。局面そのものを見たい呼び出し側
/// （テスト）と、特徴へ落とす呼び出し側で経路を分けない。
fn visit_positions(seed: u64, count: usize, max_plies: u16, mut f: impl FnMut(&Position)) {
    assert!(max_plies > 0, "max_pliesは1以上が要る");
    let mut rng = SplitMix64::new(seed);
    let mut list = MoveList::default();
    let mut done = 0;
    while done < count {
        let mut pos = Position::from_sfen(SFEN_STARTPOS).expect("初期局面のsfen");
        for _ in 0..max_plies {
            list.clear();
            generate_legal(&pos, false, &mut list);
            if list.is_empty() {
                break;
            }
            pos.do_move(list.as_slice()[rng.below(list.len())]);
            f(&pos);
            done += 1;
            if done == count {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use himawari_core::{Color, PieceType};
    use std::collections::HashSet;

    /// `chunk_specs` の順に塊を作って連結する。py側の並列生成と同じ結果に
    /// なる経路で、テストはこちらを叩く。
    fn generate(count: usize, seed: u64, max_plies: u16) -> Vec<GeneratedSample> {
        chunk_specs(count, seed)
            .into_iter()
            .flat_map(|(s, c)| generate_chunk(s, c, max_plies))
            .collect()
    }

    fn key(s: &GeneratedSample) -> (Vec<u32>, Vec<u32>, Vec<u8>, Vec<u8>) {
        (
            s.stm.clone(),
            s.opp.clone(),
            s.short.to_vec(),
            s.long.to_vec(),
        )
    }

    /// 同じ種からは同じ局面列が出る。学習を再現するための前提になる。
    #[test]
    fn same_seed_gives_the_same_positions() {
        let a = generate(3000, 7, 64);
        let b = generate(3000, 7, 64);
        assert_eq!(a.len(), 3000);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(key(x), key(y), "同じ種で局面が変わった");
        }
    }

    /// 違う種からは違う局面列が出る。エポックごとに種をずらす運用が
    /// 意味を持つ条件である。
    #[test]
    fn different_seed_gives_different_positions() {
        let a = generate(3000, 7, 64);
        let b = generate(3000, 8, 64);
        let same = a.iter().zip(&b).filter(|(x, y)| key(x) == key(y)).count();
        // 序盤の数手は種が違っても重なりうる。全部一致していないことを見る
        assert!(
            same * 2 < a.len(),
            "違う種で局面がほぼ一致した: {same}/3000"
        );
    }

    /// 生成した局面が将棋の局面として成り立っている。合法手だけを指すので
    /// 玉は取られず、盤上と持ち駒を合わせた枚数も40のままになる。
    #[test]
    fn generated_positions_are_legal() {
        let mut seen = 0;
        visit_positions(mix(1), 1000, 256, |pos| {
            seen += 1;
            for c in [Color::Black, Color::White] {
                let pc = pos.piece_on(pos.king(c));
                assert_eq!(
                    (pc.piece_type(), pc.color()),
                    (PieceType::KING, c),
                    "玉の升に自分の玉がない: {}",
                    pos.to_sfen()
                );
            }
            let on_board = pos.occupied().count();
            let in_hand: u32 = [Color::Black, Color::White]
                .iter()
                .flat_map(|&c| PieceType::HAND_KINDS.iter().map(move |&pt| (c, pt)))
                .map(|(c, pt)| pos.hand(c).count(pt))
                .sum();
            assert_eq!(on_board + in_hand, 40, "駒数が合わない: {}", pos.to_sfen());
        });
        assert_eq!(seen, 1000);
    }

    /// 活性特徴の数が妥当である。HalfKPは玉以外の40−2枚を数えるので、
    /// 駒が減らない将棋では常に38になる（halfkaは相手玉で+1）。
    /// 上限を超えないことも見る。
    #[test]
    fn feature_counts_stay_in_range() {
        let max = if cfg!(feature = "halfka") { 39 } else { 38 };
        for s in generate(2000, 3, 128) {
            for feats in [&s.stm, &s.opp] {
                assert!(
                    (2..=max).contains(&feats.len()),
                    "活性特徴の数が範囲外: {}",
                    feats.len()
                );
            }
            // 利きは必ず片方の玉ぶんが立つ。全ゼロの的は出ない
            assert!(s.short.iter().any(|&x| x > 0));
        }
    }

    /// 局面が多様である。初期局面付近は指し手が30通りしかないので重なりうる。
    /// 1000局面での実測は重複0件で、閾値は余裕を見て1割に置く。
    #[test]
    fn generated_positions_are_diverse() {
        let samples = generate(1000, 11, 256);
        let uniq: HashSet<_> = samples.iter().map(key).collect();
        let dup = samples.len() - uniq.len();
        assert!(
            dup * 10 < samples.len(),
            "重複が多い: {dup}/{}",
            samples.len()
        );
    }
}
