//! 自己対局・SPRT判定ツール（ADR-0027）。
//!
//! ベースラインと候補の2エンジンを、同一開始局面の先後入れ替えペアで
//! 対局させる。ペア得点のpentanomial度数からGSPRTのLLRを逐次更新し、
//! 閾値到達で打ち切る。
//!
//! 使い方:
//!   selfplay --baseline <path> --candidate <path> [--openings <file>]
//!            [--tc 10+0.1 | --nodes N] [--concurrency N] [--hash MB]
//!            [--elo0 0] [--elo1 5] [--alpha 0.05] [--beta 0.05]
//!            [--max-pairs N] [--max-moves 320]
//!            [--adjudicate CP,PLIES] [--option Name=Value]...
//!            [--out <path>]
//!
//! openingsは1行1局面のSFEN。#始まりはコメント。省略時は平手初期局面
//! のみ（決定的エンジン同士では毎ペア同一の進行になるため注意）。
//! 棋譜は--out（既定 selfplay.jsonl）へ1局1行のJSONで追記する。
//! 終了コード: 0=H1採択、1=H0採択、2=判定に至らず、3=実行エラー。

mod engine;
mod game;
mod sprt;

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use himawari_core::{Color, Position, SFEN_STARTPOS};

use engine::UsiEngine;
use game::{GameConfig, GameRecord, TimeControl, play_game};
use sprt::{Decision, Pentanomial, Sprt, elo_estimate};

struct Config {
    baseline: String,
    candidate: String,
    openings: Vec<String>,
    tc: TcSpec,
    concurrency: usize,
    hash_mb: u64,
    sprt: Sprt,
    max_pairs: u64,
    max_moves: usize,
    adjudicate: Option<(i32, u32)>,
    ponder: bool,
    options: Vec<(String, String)>,
    /// 候補側 / ベースライン側だけに適用するオプション。
    copts: Vec<(String, String)>,
    bopts: Vec<(String, String)>,
    out: String,
}

/// TimeControlはCloneしないため、生成用の仕様を別に持つ。
#[derive(Clone, Copy)]
enum TcSpec {
    Fischer { base_ms: u64, inc_ms: u64 },
    Nodes(u64),
}

impl TcSpec {
    fn to_time_control(self) -> TimeControl {
        match self {
            TcSpec::Fischer { base_ms, inc_ms } => TimeControl::Fischer { base_ms, inc_ms },
            TcSpec::Nodes(n) => TimeControl::Nodes(n),
        }
    }
}

fn usage_exit(msg: &str) -> ! {
    eprintln!("{msg}");
    eprintln!("使い方は crates/tools/src/bin/selfplay/main.rs 冒頭のコメントを参照");
    std::process::exit(3)
}

