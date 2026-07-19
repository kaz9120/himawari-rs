//! mate_1plyの健全性テスト（ADR-0029）。
//!
//! ランダムプレイアウトの全局面で、案B（mate_1ply）の返す手が
//! 実際に詰みであること（誤検出ゼロ）をオラクル同等の検証で確認し、
//! 近接王手クラスの詰みを見逃さないことを固定局面で確認する。

use himawari_core::{Move, MoveList, Position, SFEN_STARTPOS, generate_legal};
use himawari_engine::mate::{mate_1ply, mate_1ply_oracle};

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

fn verify_is_mate(pos: &mut Position, m: Move) {
    pos.do_move(m);
    assert!(pos.in_check(), "詰み手のはずが王手でない: {}", m.to_usi());
    let mut ev = MoveList::default();
    generate_legal(pos, true, &mut ev);
    assert!(ev.is_empty(), "詰み手のはずが回避がある: {}", m.to_usi());
    pos.undo_move(m);
}

/// 1手詰めの代表局面（tsumeスモークと同種）で見逃さないこと。
#[test]
fn finds_known_mates() {
    for sfen in [
        // 桂に支えられた金打ち
        "4k4/9/9/5N3/9/9/9/9/4K4 b G 1",
        "3k5/9/9/4N4/9/9/9/9/4K4 b G 1",
        // 頭金（5cの歩が支え）
        "4k4/9/4P4/9/9/9/9/9/4K4 b G 1",
    ] {
        let mut pos = Position::from_sfen(sfen).unwrap();
        let m = mate_1ply(&mut pos);
        assert!(m.is_some(), "1手詰めを見逃した: {sfen}");
        verify_is_mate(&mut pos, m.unwrap());
    }
}

/// ランダムプレイアウト全局面で誤検出ゼロ。オラクルとの関係も確認する。
#[test]
fn random_playouts_soundness() {
    let mut found = 0u32;
    let mut oracle_found = 0u32;
    for seed in 1..=100u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
        for _ in 0..80 {
            let mut list = MoveList::default();
            generate_legal(&pos, true, &mut list);
            if list.is_empty() {
                break;
            }
            if !pos.in_check() {
                if let Some(m) = mate_1ply(&mut pos) {
                    found += 1;
                    verify_is_mate(&mut pos, m);
                }
                if mate_1ply_oracle(&mut pos).is_some() {
                    oracle_found += 1;
                }
            }
            let m = list.as_slice()[(rng.next() % list.len() as u64) as usize];
            pos.do_move(m);
        }
    }
    // 見逃しは許容するが、オラクルが見つける詰みの大半は拾えるはず
    assert!(
        found * 10 >= oracle_found * 5,
        "検出率が低すぎる: mate_1ply {found} vs oracle {oracle_found}"
    );
}
