//! 配列盤の試作を、既存のcore（Bitboard）と突き合わせる（ADR-0241の段階3）。
//!
//! - SFENの往復がcoreの出力と一致する
//! - perft木の全ノードで、合法手の集合がcoreと一致し、利きの配列が全計算と一致する
//! - ランダムな対局の全手で同じ検査をする（終盤・駒打ち・王手の多い局面まで届かせる）
//! - perftの数がcoreと一致する（深い方は `--ignored` で走らせる）

use himawari_core::{MoveList as CoreList, Position as CorePos, generate_legal};
use himawari_mailbox::{MoveList, Position, SFEN_STARTPOS, perft};

/// coreの統合テスト（generate_legal_matches_filtered_pseudo）と同じ6局面。
const CASES: [&str; 6] = [
    SFEN_STARTPOS,
    "+Bn1g2s1l/2skg2r1/ppppp1n1p/5bpp1/5p1P1/2P6/PP1PP1P1P/1SK2S1R1/LN1G1G1NL w Lp 24",
    "lr7/2g3k2/p2Ppp2B/4s1pPp/2Pnn4/PP1+b1P1p1/1S4P1N/6S2/L3KG2L w RGSNL3Pg2p 104",
    "l6nl/5+P1gk/2np1S3/p1p4Pp/3P2Sp1/1PPb2P1P/P5GS1/R8/LN4bKL w GR5pnsg 1",
    "R8/2K1S1SSk/4B4/9/9/9/9/9/1L1L1L3 b RBGSNLP3G3N17P 1",
    "K+R+P+P+P+P+P+P+P/g6+P+P/9/9/9/9/9/9/4k4 b RB3GS 1",
];

/// 王手・ピン・打ち歩詰めの周りを狙った局面。
const EXTRA: [&str; 6] = [
    // 打ち歩詰め（1一玉。1二の歩を2三の銀が支え、2一・2二は3二の金が押さえる）
    "8k/6G2/7S1/9/9/9/9/9/K8 b P 1",
    // 歩を打つと王手だが、玉が取れるので打ち歩詰めではない
    "8k/6G2/9/9/9/9/9/9/K8 b P 1",
    // 両王手の逃げ（5三の飛と7三の角）
    "4k4/9/2B1R4/9/9/9/9/9/K8 w - 1",
    // ピンされた駒（5筋の金）と飛び利きの延長
    "4k4/4g4/9/9/4R4/9/9/9/K8 w - 1",
    // 合駒の効く王手（駒打ちの合駒）
    "4k4/9/9/9/9/9/9/9/K3R4 w gsnlp 1",
    // 玉の背後の利き（玉が飛の線に沿って逃げられない）
    "9/9/9/4k4/9/9/9/4R4/K8 w - 1",
];

fn core_moves(pos: &CorePos) -> Vec<String> {
    let mut list = CoreList::default();
    generate_legal(pos, true, &mut list);
    let mut v: Vec<String> = list.as_slice().iter().map(|m| m.to_usi()).collect();
    v.sort();
    v
}

fn mb_moves(pos: &mut Position) -> Vec<String> {
    let mut list = MoveList::default();
    pos.generate_legal(&mut list);
    let mut v: Vec<String> = list.as_slice().iter().map(|m| m.to_usi()).collect();
    v.sort();
    let n = v.len();
    v.dedup();
    assert_eq!(n, v.len(), "同じ手を2回出した: {}", pos.to_sfen());
    v
}

/// 1ノードの検査。SFEN・合法手の集合・利きの配列。
fn check_node(core: &CorePos, mb: &mut Position) {
    assert_eq!(mb.to_sfen(), core.to_sfen());
    if let Err(e) = mb.check_consistency() {
        panic!("{e}: {}", mb.to_sfen());
    }
    assert_eq!(mb.in_check(), core.in_check(), "{}", core.to_sfen());
    let a = core_moves(core);
    let b = mb_moves(mb);
    if a != b {
        let missing: Vec<_> = a.iter().filter(|m| !b.contains(m)).collect();
        let extra: Vec<_> = b.iter().filter(|m| !a.contains(m)).collect();
        panic!(
            "合法手の不一致 {}\n  足りない: {missing:?}\n  余分: {extra:?}",
            core.to_sfen()
        );
    }
}

fn find(mb: &mut Position, usi: &str) -> himawari_mailbox::Move {
    let mut list = MoveList::default();
    mb.generate_legal(&mut list);
    *list
        .as_slice()
        .iter()
        .find(|m| m.to_usi() == usi)
        .unwrap_or_else(|| panic!("手が無い: {usi}"))
}

fn walk(core: &mut CorePos, mb: &mut Position, depth: u32) {
    check_node(core, mb);
    if depth == 0 {
        return;
    }
    for usi in core_moves(core) {
        let cm = core.move_from_usi(&usi).unwrap();
        let mm = find(mb, &usi);
        let before = mb.to_sfen();
        core.do_move(cm);
        mb.do_move(mm);
        walk(core, mb, depth - 1);
        core.undo_move(cm);
        mb.undo_move(mm);
        assert_eq!(mb.to_sfen(), before, "undoで戻らない: {usi}");
        mb.check_consistency().unwrap();
    }
}

