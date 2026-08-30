//! 合法手生成（`generate_legal`）のバルクスループットベンチマーク。
//!
//! 局面の性格で生成コストが大きく変わるため、性格の違う6局面を測る。
//! SFENはすべてリポジトリ内の実在の局面から取った。
//!
//! - startpos: 平手初期局面（`SFEN_STARTPOS`）
//! - midgame_ply24: 序中盤、駒がぶつかり始めた24手目
//!   （`crates/tools/src/positions.rs` の検証局面）
//! - midgame_ply104: 中盤、駒が捌けて持ち駒が増えた104手目
//!   （`crates/core/tests/integration.rs` の桂不成テスト局面）
//! - matsuri: 指し手生成祭りの局面。双方の持ち駒が多く打ち手の生成が
//!   支配的で、生成器の定番ベンチ（やねうら王のUnitTestにも同じSFENが
//!   ある。`crates/core/src/effect.rs` の差分更新テスト局面と同一）
//! - max_moves: 合法手593手の最多手数局面
//!   （`crates/core/tests/integration.rs` のMoveList容量テスト局面）
//! - in_check: 王手がかかりEvasions経路を通る局面
//!   （`crates/core/tests/integration.rs` の入玉宣言テスト局面）
//!
//! 各局面は1回の生成時間（ns）を、bulkは6局面まとめての局面/秒
//! （elem/s）を読む。ローカルで
//! `RUSTFLAGS="-C target-cpu=native" cargo bench -p himawari-core --bench movegen`
//! を実行して計測する（ADR-0003）。

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use himawari_core::{MoveList, Position, SFEN_STARTPOS, generate_legal};

/// (ベンチ名, SFEN)。名前は結果の系列名になるので変えない。
const POSITIONS: [(&str, &str); 6] = [
    ("startpos", SFEN_STARTPOS),
    (
        "midgame_ply24",
        "+Bn1g2s1l/2skg2r1/ppppp1n1p/5bpp1/5p1P1/2P6/PP1PP1P1P/1SK2S1R1/LN1G1G1NL w Lp 24",
    ),
    (
        "midgame_ply104",
        "lr7/2g3k2/p2Ppp2B/4s1pPp/2Pnn4/PP1+b1P1p1/1S4P1N/6S2/L3KG2L w RGSNL3Pg2p 104",
    ),
    (
        "matsuri",
        "l6nl/5+P1gk/2np1S3/p1p4Pp/3P2Sp1/1PPb2P1P/P5GS1/R8/LN4bKL w GR5pnsg 1",
    ),
    (
        "max_moves",
        "R8/2K1S1SSk/4B4/9/9/9/9/9/1L1L1L3 b RBGSNLP3G3N17P 1",
    ),
    (
        "in_check",
        "K+R+P+P+P+P+P+P+P/g6+P+P/9/9/9/9/9/9/4k4 b RB3GS 1",
    ),
];

fn bench_movegen(c: &mut Criterion) {
    let positions: Vec<(&str, Position)> = POSITIONS
        .iter()
        .map(|&(name, sfen)| (name, Position::from_sfen(sfen).expect(name)))
        .collect();

    // 計測前に前提を確かめる。王手局面だけがEvasions経路を通り、
    // どの局面にも合法手がある
    for (name, pos) in &positions {
        assert_eq!(
            *name == "in_check",
            pos.in_check(),
            "王手の前提がずれた: {name}"
        );
        let mut list = MoveList::default();
        generate_legal(pos, true, &mut list);
        assert!(!list.is_empty(), "合法手が0手: {name}");
    }

    let mut g = c.benchmark_group("movegen");
    g.warm_up_time(std::time::Duration::from_millis(500));
    g.measurement_time(std::time::Duration::from_millis(1500));

    // 局面ごと: 1回の生成時間（ns）と局面/秒（elem/s）
    for (name, pos) in &positions {
        g.throughput(Throughput::Elements(1));
        g.bench_function(format!("legal/{name}"), |b| {
            b.iter(|| {
                let mut list = MoveList::default();
                generate_legal(black_box(pos), true, &mut list);
                list.len()
            });
        });
    }

    // まとめて: 6局面を順に生成し、局面/秒を読む
    g.throughput(Throughput::Elements(positions.len() as u64));
    g.bench_function("legal/bulk", |b| {
        b.iter(|| {
            let mut n = 0usize;
            for (_, pos) in black_box(&positions) {
                let mut list = MoveList::default();
                generate_legal(pos, true, &mut list);
                n += list.len();
            }
            n
        });
    });

    g.finish();
}

criterion_group!(benches, bench_movegen);
criterion_main!(benches);
