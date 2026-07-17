//! perft CLI（ADR-0018）。
//!
//! 使い方: perft [--slow] [--sfen "<sfen>"] <depth>
//! 既定はbulk counting。--slowで葉まで潜る素直な実装に切り替える。

use std::time::Instant;

use himawari_core::{Position, SFEN_STARTPOS, perft, perft_slow};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut slow = false;
    let mut sfen = SFEN_STARTPOS.to_string();
    let mut depth = 5u32;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--slow" => slow = true,
            "--sfen" => {
                i += 1;
                sfen = args.get(i).cloned().unwrap_or_else(|| {
                    eprintln!("--sfenの後にSFEN文字列が必要です");
                    std::process::exit(1);
                });
            }
            s => {
                depth = s.parse().unwrap_or_else(|_| {
                    eprintln!("深さが数値ではありません: {s}");
                    std::process::exit(1);
                });
            }
        }
        i += 1;
    }

    let mut pos = match Position::from_sfen(&sfen) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("SFENが不正です: {}", e.0);
            std::process::exit(1);
        }
    };

    println!("sfen: {sfen}");
    println!("mode: {}", if slow { "slow" } else { "bulk" });
    for d in 1..=depth {
        let start = Instant::now();
        let nodes = if slow {
            perft_slow(&mut pos, d)
        } else {
            perft(&mut pos, d)
        };
        let sec = start.elapsed().as_secs_f64();
        let nps = if sec > 0.0 { nodes as f64 / sec } else { 0.0 };
        println!("depth {d}: {nodes} nodes, {sec:.3}s, {:.0} nps", nps);
    }
}