fn parse_args() -> Config {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut baseline = None;
    let mut candidate = None;
    let mut openings_path: Option<String> = None;
    let mut tc = TcSpec::Fischer {
        base_ms: 10_000,
        inc_ms: 100,
    };
    let mut concurrency = 1usize;
    let mut hash_mb = 64u64;
    let mut elo0 = 0.0;
    let mut elo1 = 5.0;
    let mut alpha = 0.05;
    let mut beta = 0.05;
    let mut max_pairs = 0u64;
    let mut max_moves = 320usize;
    let mut adjudicate = None;
    let mut ponder = false;
    let mut options: Vec<(String, String)> = Vec::new();
    let mut copts: Vec<(String, String)> = Vec::new();
    let mut bopts: Vec<(String, String)> = Vec::new();
    let mut out = "selfplay.jsonl".to_string();

    let mut i = 0;
    let value = |args: &[String], i: usize| -> String {
        args.get(i + 1)
            .cloned()
            .unwrap_or_else(|| usage_exit(&format!("{} に値がありません", args[i])))
    };
    while i < args.len() {
        match args[i].as_str() {
            "--baseline" => baseline = Some(value(&args, i)),
            "--candidate" => candidate = Some(value(&args, i)),
            "--openings" => openings_path = Some(value(&args, i)),
            "--tc" => {
                let v = value(&args, i);
                let Some((b, inc)) = v.split_once('+') else {
                    usage_exit(&format!("--tc は base+inc 形式（秒）: {v}"));
                };
                let (Ok(b), Ok(inc)) = (b.parse::<f64>(), inc.parse::<f64>()) else {
                    usage_exit(&format!("--tc を数値にできません: {v}"));
                };
                tc = TcSpec::Fischer {
                    base_ms: (b * 1000.0).round() as u64,
                    inc_ms: (inc * 1000.0).round() as u64,
                };
            }
            "--nodes" => {
                tc = TcSpec::Nodes(
                    value(&args, i)
                        .parse()
                        .unwrap_or_else(|_| usage_exit("--nodes は整数")),
                )
            }
            "--concurrency" => {
                concurrency = value(&args, i)
                    .parse()
                    .unwrap_or_else(|_| usage_exit("--concurrency は整数"))
            }
            "--hash" => {
                hash_mb = value(&args, i)
                    .parse()
                    .unwrap_or_else(|_| usage_exit("--hash は整数(MB)"))
            }
            "--elo0" => {
                elo0 = value(&args, i)
                    .parse()
                    .unwrap_or_else(|_| usage_exit("--elo0 は数値"))
            }
            "--elo1" => {
                elo1 = value(&args, i)
                    .parse()
                    .unwrap_or_else(|_| usage_exit("--elo1 は数値"))
            }
            "--alpha" => {
                alpha = value(&args, i)
                    .parse()
                    .unwrap_or_else(|_| usage_exit("--alpha は数値"))
            }
            "--beta" => {
                beta = value(&args, i)
                    .parse()
                    .unwrap_or_else(|_| usage_exit("--beta は数値"))
            }
            "--max-pairs" => {
                max_pairs = value(&args, i)
                    .parse()
                    .unwrap_or_else(|_| usage_exit("--max-pairs は整数"))
            }
            "--max-moves" => {
                max_moves = value(&args, i)
                    .parse()
                    .unwrap_or_else(|_| usage_exit("--max-moves は整数"))
            }
            "--adjudicate" => {
                let v = value(&args, i);
                let parsed = v
                    .split_once(',')
                    .and_then(|(cp, n)| Some((cp.parse().ok()?, n.parse().ok()?)));
                let Some(pair) = parsed else {
                    usage_exit(&format!("--adjudicate は CP,PLIES 形式: {v}"));
                };
                adjudicate = Some(pair);
            }
            "--option" | "--copt" | "--bopt" => {
                let v = value(&args, i);
                let Some((name, val)) = v.split_once('=') else {
                    usage_exit(&format!("{} は Name=Value 形式: {v}", args[i]));
                };
                let entry = (name.to_string(), val.to_string());
                match args[i].as_str() {
                    "--copt" => copts.push(entry),
                    "--bopt" => bopts.push(entry),
                    _ => options.push(entry),
                }
            }
            "--out" => out = value(&args, i),
            "--ponder" => {
                ponder = true;
                i += 1;
                continue;
            }
            other => usage_exit(&format!("不明な引数: {other}")),
        }
        i += 2;
    }

    let Some(baseline) = baseline else {
        usage_exit("--baseline は必須");
    };
    let Some(candidate) = candidate else {
        usage_exit("--candidate は必須");
    };
    let openings = match &openings_path {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .unwrap_or_else(|e| usage_exit(&format!("開始局面集を読めません: {e}")));
            // 行頭の「sfen 」は配布ファイルで一般的なので剥がして受け入れる
            let list: Vec<String> = text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| l.strip_prefix("sfen ").unwrap_or(l).to_string())
                .collect();
            if list.is_empty() {
                usage_exit("開始局面集が空です");
            }
            for (i, sfen) in list.iter().enumerate() {
                if Position::from_sfen(sfen).is_err() {
                    usage_exit(&format!("開始局面集の{}行目が不正なSFEN: {sfen}", i + 1));
                }
            }
            list
        }
        None => {
            eprintln!("警告: 開始局面集なし。平手のみでは毎ペア同一進行になります");
            vec![SFEN_STARTPOS.to_string()]
        }
    };
    Config {
        baseline,
        candidate,
        openings,
        tc,
        concurrency,
        hash_mb,
        sprt: Sprt {
            elo0,
            elo1,
            alpha,
            beta,
        },
        max_pairs,
        max_moves,
        adjudicate,
        ponder,
        options,
        copts,
        bopts,
        out,
    }
}

