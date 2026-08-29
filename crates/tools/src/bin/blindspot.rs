//! 盲点ベンチマーク（ADR-0191）。
//!
//! floodgateの実戦で評価が崩壊した局面を集め、深い探索の正解ラベル付きの
//! 測定集合を作る。学習と検収が自己対局で閉じている穴を、実戦の相手が
//! 掘った局面で補う。
//!
//! 使い方:
//!   blindspot extract --dir data/raw/floodgate/2026 --out candidates.tsv
//!
//! 抽出は決定論で、同じ入力からは同じ出力が出る。実戦時の評価値
//! （CSAの `'**` コメント）だけを使い、エンジンは起動しない。

use std::collections::BTreeSet;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use himawari_core::{Position, SFEN_STARTPOS};
use himawari_tools::csa::{self, CsaGame};

#[derive(Parser)]
#[command(about = "盲点ベンチマークの抽出・ラベル・測定（ADR-0191）")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 崩壊局面の候補をCSA群から抽出してTSVへ書く
    Extract {
        /// 棋譜の置き場
        #[arg(long, default_value = "data/raw/floodgate/2026")]
        dir: PathBuf,
        /// 出力TSV
        #[arg(long, default_value = "data/raw/blindspots/candidates.tsv")]
        out: PathBuf,
        /// 自分とみなす対局者名の部分一致
        #[arg(long, default_value = "Himawari")]
        player: String,
        /// 崩壊とみなす評価の落差[cp]
        #[arg(long, default_value_t = 300)]
        drop: i32,
        /// 崩壊前の評価の床[cp]。これ未満から始まる悪化は除く
        #[arg(long, default_value_t = 0)]
        floor: i32,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("エラー: {e:#}");
            ExitCode::from(3)
        }
    }
}

fn run(cli: &Cli) -> Result<()> {
    match &cli.cmd {
        Cmd::Extract {
            dir,
            out,
            player,
            drop,
            floor,
        } => extract(dir, out, player, *drop, *floor),
    }
}

/// 1件の崩壊候補。崩壊前の自分の手番の局面を指す。
struct Candidate {
    sfen: String,
    file: String,
    /// 崩壊前の自分の手番（1始まり）。
    ply: usize,
    eval_before: i32,
    eval_after: i32,
    /// 初期局面からこの局面までのUSI手順。千日手の履歴を保つ。
    moves: String,
}

fn extract(dir: &PathBuf, out: &PathBuf, player: &str, drop: i32, floor: i32) -> Result<()> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("開けません: {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "csa"))
        .collect();
    files.sort();

    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    let (mut games, mut skipped) = (0usize, 0usize);
    for path in &files {
        let text = std::fs::read_to_string(path)?;
        let game = match csa::parse(&text) {
            Ok(g) => g,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        games += 1;
        if let Err(_e) = scan_game(&game, path, player, drop, floor, &mut seen, &mut candidates) {
            skipped += 1;
        }
    }

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut w = std::io::BufWriter::new(std::fs::File::create(out)?);
    writeln!(w, "sfen\tfile\tply\teval_before\teval_after\tmoves")?;
    for c in &candidates {
        writeln!(
            w,
            "{}\t{}\t{}\t{}\t{}\t{}",
            c.sfen, c.file, c.ply, c.eval_before, c.eval_after, c.moves
        )?;
    }
    w.flush()?;
    println!(
        "対局{games}件（読めない棋譜{skipped}件）から候補{}件を{}へ書き出しました",
        candidates.len(),
        out.display()
    );
    Ok(())
}

/// 1局を走査し、自分の評価が次の自分の手番までにdrop以上落ちた対の
/// 崩壊前局面を候補へ足す。
fn scan_game(
    game: &CsaGame,
    path: &std::path::Path,
    player: &str,
    drop: i32,
    floor: i32,
    seen: &mut BTreeSet<String>,
    out: &mut Vec<Candidate>,
) -> Result<()> {
    let Some(me) = game.side_of(player) else {
        return Ok(());
    };
    // 自分の手番のうち評価値が付いている(手index, eval)の列
    let evals: Vec<(usize, i32)> = game
        .moves
        .iter()
        .enumerate()
        .filter(|(_, m)| m.color == me)
        .filter_map(|(i, m)| m.eval_cp.map(|e| (i, e)))
        .collect();

    // 崩壊対を先に決めてから、必要な局面だけ再生する
    let wanted: Vec<(usize, i32, i32)> = evals
        .windows(2)
        .filter(|w| w[0].1 >= floor && w[0].1 - w[1].1 >= drop)
        .map(|w| (w[0].0, w[0].1, w[1].1))
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }

    let mut pos = Position::from_sfen(SFEN_STARTPOS).expect("平手初期局面");
    let mut usi_moves: Vec<String> = Vec::with_capacity(game.moves.len());
    let mut iter = wanted.iter().peekable();
    for (i, m) in game.moves.iter().enumerate() {
        if let Some(&&(idx, before, after)) = iter.peek() {
            if idx == i {
                let sfen = pos.to_sfen();
                if seen.insert(sfen.clone()) {
                    out.push(Candidate {
                        sfen,
                        file: path
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        ply: i + 1,
                        eval_before: before,
                        eval_after: after,
                        moves: usi_moves.join(" "),
                    });
                }
                iter.next();
                if iter.peek().is_none() {
                    break;
                }
            }
        }
        let mv = csa::resolve_move(&pos, m)
            .ok_or_else(|| anyhow::anyhow!("{}手目を解決できない: {}", i + 1, m.text))?;
        pos.do_move(mv);
        usi_moves.push(mv.to_usi());
    }
    Ok(())
}
