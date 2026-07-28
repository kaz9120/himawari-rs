//! 定跡生成ツール（ADR-0063）。
//!
//! 平手初期局面から幅優先で展開し、各局面をMultiPV=widthで探索して
//! 上位width手を記録する。出力はやねうら王db形式互換。
//!
//! 使い方:
//!   book gen --out <path> [--eval <hmwr>] [--ply 8] [--width 2]
//!            [--depth 24] [--hash 256] [--threads N]
//!
//! --ply は展開する手数、--width は各局面で記録する候補手数。
//! 局面数は width^0 + ... + width^ply になる（width=2, ply=8 で511）。
//! 先手番・後手番の両方を含める。相手の手を経由しないと自分の手番の
//! 局面に到達できないため。
//!
//! 探索はThreadPool経由でLazy SMP（ADR-0031）を使う。置換表は局面を
//! またいで再利用する。BFSで親から子へ展開するため、親の探索で読んだ
//! 子局面の情報がそのまま効く。TTのエントリは深さ付きなので、浅い探索
//! の結果が深い探索に流用されることはない。

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::sync::{Arc, Mutex};

use himawari_core::{Position, SFEN_STARTPOS};
use himawari_engine::{EngineOptions, Limits, ThreadPool};

struct Config {
    out: String,
    eval: String,
    ply: u16,
    width: usize,
    depth: u32,
    hash_mb: usize,
    threads: usize,
}

/// 手数を除いたsfen（盤面・手番・手駒）をキーにする。
fn book_key(pos: &Position) -> String {
    let sfen = pos.to_sfen();
    match sfen.rfind(' ') {
        Some(i) => sfen[..i].to_string(),
        None => sfen,
    }
}

/// info行から (depth, multipv, 評価値, pvの指し手列) を取り出す。
/// mateスコアは詰み手数から評価値に直す（定跡には数値で入れる）。
fn parse_info(line: &str) -> Option<(u32, usize, i32, Vec<String>)> {
    let t: Vec<&str> = line.split_whitespace().collect();
    let at = |k: &str| t.iter().position(|&x| x == k);
    let depth: u32 = t.get(at("depth")? + 1)?.parse().ok()?;
    let multipv = at("multipv")
        .and_then(|i| t.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(1usize);
    let si = at("score")?;
    let score = match *t.get(si + 1)? {
        "cp" => t.get(si + 2)?.parse().ok()?,
        "mate" => {
            let plies: i32 = t.get(si + 2)?.parse().ok()?;
            if plies >= 0 {
                30000 - plies
            } else {
                -30000 - plies
            }
        }
        _ => return None,
    };
    let pv = t[at("pv")? + 1..].iter().map(|s| s.to_string()).collect();
    Some((depth, multipv, score, pv))
}

/// 1局面をMultiPV=widthで探索し、上位width手を (指し手, 評価値, 予想応手) で返す。
fn search_lines(
    pos: &Position,
    cfg: &Config,
    pool: &ThreadPool,
    sink: &Sink,
) -> Vec<(String, i32, String)> {
    sink.lock().expect("sink").clear();
    let limits = Limits {
        depth: cfg.depth,
        ..Limits::default()
    };
    let opts = EngineOptions {
        multi_pv: cfg.width,
        threads: cfg.threads,
        hash_mb: cfg.hash_mb,
        ..EngineOptions::default()
    };
    pool.go(pos.clone(), limits, opts);
    pool.wait_idle();

    // 各ラインについて最終深さの結果を採る
    // (depth, score, pv)
    type BestEntry = (u32, i32, Vec<String>);
    let mut best: HashMap<usize, BestEntry> = HashMap::new();
    for line in sink.lock().expect("sink").iter() {
        let Some((depth, multipv, score, pv)) = parse_info(line) else {
            continue;
        };
        if pv.is_empty() {
            continue;
        }
        let e = best.entry(multipv).or_insert((0, 0, Vec::new()));
        if depth >= e.0 {
            *e = (depth, score, pv);
        }
    }
    let mut lines: Vec<(usize, BestEntry)> = best.into_iter().collect();
    lines.sort_by_key(|(k, _)| *k);
    lines
        .into_iter()
        .filter_map(|(_, (_, score, pv))| {
            let mv = pv.first()?.clone();
            let ponder = pv.get(1).cloned().unwrap_or_else(|| "none".to_string());
            Some((mv, score, ponder))
        })
        .collect()
}

type Sink = Arc<Mutex<Vec<String>>>;

fn generate(cfg: &Config) -> std::io::Result<()> {
    // 生成条件をログの先頭に残す（ADR-0082）。定跡は非決定的に生成され、
    // 評価関数にも依存するため、どの設定で作ったかを後から追えないと
    // 作り直しの判断ができない
    eprintln!(
        "BookGen: ply={} width={} depth={} threads={} hash={}MB",
        cfg.ply, cfg.width, cfg.depth, cfg.threads, cfg.hash_mb
    );
    let eval = if cfg.eval.is_empty() {
        None
    } else {
        let mut f = std::fs::File::open(&cfg.eval)?;
        let (net, lineage) = himawari_engine::nnue_io::load(&mut f)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        eprintln!("EvalFile: {} ({lineage})", cfg.eval);
        Some((cfg.eval.clone(), Arc::new(net)))
    };
    let sink: Sink = Arc::new(Mutex::new(Vec::new()));
    let on_line = {
        let s = Arc::clone(&sink);
        Arc::new(move |line: &str| {
            if line.starts_with("info depth") {
                s.lock().expect("sink").push(line.to_string());
            }
        })
    };
    let pool = ThreadPool::new(cfg.hash_mb, cfg.threads, eval, on_line);

    let root = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
    let mut queue: VecDeque<(Position, u16)> = VecDeque::new();
    queue.push_back((root, 0));
    // キーごとの候補手。挿入順を保つため別途キー列を持つ
    let mut out: HashMap<String, Vec<(String, i32, String)>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let started = std::time::Instant::now();

    while let Some((pos, ply)) = queue.pop_front() {
        if ply >= cfg.ply {
            continue;
        }
        let key = book_key(&pos);
        if out.contains_key(&key) {
            continue;
        }
        let lines = search_lines(&pos, cfg, &pool, &sink);
        if lines.is_empty() {
            continue;
        }
        eprintln!(
            "[{:>4}局面 {:>5}s] ply={} 手={} 評価={}",
            order.len() + 1,
            started.elapsed().as_secs(),
            ply,
            lines[0].0,
            lines[0].1
        );
        for (mv, _, _) in &lines {
            let Some(m) = pos.move_from_usi(mv) else {
                continue;
            };
            let mut next = pos.clone();
            next.do_move(m);
            queue.push_back((next, ply + 1));
        }
        out.insert(key.clone(), lines);
        order.push(key);
    }
    pool.quit();

    let mut f = std::fs::File::create(&cfg.out)?;
    writeln!(f, "#YANEURAOU-DB2016 1.00")?;
    for key in &order {
        writeln!(f, "sfen {key} 1")?;
        for (mv, score, ponder) in &out[key] {
            writeln!(f, "{mv} {ponder} {score} {} 1", cfg.depth)?;
        }
    }
    eprintln!(
        "{} に {}局面を書き出しました（{}秒）",
        cfg.out,
        order.len(),
        started.elapsed().as_secs()
    );
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
        threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
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
            "--threads" => cfg.threads = val.parse::<usize>().unwrap_or(cfg.threads).max(1),
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
