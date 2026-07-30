//! `mate_1ply` 単体のスループット計測。
//!
//! ランダムプレイアウトで王手のかかっていない局面を集め、1局面あたりの
//! 平均コストを測る。詰みがある局面は早期リターンで軽くなるため、
//! 「詰みなし」の局面だけを集めた集合も別に測る。
//!
//! ローカルで `RUSTFLAGS="-C target-cpu=native" cargo bench -p himawari-engine
//! --bench mate1ply` を実行して計測する（ADR-0003）。

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use himawari_core::{MoveList, Position, SFEN_STARTPOS, generate_legal};
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

/// ランダムプレイアウトで王手のかかっていない局面のsfenを集める。
fn collect_positions(games: u64, plies: usize) -> Vec<String> {
    let mut out = Vec::new();
    for seed in 1..=games {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
        for _ in 0..plies {
            let mut list = MoveList::default();
            generate_legal(&pos, true, &mut list);
            if list.is_empty() {
                break;
            }
            if !pos.in_check() {
                out.push(pos.to_sfen());
            }
            let m = list.as_slice()[(rng.next() % list.len() as u64) as usize];
            pos.do_move(m);
        }
    }
    out
}

fn bench(c: &mut Criterion) {
    let sfens = collect_positions(60, 90);
    let mut all: Vec<Position> = sfens
        .iter()
        .map(|s| Position::from_sfen(s).unwrap())
        .collect();
    // 詰みのない局面だけの集合。オラクルで振り分ける
    let mut no_mate: Vec<Position> = sfens
        .iter()
        .map(|s| Position::from_sfen(s).unwrap())
        .filter(|p| mate_1ply_oracle(&mut p.clone()).is_none())
        .collect();
    println!(
        "mate1ply bench: all={} positions, no_mate={} positions",
        all.len(),
        no_mate.len()
    );

    let mut g = c.benchmark_group("mate_1ply");
    g.throughput(criterion::Throughput::Elements(all.len() as u64));
    g.bench_function("all", |b| {
        b.iter(|| {
            for p in all.iter_mut() {
                black_box(mate_1ply(black_box(p)));
            }
        });
    });
    g.throughput(criterion::Throughput::Elements(no_mate.len() as u64));
    g.bench_function("no_mate", |b| {
        b.iter(|| {
            for p in no_mate.iter_mut() {
                black_box(mate_1ply(black_box(p)));
            }
        });
    });
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
