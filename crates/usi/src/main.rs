//! USIエンジンのエントリポイント（ADR-0019）。
//!
//! stdin読み取りスレッド＋コマンドループ＋探索スレッド分離。
//! 出力は行単位でロックしてflushする。

mod book;

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

use himawari_core::{Position, SFEN_STARTPOS};
use himawari_engine::nnue::NnueNetwork;
use himawari_engine::{EngineOptions, Limits, ThreadPool};

use book::{Book, BookOptions};

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
static LOG_FILE: Mutex<Option<DebugLog>> = Mutex::new(None);

/// 開いているログ。パスに `%d` があると日付で開き直す。
struct DebugLog {
    /// setoptionで受けたパス。`%d` を含むと日毎のローテーションになる
    template: String,
    /// いま開いているファイルの通日。`%d` が無いときは使わない
    day: i64,
    writer: std::io::BufWriter<std::fs::File>,
}

/// UNIX時刻の秒を通日（1970-01-01を0とするUTCの日数）にする。
fn days_from_epoch(secs: i64) -> i64 {
    secs.div_euclid(86_400)
}

/// 通日をYYYY-MM-DDにする（Howard Hinnantのcivil_from_days）。
fn date_string(days: i64) -> String {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `%d` を日付へ置き換える。含まなければそのまま返す。
fn resolve_log_path(template: &str, day: i64) -> String {
    if template.contains("%d") {
        template.replace("%d", &date_string(day))
    } else {
        template.to_string()
    }
}

fn open_log_writer(path: &str) -> Result<std::io::BufWriter<std::fs::File>, String> {
    let f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("cannot open {path}: {e}"))?;
    Ok(std::io::BufWriter::new(f))
}

/// いまのUNIX時刻をミリ秒で返す。
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// ログを開き直す。開いたファイルのパス（`%d` を解決した後）を返す。
fn log_open(path: &str) -> Result<String, String> {
    let mut guard = LOG_FILE
        .lock()
        .map_err(|_| "failed to lock the debug log".to_string())?;
    if path.is_empty() {
        LOG_ON.store(false, Ordering::Relaxed);
        *guard = None;
        return Ok(String::new());
    }
    let day = days_from_epoch(now_millis().div_euclid(1_000));
    let resolved = resolve_log_path(path, day);
    let writer = open_log_writer(&resolved)?;
    *guard = Some(DebugLog {
        template: path.to_string(),
        day,
        writer,
    });
    LOG_ON.store(true, Ordering::Relaxed);
    Ok(resolved)
}

/// 行を1本書く。`dir` は `<`（受信）か `>`（送信）。
/// 落ちても末尾を失わないよう、毎行flushする。
fn log_line(dir: char, s: &str) {
    if !LOG_ON.load(Ordering::Relaxed) {
        return;
    }
    let ms = now_millis();
    if let Ok(mut guard) = LOG_FILE.lock()
        && let Some(log) = guard.as_mut()
    {
        let day = days_from_epoch(ms.div_euclid(1_000));
        // 日付が変わったら開き直す。開けなければ元のファイルへ書き続ける。
        // ここからprint_lineを呼ぶとログの排他で詰まるので、失敗は黙って捨てる。
        if day != log.day
            && log.template.contains("%d")
            && let Ok(w) = open_log_writer(&resolve_log_path(&log.template, day))
        {
            let _ = log.writer.flush();
            log.writer = w;
            log.day = day;
        }
        let _ = writeln!(log.writer, "{ms} {dir} {s}");
        let _ = log.writer.flush();
    }
}

/// USIへ出す行をASCIIに揃える（issue #745）。
///
/// Windowsの将棋所はエンジンの出力をcp932として読むので、UTF-8の日本語は
/// 化ける。固定の文言は英語で書くが、パスやOSのエラー文のように外から来る
/// 文字列は選べない（Windowsのio::Errorは日本語の文言を返す）。非ASCIIの
/// 文字は `\u{5c06}` の形へ置き換え、読み手が元の文字を辿れるようにする。
fn usi_ascii(s: &str) -> std::borrow::Cow<'_, str> {
    if s.is_ascii() {
        return std::borrow::Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            out.extend(c.escape_unicode());
        }
    }
    std::borrow::Cow::Owned(out)
}

