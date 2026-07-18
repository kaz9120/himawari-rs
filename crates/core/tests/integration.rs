//! Position・movegen・perftの統合テスト（ADR-0006, 0018）。
//!
//! do/undo往復の完全一致と、差分更新キー＝全計算キーの一致を
//! ランダムプレイアウトで検証する。perftは公開値と照合する。

use himawari_core::{
    Color, GenType, Move, MoveList, Position, Repetition, SFEN_STARTPOS, generate, generate_legal,
    perft, perft_slow,
};

fn apply(pos: &mut Position, moves: &[&str]) {
    for s in moves {
        let m = pos
            .move_from_usi(s)
            .unwrap_or_else(|| panic!("illegal: {s}"));
        pos.do_move(m);
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

#[test]
fn sfen_startpos_roundtrip() {
    let pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    assert_eq!(pos.to_sfen(), SFEN_STARTPOS);
    assert_eq!(pos.side_to_move(), Color::Black);
    assert!(!pos.in_check());
}

#[test]
fn sfen_rejects_invalid() {
    // 玉がない
    assert!(Position::from_sfen("9/9/9/9/9/9/9/9/9 b - 1").is_err());
    // 二歩
    assert!(Position::from_sfen("4k4/9/9/9/9/9/4P4/4P4/4K4 b - 1").is_err());
    // 行き所のない桂
    assert!(Position::from_sfen("N3k4/9/9/9/9/9/9/9/4K4 b - 1").is_err());
    // 歩が19枚
    assert!(Position::from_sfen("4k4/9/9/9/9/9/9/9/4K4 b 19P 1").is_err());
}

#[test]
fn startpos_has_30_moves() {
    let pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    let mut list = MoveList::default();
    generate_legal(&pos, true, &mut list);
    assert_eq!(list.len(), 30);
}

#[test]
fn perft_startpos_shallow() {
    let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    assert_eq!(perft(&mut pos, 1), 30);
    assert_eq!(perft(&mut pos, 2), 900);
    assert_eq!(perft(&mut pos, 3), 25_470);
}

#[test]
fn perft_matches_slow_perft() {
    let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    for d in 1..=3 {
        assert_eq!(perft(&mut pos, d), perft_slow(&mut pos, d));
    }
}

// releaseビルドでのみ実行（CIのreleaseテストが回す。ADR-0006/0018）
#[cfg_attr(debug_assertions, ignore)]
#[test]
fn perft_startpos_depth4() {
    let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    assert_eq!(perft(&mut pos, 4), 719_731);
}

#[cfg_attr(debug_assertions, ignore)]
#[test]
fn perft_startpos_depth5() {
    let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    assert_eq!(perft(&mut pos, 5), 19_861_490);
}

/// 最大分岐の既知局面（593手）。
#[test]
fn max_branching_position() {
    let pos = Position::from_sfen("R8/2K1S1SSk/4B4/9/9/9/9/9/1L1L1L3 b RBGSNLP3G3N17P 1").unwrap();
    let mut list = MoveList::default();
    generate_legal(&pos, true, &mut list);
    assert_eq!(list.len(), 593);
}

/// 打ち歩詰め: 5二への歩打ちは詰みなので非合法。
/// 金5三が歩と4二・6二を守り、龍2枚が4一・6一を抑える。
#[test]
fn uchifuzume_is_illegal() {
    let pos = Position::from_sfen("4k4/9/4G4/9/9/9/3+R1+R3/9/K8 b P 1").unwrap();
    let mut list = MoveList::default();
    generate_legal(&pos, true, &mut list);
    let drop = list.as_slice().iter().find(|m| m.to_usi() == "P*5b");
    assert!(drop.is_none(), "打ち歩詰めが生成された");

    // 金がなければ玉が歩を取れるので合法
    let pos2 = Position::from_sfen("4k4/9/9/9/9/9/3+R1+R3/9/K8 b P 1").unwrap();
    let mut list2 = MoveList::default();
    generate_legal(&pos2, true, &mut list2);
    let drop2 = list2.as_slice().iter().find(|m| m.to_usi() == "P*5b");
    assert!(drop2.is_some(), "詰みでない歩打ちが消えた");
}

fn random_playout(seed: u64, plies: usize, all: bool) {
    let mut rng = Rng(seed);
    let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    let mut history: Vec<(Move, String, u64)> = Vec::new();

    for _ in 0..plies {
        let mut list = MoveList::default();
        generate_legal(&pos, all, &mut list);
        if list.is_empty() {
            break;
        }

        // 差分更新キー＝SFEN経由の全計算キー
        let fresh = Position::from_sfen(&pos.to_sfen()).unwrap();
        assert_eq!(pos.key(), fresh.key(), "差分キーと全計算キーの不一致");
        assert_eq!(
            pos.state().material,
            fresh.state().material,
            "差分materialと全計算の不一致"
        );

        // gives_checkの整合
        let idx = (rng.next() % list.len() as u64) as usize;
        let m = list.as_slice()[idx];
        let predicted = pos.gives_check(m);

        history.push((m, pos.to_sfen(), pos.key()));
        pos.do_move(m);
        assert_eq!(
            pos.in_check(),
            predicted,
            "gives_checkの不一致: {}",
            m.to_usi()
        );
    }

    // 全部巻き戻して一致を確認
    while let Some((m, sfen, key)) = history.pop() {
        pos.undo_move(m);
        assert_eq!(pos.to_sfen(), sfen, "undo後のSFEN不一致");
        assert_eq!(pos.key(), key, "undo後のキー不一致");
    }
    assert_eq!(pos.to_sfen(), SFEN_STARTPOS);
}

#[test]
fn do_undo_roundtrip_random_games() {
    for seed in 1..=8u64 {
        random_playout(seed, 256, true);
        random_playout(seed.wrapping_mul(0x9E37_79B9), 256, false);
    }
}

/// Captures ∪ Quiets == NonEvasions（多重集合として一致）。
#[test]
fn captures_plus_quiets_equals_non_evasions() {
    let mut rng = Rng(0xC0FF_EE00_1234_5678);
    let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    for _ in 0..300 {
        let mut legal = MoveList::default();
        generate_legal(&pos, true, &mut legal);
        if legal.is_empty() {
            break;
        }
        if !pos.in_check() {
            for all in [false, true] {
                let mut ne = MoveList::default();
                let mut cap = MoveList::default();
                let mut qt = MoveList::default();
                generate(&pos, GenType::NonEvasions, all, &mut ne);
                generate(&pos, GenType::Captures, all, &mut cap);
                generate(&pos, GenType::Quiets, all, &mut qt);
                let mut a: Vec<String> = ne.as_slice().iter().map(|m| m.to_usi()).collect();
                let mut b: Vec<String> = cap
                    .as_slice()
                    .iter()
                    .chain(qt.as_slice())
                    .map(|m| m.to_usi())
                    .collect();
                a.sort();
                b.sort();
                assert_eq!(a, b, "分類の合成がNonEvasionsと不一致 (all={all})");
            }
        }
        let idx = (rng.next() % legal.len() as u64) as usize;
        pos.do_move(legal.as_slice()[idx]);
    }
}

/// move_from_usiが合法手を復元できる。
#[test]
fn move_from_usi_roundtrip() {
    let pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    let m = pos.move_from_usi("7g7f").unwrap();
    assert_eq!(m.to_usi(), "7g7f");
    assert!(pos.move_from_usi("7g7e").is_none());
}

/// 飛車の往復による通常の千日手（ADR-0026）。
#[test]
fn repetition_draw() {
    let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    assert_eq!(pos.repetition_state(), Repetition::None);
    apply(&mut pos, &["2h3h", "8b7b", "3h2h", "7b8b"]);
    assert_eq!(pos.repetition_state(), Repetition::Draw);
}

/// 連続王手の千日手。王手を掛け続けた側（先手）がLose。
#[test]
fn perpetual_check_is_loss_for_checker() {
    let mut pos = Position::from_sfen("4k4/9/9/9/9/9/9/5R3/4K4 b - 1").unwrap();
    apply(&mut pos, &["4h5h", "5a4a", "5h4h", "4a5a"]);
    // ここで盤面はループ先頭と同一。先手の手はすべて王手だった
    assert_eq!(pos.repetition_state(), Repetition::Lose);
}

/// SEE: 玉に守られた歩を香で取るのは損、守られていなければ得。
#[test]
fn see_defended_and_undefended() {
    let pos = Position::from_sfen("9/4k4/4p4/9/4L4/9/9/9/K8 b - 1").unwrap();
    let m = pos.move_from_usi("5e5c").unwrap();
    assert!(!pos.see_ge(m, 0), "守られた歩を取るのは損のはず");
    assert!(pos.see_ge(m, -300), "損は歩と香の差程度のはず");

    let pos2 = Position::from_sfen("5k3/9/4p4/9/4L4/9/9/9/K8 b - 1").unwrap();
    let m2 = pos2.move_from_usi("5e5c").unwrap();
    assert!(pos2.see_ge(m2, 0), "守られていない歩は取り得のはず");
}

/// pseudo_legalとto_moveの整合: 全合法手は復元・検査を通過する。
#[test]
fn pseudo_legal_accepts_generated_moves() {
    let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    let mut rng = Rng(0xABCD_EF01_2345_6789);
    for _ in 0..120 {
        let mut list = MoveList::default();
        generate_legal(&pos, true, &mut list);
        if list.is_empty() {
            break;
        }
        for &m in &list {
            assert!(
                pos.pseudo_legal(m),
                "合法手がpseudo_legalで弾かれた: {}",
                m.to_usi()
            );
            let restored = pos.to_move(m.to_move16()).unwrap();
            assert_eq!(restored, m, "Move16復元の不一致: {}", m.to_usi());
        }
        let idx = (rng.next() % list.len() as u64) as usize;
        pos.do_move(list.as_slice()[idx]);
    }
}