struct Aggregate {
    pent: Pentanomial,
    /// 候補側から見た勝ち・引き分け・負け（局単位）。
    wdl: [u64; 3],
    /// LLRが最初に閾値へ到達した時点の判定。SPRTは逐次検定なので、
    /// 停止後に完走した飛行中ペアで判定を再計算してはならない。
    decision: Option<Decision>,
    out: std::fs::File,
}

fn candidate_score(rec: &GameRecord, candidate_color: Color) -> f64 {
    match rec.winner {
        None => 0.5,
        Some(c) if c == candidate_color => 1.0,
        Some(_) => 0.0,
    }
}

fn jsonl_line(
    pair: u64,
    game: u8,
    opening: &str,
    candidate_color: Color,
    rec: &GameRecord,
) -> String {
    let winner = match rec.winner {
        Some(Color::Black) => "b",
        Some(Color::White) => "w",
        None => "draw",
    };
    let cand = if candidate_color == Color::Black {
        "b"
    } else {
        "w"
    };
    format!(
        "{{\"pair\":{pair},\"game\":{game},\"opening\":\"{opening}\",\"candidate\":\"{cand}\",\"winner\":\"{winner}\",\"reason\":\"{}\",\"plies\":{},\"moves\":\"{}\"}}",
        rec.reason,
        rec.moves.len(),
        rec.moves.join(" ")
    )
}

fn record_game(agg: &mut Aggregate, rec: &GameRecord, candidate_color: Color) {
    let s = candidate_score(rec, candidate_color);
    let idx = if s == 1.0 {
        0
    } else if s == 0.5 {
        1
    } else {
        2
    };
    agg.wdl[idx] += 1;
}

#[allow(clippy::too_many_arguments)]
fn worker(
    cfg: &Config,
    stop: &AtomicBool,
    counter: &AtomicU64,
    agg: &Mutex<Aggregate>,
) -> Result<(), String> {
    let common = vec![
        ("USI_Hash".to_string(), cfg.hash_mb.to_string()),
        ("Threads".to_string(), "1".to_string()),
    ];

    let mut base_opts = common.clone();
    base_opts.extend(cfg.options.iter().cloned());
    base_opts.extend(cfg.bopts.iter().cloned());
    let mut cand_opts = common;
    cand_opts.extend(cfg.options.iter().cloned());
    cand_opts.extend(cfg.copts.iter().cloned());
    if cfg.ponder {
        cand_opts.push(("USI_Ponder".to_string(), "true".to_string()));
    }
    let mut baseline = UsiEngine::launch(&cfg.baseline, &base_opts)?;
    let mut candidate = UsiEngine::launch(&cfg.candidate, &cand_opts)?;
    let game_cfg = GameConfig {
        tc: cfg.tc.to_time_control(),
        max_moves: cfg.max_moves,
        adjudicate: cfg.adjudicate,
    };

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let pair = counter.fetch_add(1, Ordering::Relaxed);
        if cfg.max_pairs > 0 && pair >= cfg.max_pairs {
            break;
        }
        let opening = &cfg.openings[(pair as usize) % cfg.openings.len()];
        // 候補が先手→後手の順で同一開始局面のペアを消化する
        // ponderは候補側だけに適用する（効果測定モード。ADR-0033）
        let g1 = play_game(
            &mut candidate,
            &mut baseline,
            opening,
            &game_cfg,
            [cfg.ponder, false],
        )?;
        let g2 = play_game(
            &mut baseline,
            &mut candidate,
            opening,
            &game_cfg,
            [false, cfg.ponder],
        )?;
        let s1 = candidate_score(&g1, Color::Black);
        let s2 = candidate_score(&g2, Color::White);
        let bin = ((s1 + s2) * 2.0).round() as usize;

        let mut a = agg.lock().expect("agg lock");
        a.pent.add(bin);
        record_game(&mut a, &g1, Color::Black);
        record_game(&mut a, &g2, Color::White);
        let line1 = jsonl_line(pair, 1, opening, Color::Black, &g1);
        let line2 = jsonl_line(pair, 2, opening, Color::White, &g2);
        let _ = writeln!(a.out, "{line1}\n{line2}");
        let llr = cfg.sprt.llr(&a.pent);
        let (lower, upper) = cfg.sprt.bounds();
        let (elo, lo, hi) = elo_estimate(&a.pent);
        let p = a.pent.0;
        println!(
            "pairs {:>5} | +{} ={} -{} | [{},{},{},{},{}] | Elo {elo:+.1} [{lo:+.1},{hi:+.1}] | LLR {llr:+.2} [{lower:.2},{upper:.2}]",
            a.pent.total(),
            a.wdl[0],
            a.wdl[1],
            a.wdl[2],
            p[0],
            p[1],
            p[2],
            p[3],
            p[4],
        );
        match cfg.sprt.decision(llr) {
            Decision::Continue => {}
            d => {
                if a.decision.is_none() {
                    a.decision = Some(d);
                }
                stop.store(true, Ordering::Relaxed);
            }
        }
    }
    baseline.quit();
    candidate.quit();
    Ok(())
}