/// 受信した1行を文字列にする。改行（`\n` と `\r\n`）を落とす。
///
/// UTF-8でないバイト列も捨てない。将棋所が日本語のパスをcp932で送ると、
/// `BufRead::lines` はErrを返し、読み取りループを抜けてエンジンが終わって
/// いた。不正なバイトはU+FFFDに置き換えて、その行のコマンドとして扱う。
fn decode_line(buf: &[u8]) -> String {
    let buf = buf.strip_suffix(b"\n").unwrap_or(buf);
    let buf = buf.strip_suffix(b"\r").unwrap_or(buf);
    String::from_utf8_lossy(buf).into_owned()
}

fn print_line(s: &str) {
    let s = usi_ascii(s);
    let s = s.as_ref();
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
    // 実務オプション（ADR-0109のG10。S:151-155）
    print_line("option name ResignValue type spin default 99999 min 0 max 99999");
    print_line("option name DrawValueBlack type spin default -2 min -30000 max 30000");
    print_line("option name DrawValueWhite type spin default -2 min -30000 max 30000");
    print_line("option name EvalFile type string default <empty>");
    print_line("option name BookFile type string default <empty>");
    print_line("option name BookDepth type spin default 24 min 0 max 1000");
    // 定跡の引き方（ADR-0109のG10。既定値はbook.cpp:1308-1392）
    print_line("option name USI_OwnBook type check default true");
    print_line("option name BookIgnoreRate type spin default 0 min 0 max 100");
    print_line("option name BookEvalDiff type spin default 30 min 0 max 99999");
    print_line("option name BookEvalBlackLimit type spin default 0 min -99999 max 99999");
    print_line("option name BookEvalWhiteLimit type spin default -140 min -99999 max 99999");
    print_line("option name BookDepthLimit type spin default 16 min 0 max 99999");
    print_line("option name FlippedBook type check default true");
    print_line("option name BookRandomize type check default false");
    print_line("option name DebugLogFile type string default <empty>");
    // SPSAのチューニング項目（ADR-0143）。tuneビルドだけが宣言する
    #[cfg(feature = "tune")]
    for e in himawari_engine::tunables::ENTRIES {
        print_line(&format!(
            "option name {} type spin default {} min {} max {}",
            e.name, e.default, e.min, e.max
        ));
    }
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
    // 計時の起点はgoを受け取った時刻にする（usi.cpp:506）。
    // 解析やスレッド起動にかかる分だけ計時が遅れるのを防ぐ
    let mut limits = Limits {
        start: Some(Instant::now()),
        ..Limits::default()
    };
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
        "ResignValue" => {
            if let Ok(v) = value.parse() {
                opts.resign_value = v;
            }
        }
        "DrawValueBlack" => {
            if let Ok(v) = value.parse() {
                opts.draw_value_black = v;
            }
        }
        "DrawValueWhite" => {
            if let Ok(v) = value.parse() {
                opts.draw_value_white = v;
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
                bopts.params.book_moves = v;
            }
        }
        "USI_OwnBook" => bopts.params.own_book = value == "true",
        "BookIgnoreRate" => {
            if let Ok(v) = value.parse() {
                bopts.params.ignore_rate = v;
            }
        }
        "BookEvalDiff" => {
            if let Ok(v) = value.parse() {
                bopts.params.eval_diff = v;
            }
        }
        "BookEvalBlackLimit" => {
            if let Ok(v) = value.parse() {
                bopts.params.eval_black_limit = v;
            }
        }
        "BookEvalWhiteLimit" => {
            if let Ok(v) = value.parse() {
                bopts.params.eval_white_limit = v;
            }
        }
        "BookDepthLimit" => {
            if let Ok(v) = value.parse() {
                bopts.params.depth_limit = v;
            }
        }
        "FlippedBook" => bopts.params.flipped = value == "true",
        "BookRandomize" => bopts.params.randomize = value == "true",
        // SPSAのチューニング項目（ADR-0143）。tuneビルドだけが受ける
        #[cfg(feature = "tune")]
        name if himawari_engine::tunables::ENTRIES
            .iter()
            .any(|e| e.name == name) =>
        {
            if let Ok(v) = value.parse::<i64>() {
                himawari_engine::tunables::set(name, v);
            }
        }
        "DebugLogFile" => {
            let path: &str = if value == "<empty>" {
                ""
            } else {
                value.as_ref()
            };
            match log_open(path) {
                Ok(_) if path.is_empty() => print_line("info string debug log off"),
                Ok(resolved) => print_line(&format!("info string debug log -> {resolved}")),
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
                "info string BookFile loaded: {path} ({} positions)",
                b.positions()
            ));
            Some(b)
        }
        Err(e) => {
            print_line(&format!("info string warning: cannot read BookFile: {e}"));
            print_line(&format!("info string   path = {path:?}"));
            print_line(&format!(
                "info string   cwd  = {:?}",
                std::env::current_dir().unwrap_or_default()
            ));
            None
        }
    }
}

