//! USIエンジンのエントリポイント（ADR-0019）。
//!
//! stdin読み取りスレッド＋コマンドループ＋探索スレッド分離。
//! 出力は行単位でロックしてflushする。

use std::io::Write;
use std::sync::{Arc, mpsc};

use himawari_core::{Position, SFEN_STARTPOS};
use himawari_engine::nnue::NnueNetwork;
use himawari_engine::{EngineOptions, Limits, ThreadPool};

const ENGINE_NAME: &str = "Himawari";
const ENGINE_AUTHOR: &str = "Kazumasa Yamamoto";

fn version_string() -> String {
    // devビルドの識別はCIが設定するHIMAWARI_BUILD_ID（ADR-0007）
    match option_env!("HIMAWARI_BUILD_ID") {
        Some(id) => format!("{} ({})", env!("CARGO_PKG_VERSION"), id),
        None => format!("{}-dev", env!("CARGO_PKG_VERSION")),
    }
}

fn print_line(s: &str) {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = writeln!(lock, "{s}");
    let _ = lock.flush();
}

/// setoptionで設定できるオプションのUSI宣言（ADR-0019のレジストリ）。
fn print_options() {
    print_line("option name USI_Hash type spin default 256 min 1 max 33554432");
    print_line("option name USI_Ponder type check default false");
    print_line("option name Threads type spin default 1 min 1 max 512");
    print_line("option name NetworkDelay type spin default 120 min 0 max 10000");
    print_line("option name NetworkDelay2 type spin default 1120 min 0 max 10000");
    print_line("option name MaxMovesToDraw type spin default 0 min 0 max 100000");
    print_line("option name MultiPV type spin default 1 min 1 max 128");
    print_line("option name EvalFile type string default <empty>");
}

fn parse_position(tokens: &[&str]) -> Option<Position> {
    let mut i = 0;
    let mut pos = if tokens.first() == Some(&"startpos") {
        i += 1;
        Position::from_sfen(SFEN_STARTPOS).ok()?
    } else if tokens.first() == Some(&"sfen") {
        // sfenは4トークン（盤面・手番・手駒・手数）
        if tokens.len() < 5 {
            return None;
        }
        let sfen = tokens[1..5].join(" ");
        i += 5;
        Position::from_sfen(&sfen).ok()?
    } else {
        return None;
    };
    if tokens.get(i) == Some(&"moves") {
        for s in &tokens[i + 1..] {
            let m = pos.move_from_usi(s)?;
            pos.do_move(m);
        }
    }
    Some(pos)
}

fn parse_go(tokens: &[&str]) -> Limits {
    let mut limits = Limits::default();
    let mut i = 0;
    while i < tokens.len() {
        let value = |j: usize| tokens.get(j).and_then(|s| s.parse::<u64>().ok());
        match tokens[i] {
            "btime" => limits.btime = value(i + 1).unwrap_or(0),
            "wtime" => limits.wtime = value(i + 1).unwrap_or(0),
            "byoyomi" => limits.byoyomi = value(i + 1).unwrap_or(0),
            "binc" => limits.binc = value(i + 1).unwrap_or(0),
            "winc" => limits.winc = value(i + 1).unwrap_or(0),
            "movetime" => limits.movetime = value(i + 1).unwrap_or(0),
            "depth" => limits.depth = value(i + 1).unwrap_or(0) as u32,
            "nodes" => limits.nodes = value(i + 1).unwrap_or(0),
            "infinite" => {
                limits.infinite = true;
                i += 1;
                continue;
            }
            "ponder" => {
                // ponderフラグは呼び出し側がトークンで判定する（ADR-0033）
                i += 1;
                continue;
            }
            _ => {
                i += 1;
                continue;
            }
        }
        i += 2;
    }
    limits
}