fn main() {
    let cfg = Arc::new(parse_args());
    let tc_str = match cfg.tc {
        TcSpec::Fischer { base_ms, inc_ms } => {
            format!("{}+{}", base_ms as f64 / 1000.0, inc_ms as f64 / 1000.0)
        }
        TcSpec::Nodes(n) => format!("nodes {n}"),
    };
    println!(
        "selfplay: {} vs {} | tc {tc_str} | 並列 {} | SPRT elo[{}, {}] α={} β={} | 開始局面 {}件",
        cfg.candidate,
        cfg.baseline,
        cfg.concurrency,
        cfg.sprt.elo0,
        cfg.sprt.elo1,
        cfg.sprt.alpha,
        cfg.sprt.beta,
        cfg.openings.len(),
    );

    let out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&cfg.out)
        .unwrap_or_else(|e| {
            eprintln!("棋譜ファイルを開けません: {e}");
            std::process::exit(3)
        });
    let agg = Arc::new(Mutex::new(Aggregate {
        pent: Pentanomial::default(),
        wdl: [0; 3],
        decision: None,
        out,
    }));
    let stop = Arc::new(AtomicBool::new(false));
    let counter = Arc::new(AtomicU64::new(0));
    let failed = Arc::new(AtomicBool::new(false));

    let handles: Vec<_> = (0..cfg.concurrency.max(1))
        .map(|_| {
            let (cfg, stop, counter, agg, failed) = (
                Arc::clone(&cfg),
                Arc::clone(&stop),
                Arc::clone(&counter),
                Arc::clone(&agg),
                Arc::clone(&failed),
            );
            std::thread::spawn(move || {
                if let Err(e) = worker(&cfg, &stop, &counter, &agg) {
                    eprintln!("エラー: {e}");
                    failed.store(true, Ordering::Relaxed);
                    stop.store(true, Ordering::Relaxed);
                }
            })
        })
        .collect();
    for h in handles {
        let _ = h.join();
    }

    let a = agg.lock().expect("agg lock");
    let llr = cfg.sprt.llr(&a.pent);
    let (elo, lo, hi) = elo_estimate(&a.pent);
    // 閾値到達時点の判定を優先する（最終値は飛行中ペアで希釈されている）
    let decision = a.decision.unwrap_or_else(|| cfg.sprt.decision(llr));
    let label = match decision {
        Decision::AcceptH1 => "H1採択（候補は有意に強い）",
        Decision::AcceptH0 => "H0採択（有意な改善なし）",
        Decision::Continue => "判定に至らず",
    };
    println!(
        "----\n{label} | pairs {} games {} | +{} ={} -{} | Elo {elo:+.1} [{lo:+.1},{hi:+.1}] | LLR {llr:+.2}",
        a.pent.total(),
        a.pent.total() * 2,
        a.wdl[0],
        a.wdl[1],
        a.wdl[2],
    );
    let code = if failed.load(Ordering::Relaxed) {
        3
    } else {
        match decision {
            Decision::AcceptH1 => 0,
            Decision::AcceptH0 => 1,
            Decision::Continue => 2,
        }
    };
    std::process::exit(code);
}