/// 定跡を引き、ヒットしたら返すbestmoveの行を作る（ADR-0063）。
/// 引いた経過はinfo stringで出す。
fn probe_book(bopts: &mut BookOptions, pos: &Position, ponder: bool) -> Option<String> {
    let mut notes = Vec::new();
    let hit = bopts.probe(pos, &mut notes);
    for n in notes {
        print_line(&format!("info string {n}"));
    }
    let h = hit?;
    print_line(&format!(
        "info string book hit: score {} depth {}",
        h.value, h.depth
    ));
    // 定跡の予想応手をponder手として返す（B:58、S:1364-1369）。
    // これがないと定跡区間でGUIがponderを始められない
    let ponder_hint = if ponder && h.ponder != himawari_core::Move::NONE {
        format!(" ponder {}", h.ponder.to_usi())
    } else {
        String::new()
    };
    Some(format!("bestmove {}{}", h.mv.to_usi(), ponder_hint))
}

/// 配布する評価関数の固定名（ADR-0080の追記）。
const DEFAULT_EVAL_NAME: &str = "eval.hmwr";

/// 読み込む評価関数のパスを決める。EvalFileが空なら、実行ファイルと
/// 同じフォルダの固定名にする（issue #744）。放流機の更新が上書きコピー
/// だけで済む。カレントディレクトリはGUIの都合で変わるので基準にしない。
/// 実行ファイルの場所が分からなければ空のまま返し、load_evalが止める。
fn eval_path(eval_file: &str) -> String {
    if !eval_file.is_empty() {
        return eval_file.to_string();
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(DEFAULT_EVAL_NAME)))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// EvalFileを読み込む。読めなければ起動エラー（ADR-0037: 駒割への
/// フォールバックはしない。気づかず弱いまま対局する事故を防ぐ）。
fn load_eval(path: &str) -> Option<(String, std::sync::Arc<NnueNetwork>)> {
    if path.is_empty() {
        print_line("info string error: EvalFile is not set");
        print_line("info string   send setoption name EvalFile value <path> before isready");
        std::process::exit(1);
    }
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            // パスはデバッグ表示で出す。制御文字や全角空白が混入していても
            // エスケープされて見えるため、原因の切り分けができる
            print_line(&format!("info string error: cannot open EvalFile: {e}"));
            print_line(&format!(
                "info string   path = {path:?} ({} chars, {} bytes)",
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
            print_line(&format!("info string error: failed to load EvalFile: {e}"));
            std::process::exit(1);
        }
    }
}

