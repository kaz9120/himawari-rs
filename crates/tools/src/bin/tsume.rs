//! 詰将棋一括実行ツール（P2出口条件の検証用）。
//!
//! 使い方: tsume [--depth D] [--file path]
//! ファイル形式: 1行1問 `<SFEN>,<詰み手数>`。#始まりはコメント。
//! ファイル指定がなければ組み込みのスモークセットを実行する。
//!
//! 正答条件: 期待手数以下の詰みを発見すること。

use std::sync::Arc;
use std::time::Instant;

use himawari_core::Position;
use himawari_engine::eval::Evaluator;
use himawari_engine::movepick::{CorrectionHistory, CounterMoves, History};
use himawari_engine::search::{Shared, Worker};
use himawari_engine::timeman::{Limits, TimeManager};
use himawari_engine::value::VALUE_MATE;

/// 手で検証済みのスモークセット（SFEN, 詰み手数）。
const BUILTIN: &[(&str, u32)] = &[
    // 1手詰め: 桂に支えられた金打ち
    ("4k4/9/9/5N3/9/9/9/9/4K4 b G 1", 1),
    ("3k5/9/9/4N4/9/9/9/9/4K4 b G 1", 1),
    // 3手詰め: 飛車の歩取り王手（成って龍）→ 金打ちまで
    ("4k4/9/4p4/9/9/9/9/4R4/1K7 b G 1", 3),
    ("5k3/9/5p3/9/9/9/9/5R3/1K7 b G 1", 3),
];

fn solve(sfen: &str, depth: u32, shared: &Arc<Shared>) -> Option<(u32, String)> {
    let pos = Position::from_sfen(sfen).ok()?;
    let limits = Limits {
        depth,
        ..Limits::default()
    };
    let tm = TimeManager::new(&limits, pos.side_to_move(), pos.game_ply(), 0, 0);
    let mut worker = Worker::new(
        pos,
        Arc::clone(shared),
        limits,
        tm,
        0,
        1,
        Evaluator::material(),
        History::default(),
        CounterMoves::default(),
        CorrectionHistory::default(),
    );
    let result = worker.iterate(&mut |_| {});
    if result.score > VALUE_MATE - 256 {
        let plies = (VALUE_MATE - result.score) as u32;
        Some((plies, result.best.to_usi()))
    } else {
        None
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut depth = 9u32;
    let mut file: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--depth" => {
                i += 1;
                depth = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(9);
            }
            "--file" => {
                i += 1;
                file = args.get(i).cloned();
            }
            other => {
                eprintln!("不明な引数: {other}");
                std::process::exit(1);
            }
        }
        i += 1;
    }

    let problems: Vec<(String, u32)> = match &file {
        Some(path) => {
            let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
                eprintln!("ファイルを読めません: {e}");
                std::process::exit(1);
            });
            text.lines()
                .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
                .filter_map(|l| {
                    let (sfen, n) = l.rsplit_once(',')?;
                    Some((sfen.trim().to_string(), n.trim().parse().ok()?))
                })
                .collect()
        }
        None => BUILTIN.iter().map(|&(s, n)| (s.to_string(), n)).collect(),
    };

    if problems.is_empty() {
        eprintln!("問題がありません");
        std::process::exit(1);
    }

    let shared = Arc::new(Shared::new(64));
    let mut solved = 0usize;
    let start = Instant::now();
    for (i, (sfen, expected)) in problems.iter().enumerate() {
        let t = Instant::now();
        let result = solve(sfen, depth, &shared);
        let ms = t.elapsed().as_millis();
        match result {
            Some((plies, mv)) if plies <= *expected => {
                solved += 1;
                println!(
                    "#{:<3} OK   mate {plies} ({expected}手詰) {mv} {ms}ms",
                    i + 1
                );
            }
            Some((plies, mv)) => {
                println!(
                    "#{:<3} LONG mate {plies} ({expected}手詰) {mv} {ms}ms",
                    i + 1
                );
            }
            None => {
                println!("#{:<3} FAIL 詰み発見できず ({expected}手詰) {ms}ms", i + 1);
            }
        }
    }
    let total = problems.len();
    println!(
        "----\n正答 {solved}/{total} ({:.1}%) depth {depth} {:.1}s",
        solved as f64 * 100.0 / total as f64,
        start.elapsed().as_secs_f64()
    );
    if solved < total {
        std::process::exit(2);
    }
}
