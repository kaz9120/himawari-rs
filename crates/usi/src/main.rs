//! USIエンジンのエントリポイント（ADR-0019）。
//!
//! stdin読み取りスレッド＋コマンドループ＋探索スレッド分離。
//! 出力は行単位でロックしてflushする。

mod book;

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use himawari_core::{Position, SFEN_STARTPOS};
use himawari_engine::nnue::NnueNetwork;
use himawari_engine::{EngineOptions, Limits, ThreadPool};

use book::Book;

/// 定跡の設定（ADR-0063）。探索器には渡さずUSI層で閉じる。
#[derive(Default)]
struct BookOptions {
    file: String,
    depth: u16,
    book: Option<Book>,
}

const ENGINE_NAME: &str = "Himawari";
const ENGINE_AUTHOR: &str = "Kazumasa Yamamoto";

fn version_string() -> String {
    // devビルドの識別はCIが設定するHIMAWARI_BUILD_ID（ADR-0007）
    match option_env!("HIMAWARI_BUILD_ID") {
        Some(id) => format!("{} ({})", env!("CARGO_PKG_VERSION"), id),
        None => format!("{}-dev", env!("CARGO_PKG_VERSION")),
    }
}

/// USIの入出力をファイルへ写す（`DebugLogFile`）。
///
/// floodgateの2026-07-29の対局で、棋譜のコメントに残った `4723++` だけが
/// 手掛かりになり、原因の特定に時間がかかった。info行が残っていれば
/// すぐ分かる。探索の中には入れず、USI層の行だけを写すので、1手あたり
/// 数十行にしかならない。無指定なら分岐1つ分のコストで済む。
static LOG_ON: AtomicBool = AtomicBool::new(false);
static LOG_FILE: Mutex<Option<std::io::BufWriter<std::fs::File>>> = Mutex::new(None);

fn log_open(path: &str) -> Result<(), String> {
    let mut guard = LOG_FILE
        .lock()
        .map_err(|_| "ログの排他に失敗".to_string())?;
    if path.is_empty() {
        LOG_ON.store(false, Ordering::Relaxed);
        *guard = None;
        return Ok(());
    }
    let f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("{path} を開けない: {e}"))?;
    *guard = Some(std::io::BufWriter::new(f));
    LOG_ON.store(true, Ordering::Relaxed);
    Ok(())
}

/// 行を1本書く。`dir` は `<`（受信）か `>`（送信）。
/// 落ちても末尾を失わないよう、毎行flushする。
fn log_line(dir: char, s: &str) {
    if !LOG_ON.load(Ordering::Relaxed) {
        return;
    }
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if let Ok(mut guard) = LOG_FILE.lock()
        && let Some(w) = guard.as_mut()
    {
        let _ = writeln!(w, "{ms} {dir} {s}");
        let _ = w.flush();
    }
}

fn print_line(s: &str) {
    log_line('>', s);
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
    print_line("option name MinimumThinkingTime type spin default 2000 min 1 max 100000");
    print_line("option name SlowMover type spin default 100 min 1 max 1000");
    print_line("option name RoundUpToFullSecond type check default true");
    print_line("option name MaxMovesToDraw type spin default 0 min 0 max 100000");
    print_line("option name MultiPV type spin default 1 min 1 max 128");
    print_line("option name EvalFile type string default <empty>");
    print_line("option name BookFile type string default <empty>");
    print_line("option name BookDepth type spin default 24 min 0 max 1000");
    print_line("option name DebugLogFile type string default <empty>");
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

fn set_option(opts: &mut EngineOptions, bopts: &mut BookOptions, tokens: &[&str], line: &str) {
    // setoption name <id> value <x>
    let name_idx = tokens.iter().position(|&t| t == "name");
    let value_idx = tokens.iter().position(|&t| t == "value");
    let (Some(ni), Some(vi)) = (name_idx, value_idx) else {
        return;
    };
    let name = tokens[ni + 1..vi].join(" ");
    // 値は元の行から切り出す。トークン列をjoin(" ")で復元すると、
    // split_whitespaceがUnicodeの空白（全角スペース U+3000 など）も
    // 区切りにするため、それを含むパスが半角スペースへ化ける
    let value = match line.find(" value ") {
        Some(i) => line[i + " value ".len()..].trim(),
        None => "",
    };
    // 値を囲む引用符は落とす。USIは引用符を使わないが、Windowsのパスを
    // 扱う習慣で付けて渡されることがある。引用符はWindowsのファイル名に
    // 使えない文字なので、残すと ERROR_INVALID_NAME になる
    let value = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value)
        .to_string();
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
        "MinimumThinkingTime" => {
            if let Ok(v) = value.parse() {
                opts.minimum_thinking_time = v;
            }
        }
        "SlowMover" => {
            if let Ok(v) = value.parse() {
                opts.slow_mover = v;
            }
        }
        "RoundUpToFullSecond" => {
            opts.round_up_to_full_second = value == "true";
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
        "BookFile" => {
            bopts.file = if value == "<empty>" {
                String::new()
            } else {
                value
            };
            bopts.book = load_book(&bopts.file);
        }
        "BookDepth" => {
            if let Ok(v) = value.parse() {
                bopts.depth = v;
            }
        }
        "DebugLogFile" => {
            let path: &str = if value == "<empty>" {
                ""
            } else {
                value.as_ref()
            };
            match log_open(path) {
                Ok(()) if path.is_empty() => print_line("info string debug log off"),
                Ok(()) => print_line(&format!("info string debug log -> {path}")),
                Err(e) => print_line(&format!("info string {e}")),
            }
        }
        _ => {}
    }
}

/// 定跡ファイルを読み込む（ADR-0063）。EvalFileと違い、読めなくても
/// 起動を止めない。定跡なしで対局できるため、事故にはならない。
fn load_book(path: &str) -> Option<Book> {
    if path.is_empty() {
        return None;
    }
    match Book::load(path) {
        Ok(b) => {
            print_line(&format!(
                "info string BookFile loaded: {path} ({}局面)",
                b.positions()
            ));
            Some(b)
        }
        Err(e) => {
            print_line(&format!("info string warning: BookFileを読めません: {e}"));
            print_line(&format!("info string   path = {path:?}"));
            print_line(&format!(
                "info string   cwd  = {:?}",
                std::env::current_dir().unwrap_or_default()
            ));
            None
        }
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
            // パスはデバッグ表示で出す。制御文字や全角空白が混入していても
            // エスケープされて見えるため、原因の切り分けができる
            print_line(&format!("info string error: EvalFileを開けません: {e}"));
            print_line(&format!(
                "info string   path = {path:?} ({}文字 {}バイト)",
                path.chars().count(),
                path.len()
            ));
            print_line(&format!(
                "info string   cwd  = {:?}",
                std::env::current_dir().unwrap_or_default()
            ));
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
                    log_line('<', &l);
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
    let mut bopts = BookOptions {
        depth: 24,
        ..BookOptions::default()
    };
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
            "setoption" => set_option(&mut opts, &mut bopts, &tokens[1..], &line),
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
                // 定跡ヒットなら探索せず即指す（ADR-0063）。
                // ponderは相手番を読む処理なので定跡を引かない
                if !is_ponder
                    && position.game_ply() <= bopts.depth
                    && let Some(e) = bopts.book.as_ref().and_then(|b| b.probe(&position))
                {
                    print_line("info string book hit");
                    print_line(&format!("bestmove {}", e.mv));
                    continue;
                }
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