fn main() {
    // `himawari threadtune ...` は動作環境の最適なスレッド数を測るモード
    // （ADR-0203）。引数なしならUSIエンジンとして動く
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("threadtune") {
        std::process::exit(i32::from(himawari_usi::threadtune::main(&args[1..])));
    }
    // panicは黙って止めない。探索スレッドが落ちるとbestmoveが永久に返らず、
    // 実戦では残り時間を抱えたまま時間切れになる（floodgateで60手以上の
    // 時間切れ19局、ADR-0199）。原因をログとstderrへ残し、プロセスごと
    // 終える。落ちたことがGUIとログから見えるようにする
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("panic: {info}");
        log_line('!', &msg);
        eprintln!("info string error: {msg}");
        std::process::exit(101);
    }));
    // stdin読み取り専用スレッド（ADR-0019）
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match std::io::BufRead::read_until(&mut reader, b'\n', &mut buf) {
                Ok(0) => break,
                Ok(_) => {
                    let l = decode_line(&buf);
                    log_line('<', &l);
                    if tx.send(l).is_err() {
                        return;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        // EOFはquit扱い
        let _ = tx.send("quit".to_string());
    });

    let mut opts = EngineOptions::default();
    let mut bopts = BookOptions::default();
    // go infinite中に定跡へヒットしたときの保留（S:1157-1187）。
    // 探索をスキップしたままstopを待ち、そのあとでbestmoveを出す。
    // 探索スレッドが動いていないのでUSI層で保持する
    let mut pending_book: Option<String> = None;
    // go ponderの局面。定跡はponderhitで引く（ADR-0231）。ponder中は
    // 定跡を引かずに探索し、定跡を抜けた直後の置換表を温める
    let mut ponder_root: Option<Position> = None;
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
            // SPSAランナーがチューニング項目を発見するための一覧出力
            // （ADR-0143）。tuneビルドだけが応答する
            #[cfg(feature = "tune")]
            "tunables" => {
                for e in himawari_engine::tunables::ENTRIES {
                    print_line(&format!(
                        "tunable name {} default {} min {} max {}",
                        e.name, e.default, e.min, e.max
                    ));
                }
                print_line("tunablesok");
            }
            "isready" => {
                // 重い初期化（置換表確保・スレッド起動・評価関数読み込み）は
                // ここで行う。Hash/Threads/EvalFileが変わったら作り直す
                // 比べるのは解決後のパス。プールは読んだパスを持つので、
                // 空のまま比べると毎回作り直しになる
                let eval_file = eval_path(&opts.eval_file);
                let params = Some((opts.hash_mb, opts.threads.max(1), eval_file.clone()));
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
                        load_eval(&eval_file),
                        Arc::new(print_line),
                    ));
                }
                print_line("readyok");
            }
            "usinewgame" => {
                // 前局の保留が残っていても持ち越さない
                pending_book = None;
                ponder_root = None;
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
                // 前のgoの保留は持ち越さない
                pending_book = None;
                ponder_root = None;
                // 定跡ヒットなら探索しない（ADR-0063）。go ponderでは引かずに
                // 探索し、ponderhitで引く（ADR-0231。参照実装のS:1115-1117は
                // ponderでも引くが、それだと定跡区間の相手番に探索しない）
                if is_ponder {
                    ponder_root = Some(position.clone());
                } else if let Some(line) = probe_book(&mut bopts, &position, opts.ponder) {
                    // go infiniteではGUIの指示を待ってから出す
                    // （S:1157-1160の `ponder || limits.infinite`）
                    if limits.infinite {
                        pending_book = Some(line);
                    } else {
                        print_line(&line);
                    }
                    continue;
                }
                if pool.is_none() {
                    pool = Some(ThreadPool::new(
                        opts.hash_mb,
                        opts.threads,
                        load_eval(&eval_path(&opts.eval_file)),
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
            "stop" => {
                // ponderが外れた。探索の結論をそのまま出す
                ponder_root = None;
                // 保留していた定跡のbestmoveをここで出す（S:1157-1187）
                if let Some(line) = pending_book.take() {
                    print_line(&line);
                }
                if let Some(p) = &pool {
                    p.stop();
                }
            }
            "gameover" => {
                // 対局終了なのでbestmoveは出さない
                pending_book = None;
                ponder_root = None;
                if let Some(p) = &pool {
                    p.stop();
                }
            }
            "ponderhit" => {
                // ponderした局面を定跡で引く（ADR-0231）。ヒットしたら探索の
                // 結論を出さずに止め、終わるのを待ってから定跡の手を出す。
                // 1回のgoに対してbestmoveは1本だけになる
                let book_line = ponder_root
                    .take()
                    .and_then(|root| probe_book(&mut bopts, &root, opts.ponder));
                match (book_line, &pool) {
                    (Some(line), Some(p)) => {
                        p.abandon_ponder();
                        print_line(&line);
                    }
                    (Some(line), None) => print_line(&line),
                    (None, Some(p)) => p.ponderhit(),
                    (None, None) => {}
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_string_matches_known_days() {
        assert_eq!(date_string(0), "1970-01-01");
        assert_eq!(date_string(20_714), "2026-09-18");
        // 閏日の前後と、400年規則の閏年
        assert_eq!(date_string(19_782), "2024-02-29");
        assert_eq!(date_string(19_783), "2024-03-01");
        assert_eq!(date_string(11_016), "2000-02-29");
        // 1970より前は通日が負になる
        assert_eq!(date_string(-1), "1969-12-31");
        assert_eq!(date_string(-25_508), "1900-03-01");
    }

    #[test]
    fn usi_ascii_escapes_non_ascii() {
        assert_eq!(usi_ascii("readyok"), "readyok");
        assert_eq!(
            usi_ascii("info string path = C:\\将棋\\a.db"),
            "info string path = C:\\\\u{5c06}\\u{68cb}\\a.db"
        );
        assert!(usi_ascii("全角\u{3000}空白").is_ascii());
    }

    #[test]
    fn decode_line_keeps_non_utf8_bytes_as_a_line() {
        assert_eq!(decode_line(b"isready\n"), "isready");
        assert_eq!(decode_line(b"isready\r\n"), "isready");
        assert_eq!(decode_line(b"quit"), "quit");
        // cp932の「将棋」。UTF-8として不正でも行として残す
        let line = decode_line(b"setoption name EvalFile value C:\\\x8f\xab\x8a\xfb\\a\r\n");
        assert!(line.starts_with("setoption name EvalFile value C:\\"));
        assert!(line.ends_with("\\a"));
    }

    #[test]
    fn days_from_epoch_rounds_down() {
        assert_eq!(days_from_epoch(0), 0);
        assert_eq!(days_from_epoch(86_399), 0);
        assert_eq!(days_from_epoch(86_400), 1);
        assert_eq!(days_from_epoch(-1), -1);
    }

    #[test]
    fn resolve_log_path_replaces_placeholder() {
        assert_eq!(
            resolve_log_path("/var/log/usi-%d.log", 20_714),
            "/var/log/usi-2026-09-18.log"
        );
        assert_eq!(
            resolve_log_path("/var/log/usi.log", 20_714),
            "/var/log/usi.log"
        );
    }

    /// ローテーションはグローバルなログを触るので、1つのテストにまとめる。
    #[test]
    fn debug_log_reopens_when_day_changes() {
        let dir = std::env::temp_dir().join(format!("himawari-usilog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create_dir_all");

        let today = days_from_epoch(now_millis().div_euclid(1_000));
        let template = dir.join("usi-%d.log").to_string_lossy().to_string();

        // %dつきのパスは、開いた時点の日付のファイルになる
        let opened = log_open(&template).expect("log_open");
        assert_eq!(opened, resolve_log_path(&template, today));

        // 昨日から開きっぱなしの状態を作る
        set_open_day(&template, today - 1);
        log_line('<', "after midnight");
        let yesterday =
            std::fs::read_to_string(resolve_log_path(&template, today - 1)).expect("read");
        let current = std::fs::read_to_string(&opened).expect("read");
        assert!(!yesterday.contains("after midnight"));
        assert!(current.contains("after midnight"));

        // %dが無いパスは日付が変わっても開き直さない
        let plain = dir.join("usi.log").to_string_lossy().to_string();
        log_open(&plain).expect("log_open");
        set_open_day(&plain, today - 1);
        log_line('>', "same file");
        assert!(
            std::fs::read_to_string(&plain)
                .expect("read")
                .contains("same file")
        );

        log_open("").expect("log_open");
        assert!(!LOG_ON.load(Ordering::Relaxed));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 開いている日付を過去へずらし、その日のファイルを書き先にする。
    fn set_open_day(template: &str, day: i64) {
        let mut guard = LOG_FILE.lock().expect("lock");
        let log = guard.as_mut().expect("open");
        log.writer = open_log_writer(&resolve_log_path(template, day)).expect("open_log_writer");
        log.day = day;
    }
}
