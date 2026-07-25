//! 定跡生成ツール（ADR-0063）。
//!
//! 平手初期局面から幅優先で展開し、各局面をMultiPV=widthで探索して
//! 上位width手を記録する。出力はやねうら王db形式互換。
//!
//! 使い方:
//!   book gen --out <path> [--eval <hmwr>] [--ply 8] [--width 2]
//!            [--depth 24] [--hash 256]
//!
//! --ply は展開する手数、--width は各局面で記録する候補手数。
//! 局面数は width^0 + ... + width^ply になる（width=2, ply=8 で511）。
//! 先手番・後手番の両方を含める。相手の手を経由しないと自分の手番の
//! 局面に到達できないため。

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::sync::Arc;

use himawari_core::{Move, Position, SFEN_STARTPOS};
use himawari_engine::eval::Evaluator;
use himawari_engine::movepick::{ContinuationHistory, CorrectionHistory, CounterMoves, History};
use himawari_engine::search::{Shared, Worker};
use himawari_engine::timeman::{Limits, TimeManager};

struct Config {
    out: String,
    eval: String,
    ply: u16,
    width: usize,
    depth: u32,
    hash_mb: usize,
}

/// 手数を除いたsfen（盤面・手番・手駒）をキーにする。
fn book_key(pos: &Position) -> String {
    let sfen = pos.to_sfen();
    match sfen.rfind(' ') {
        Some(i) => sfen[..i].to_string(),
        None => sfen,
    }
}

/// 1局面をMultiPV=widthで探索し、上位width手を (指し手, 評価値, 予想応手) で返す。
fn search_lines(
    pos: &Position,
    cfg: &Config,
    shared: &Arc<Shared>,
    eval: Option<&Arc<himawari_engine::nnue::NnueNetwork>>,
) -> Vec<(Move, i32, String)> {
    let limits = Limits {
        depth: cfg.depth,
        ..Limits::default()
    };
    let tm = TimeManager::new(&limits, pos.side_to_move(), pos.game_ply(), 0, 0);
    let evaluator = match eval {
        Some(n) => Evaluator::nnue(Arc::clone(n)),
        None => Evaluator::material(),
    };
    let mut worker = Worker::new(
        pos.clone(),
        Arc::clone(shared),
        limits,
        tm,
        0,
        cfg.width,
        evaluator,
        History::default(),
        CounterMoves::default(),
        CorrectionHistory::default(),
        ContinuationHistory::default(),
    );
    // 最終深さの各ラインを拾う。MultiPV>1ならmultipvは1始まり
    let mut best: HashMap<usize, (u32, i32, Vec<Move>)> = HashMap::new();
    worker.iterate(&mut |info| {
        let line = info.multipv.max(1);
        let e = best.entry(line).or_insert((0, 0, Vec::new()));
        if info.depth >= e.0 {
            *e = (info.depth, info.score, info.pv.clone());
        }
    });
    let mut lines: Vec<(usize, (u32, i32, Vec<Move>))> = best.into_iter().collect();
    lines.sort_by_key(|(k, _)| *k);
    lines
        .into_iter()
        .filter_map(|(_, (_, score, pv))| {
            let mv = *pv.first()?;
            let ponder = pv.get(1).map_or("none".to_string(), |m| m.to_usi());
            Some((mv, score, ponder))
        })
        .collect()
}

fn generate(cfg: &Config) -> std::io::Result<()> {
    let eval = if cfg.eval.is_empty() {
        None
    } else {
        let mut f = std::fs::File::open(&cfg.eval)?;
        let (net, lineage) = himawari_engine::nnue_io::load(&mut f)
            .map_err(|e| std::io::Error::other(format!("{e}")))?;
        eprintln!("EvalFile: {} ({lineage})", cfg.eval);
        Some(Arc::new(net))
    };
    let shared = Arc::new(Shared::new(cfg.hash_mb));

    let root = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
    let mut queue: VecDeque<(Position, u16)> = VecDeque::new();
    queue.push_back((root, 0));
    // キーごとの候補手。挿入順を保つため別途キー列を持つ
    let mut out: HashMap<String, Vec<(Move, i32, String)>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    while let Some((pos, ply)) = queue.pop_front() {
        if ply >= cfg.ply {
            continue;
        }
        let key = book_key(&pos);
        if out.contains_key(&key) {
            continue;
        }
        let lines = search_lines(&pos, cfg, &shared, eval.as_ref());
        if lines.is_empty() {
            continue;
        }
        eprintln!(
            "[{:>4}局面] ply={} 手={} 評価={}",
            order.len() + 1,
            ply,
            lines[0].0.to_usi(),
            lines[0].1
        );
        for (mv, _, _) in &lines {
            let mut next = pos.clone();
            next.do_move(*mv);
            queue.push_back((next, ply + 1));
        }
        out.insert(key.clone(), lines);
        order.push(key);
    }

    let mut f = std::fs::File::create(&cfg.out)?;
    writeln!(f, "#YANEURAOU-DB2016 1.00")?;
    for key in &order {
        writeln!(f, "sfen {key} 1")?;
        for (mv, score, ponder) in &out[key] {
            writeln!(f, "{} {ponder} {score} {} 1", mv.to_usi(), cfg.depth)?;
        }
    }
    eprintln!("{} に {}局面を書き出しました", cfg.out, order.len());
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("gen") {
        eprintln!("使い方は crates/tools/src/bin/book.rs 冒頭のコメントを参照");
        std::process::exit(3);
    }
    let mut cfg = Config {
        out: "data/book/mini.db".to_string(),
        eval: String::new(),
        ply: 8,
        width: 2,
        depth: 24,
        hash_mb: 256,
    };
    let mut i = 1;
    while i < args.len() {
        let val = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--out" => cfg.out = val,
            "--eval" => cfg.eval = val,
            "--ply" => cfg.ply = val.parse().unwrap_or(cfg.ply),
            "--width" => cfg.width = val.parse::<usize>().unwrap_or(cfg.width).max(1),
            "--depth" => cfg.depth = val.parse().unwrap_or(cfg.depth),
            "--hash" => cfg.hash_mb = val.parse().unwrap_or(cfg.hash_mb),
            other => {
                eprintln!("不明な引数: {other}");
                std::process::exit(3);
            }
        }
        i += 2;
    }
    if let Some(dir) = std::path::Path::new(&cfg.out).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = generate(&cfg) {
        eprintln!("エラー: {e}");
        std::process::exit(3);
    }
}