#[test]
fn sfen_roundtrip_matches_core() {
    for sfen in CASES.iter().chain(EXTRA.iter()) {
        let core = CorePos::from_sfen(sfen).unwrap();
        let mb = Position::from_sfen(sfen).unwrap();
        assert_eq!(mb.to_sfen(), core.to_sfen());
    }
}

#[test]
fn every_node_matches_core_shallow() {
    let depths = [3, 2, 2, 2, 1, 2];
    for (sfen, d) in CASES.iter().zip(depths) {
        let mut core = CorePos::from_sfen(sfen).unwrap();
        let mut mb = Position::from_sfen(sfen).unwrap();
        walk(&mut core, &mut mb, d);
    }
    for sfen in EXTRA {
        let mut core = CorePos::from_sfen(sfen).unwrap();
        let mut mb = Position::from_sfen(sfen).unwrap();
        walk(&mut core, &mut mb, 3);
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

/// ランダムに指し進め、全手で突き合わせる。終局（合法手なし）か上限の手数で止める。
/// 届いた局面の種類を数え、検査が王手・駒打ち・詰みまで届いたことを確かめる。
fn playouts(games: u32, max_ply: u32, seed: u64) {
    let mut rng = Rng(seed);
    let (mut nodes, mut checks, mut drops, mut mates) = (0u64, 0u64, 0u64, 0u64);
    for g in 0..games {
        let start = CASES[g as usize % CASES.len()];
        let mut core = CorePos::from_sfen(start).unwrap();
        let mut mb = Position::from_sfen(start).unwrap();
        let mut history = Vec::new();
        for _ in 0..max_ply {
            check_node(&core, &mut mb);
            nodes += 1;
            checks += u64::from(core.in_check());
            let moves = core_moves(&core);
            if moves.is_empty() {
                mates += 1;
                break;
            }
            drops += moves.iter().filter(|m| m.contains('*')).count() as u64;
            let usi = moves[(rng.next() % moves.len() as u64) as usize].clone();
            let cm = core.move_from_usi(&usi).unwrap();
            let mm = find(&mut mb, &usi);
            core.do_move(cm);
            mb.do_move(mm);
            history.push((cm, mm));
        }
        while let Some((cm, mm)) = history.pop() {
            core.undo_move(cm);
            mb.undo_move(mm);
            assert_eq!(mb.to_sfen(), core.to_sfen());
        }
        mb.check_consistency().unwrap();
    }
    println!("局面 {nodes}、王手 {checks}、駒打ちの合法手 {drops}、詰み {mates}");
    assert!(checks > 0 && drops > 0 && mates > 0);
}

#[test]
fn random_playouts_match_core() {
    playouts(60, 300, 0x9E37_79B9_7F4A_7C15);
}

#[test]
#[ignore = "重い。cargo test --release -- --ignored で走らせる"]
fn random_playouts_match_core_long() {
    playouts(3000, 400, 0x2545_F491_4F6C_DD1D);
}

fn perft_vs_core(sfen: &str, depth: u32) {
    let mut core = CorePos::from_sfen(sfen).unwrap();
    let mut mb = Position::from_sfen(sfen).unwrap();
    let a = himawari_core::perft(&mut core, depth);
    let b = perft(&mut mb, depth);
    println!("perft {depth} = {b} (core {a}): {sfen}");
    assert_eq!(a, b, "perftの不一致 depth={depth}: {sfen}");
}

#[test]
fn perft_matches_core_shallow() {
    for sfen in CASES.iter().chain(EXTRA.iter()) {
        perft_vs_core(sfen, 3);
    }
}

#[test]
#[ignore = "重い。cargo test --release -- --ignored で走らせる"]
fn perft_matches_core_deep() {
    let depths = [5, 4, 4, 4, 3, 4];
    for (sfen, d) in CASES.iter().zip(depths) {
        perft_vs_core(sfen, d);
    }
    for sfen in EXTRA {
        perft_vs_core(sfen, 5);
    }
}

/// 打ち歩詰めの規則。詰む歩打ちは出さず、詰まない歩打ちは出す。
#[test]
fn pawn_drop_mate_rule() {
    let mut mate = Position::from_sfen(EXTRA[0]).unwrap();
    assert!(!mb_moves(&mut mate).contains(&"P*1b".to_string()));
    assert!(mb_moves(&mut mate).contains(&"P*1c".to_string()));
    let mut not_mate = Position::from_sfen(EXTRA[1]).unwrap();
    assert!(mb_moves(&mut not_mate).contains(&"P*1b".to_string()));
}

#[test]
fn startpos_perft_known_values() {
    let mut mb = Position::from_sfen(SFEN_STARTPOS).unwrap();
    assert_eq!(perft(&mut mb, 4), 719_731);
}