fn set_option(opts: &mut EngineOptions, tokens: &[&str]) {
    // setoption name <id> value <x>
    let name_idx = tokens.iter().position(|&t| t == "name");
    let value_idx = tokens.iter().position(|&t| t == "value");
    let (Some(ni), Some(vi)) = (name_idx, value_idx) else {
        return;
    };
    let name = tokens[ni + 1..vi].join(" ");
    let value = tokens[vi + 1..].join(" ");
    match name.as_str() {
        "USI_Hash" => {
            if let Ok(v) = value.parse() {
                opts.hash_mb = v;
            }
        }
        "Threads" => {
            if let Ok(v) = value.parse() {
                opts.threads = v;
            }
        }
        "NetworkDelay" => {
            if let Ok(v) = value.parse() {
                opts.network_delay = v;
            }
        }
        "NetworkDelay2" => {
            if let Ok(v) = value.parse() {
                opts.network_delay2 = v;
            }
        }
        "MaxMovesToDraw" => {
            if let Ok(v) = value.parse() {
                opts.max_moves_to_draw = v;
            }
        }
        "MultiPV" => {
            if let Ok(v) = value.parse::<usize>() {
                opts.multi_pv = v.max(1);
            }
        }
        "USI_Ponder" => {
            opts.ponder = value == "true";
        }
        "EvalFile" => {
            opts.eval_file = if value == "<empty>" {
                String::new()
            } else {
                value
            };
        }
        _ => {}
    }
}

/// EvalFileを読み込む。失敗は起動エラー（ADR-0037: 駒割への
/// フォールバックはしない。気づかず弱いまま対局する事故を防ぐ）。
fn load_eval(path: &str) -> Option<(String, std::sync::Arc<NnueNetwork>)> {
    if path.is_empty() {
        return None;
    }
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            print_line(&format!("info string error: EvalFileを開けません: {e}"));
            std::process::exit(1);
        }
    };
    match himawari_engine::nnue_io::load(&mut f) {
        Ok((net, lineage)) => {
            print_line(&format!("info string EvalFile loaded: {path} ({lineage})"));
            Some((path.to_string(), std::sync::Arc::new(net)))
        }
        Err(e) => {
            print_line(&format!("info string error: EvalFile読み込み失敗: {e}"));
            std::process::exit(1);
        }
    }
}

fn main() {
    // stdin読み取り専用スレッド（ADR-0019）
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in std::io::BufRead::lines(stdin.lock()) {
            match line {
                Ok(l) => {
                    if tx.send(l).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        // EOFはquit扱い
        let _ = tx.send("quit".to_string());
    });

    let mut opts = EngineOptions::default();
    let mut pool: Option<ThreadPool> = None;
    let mut position = Position::from_sfen(SFEN_STARTPOS).expect("startpos");

    while let Ok(line) = rx.recv() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let Some(&cmd) = tokens.first() else {
            continue;
        };
        match cmd {
            "usi" => {
                print_line(&format!("id name {} {}", ENGINE_NAME, version_string()));
                print_line(&format!("id author {ENGINE_AUTHOR}"));
                print_options();
                print_line("usiok");
            }
            "setoption" => set_option(&mut opts, &tokens[1..]),
            "isready" => {
                // 重い初期化（置換表確保・スレッド起動・評価関数読み込み）は
                // ここで行う。Hash/Threads/EvalFileが変わったら作り直す
                let params = Some((opts.hash_mb, opts.threads.max(1), opts.eval_file.clone()));
                if pool
                    .as_ref()
                    .map(|p| (p.hash_mb, p.threads, p.eval_file.clone()))
                    != params
                {
                    if let Some(p) = pool.take() {
                        p.quit();
                    }
                    pool = Some(ThreadPool::new(
                        opts.hash_mb,
                        opts.threads,
                        load_eval(&opts.eval_file),
                        Arc::new(print_line),
                    ));
                }
                print_line("readyok");
            }
            "usinewgame" => {
                if let Some(p) = &pool {
                    p.new_game();
                }
            }
            "position" => match parse_position(&tokens[1..]) {
                Some(p) => position = p,
                None => print_line("info string error: invalid position"),
            },
            "go" => {
                let limits = parse_go(&tokens[1..]);
                let is_ponder = tokens.contains(&"ponder");
                if pool.is_none() {
                    pool = Some(ThreadPool::new(
                        opts.hash_mb,
                        opts.threads,
                        load_eval(&opts.eval_file),
                        Arc::new(print_line),
                    ));
                }
                if let Some(p) = &pool {
                    if is_ponder {
                        p.go_ponder(position.clone(), limits, opts.clone());
                    } else {
                        p.go(position.clone(), limits, opts.clone());
                    }
                }
            }
            "stop" | "gameover" => {
                if let Some(p) = &pool {
                    p.stop();
                }
            }
            "ponderhit" => {
                if let Some(p) = &pool {
                    p.ponderhit();
                }
            }
            "quit" => break,
            _ => {}
        }
    }
    if let Some(p) = pool.take() {
        p.quit();
    }
}
