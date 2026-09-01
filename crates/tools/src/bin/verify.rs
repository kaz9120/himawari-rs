//! 機能検証（ADR-0074・ADR-0122）。固定深さでのノード数を変更前後で比べる。
//!
//! 局面を毎回書き下すと条件がぶれるため、局面と深さを固定する。
//! ADRへ転記できる形（markdown表）で出力する。
//!
//! 全局面でノード数が一致したら終了コード1を返す。その変更は探索に
//! 影響しておらず、SPRTにかけても中立にしかならない。

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Result, anyhow};
use clap::Parser;

use himawari_tools::positions::{builtin_positions, read_positions};
use himawari_tools::usi_engine::UsiEngine;
use himawari_tools::{
    OrBail, ensure_executable, eval_file, exit, path_str, percent_delta, single_thread_options,
    thousands,
};

#[derive(Parser)]
#[command(
    about = "機能検証（ADR-0074）。固定深さのノード数を比べる",
    long_about = "機能検証（ADR-0074）。固定深さのノード数を比べる。

candidateを省くとbaselineだけを測る。局面は初期局面と
openings/start_sfens_ply24.txt の先頭3行の計4つで、ADR-0074の
「3局面以上」を満たす。

全局面でノード数が一致したら終了コード1を返す。その変更は探索に
影響していない。評価関数は EVAL_FILE か --eval-file で渡す。"
)]
struct Cli {
    /// 変更前のバイナリ
    #[arg(value_name = "baseline")]
    baseline: PathBuf,

    /// 変更後のバイナリ。省略するとbaselineだけを測る
    #[arg(value_name = "candidate")]
    candidate: Option<PathBuf>,

    /// 探索の深さ
    #[arg(long, default_value_t = 13, value_parser = clap::value_parser!(u32).range(1..))]
    depth: u32,

    /// 1局面を打ち切るまでの秒数
    #[arg(long, default_value_t = 300)]
    timeout: u64,

    /// 評価関数。省略時は環境変数 EVAL_FILE
    #[arg(long, value_name = "パス")]
    eval_file: Option<PathBuf>,

    /// 局面リストのファイル。省略時は組み込みの4局面
    #[arg(long, value_name = "パス")]
    positions: Option<PathBuf>,
}

/// 1局面の測定結果。
struct Measured {
    nodes: u64,
    /// USI表記のscore（"cp 123" / "mate 5"）。
    score: String,
    bestmove: String,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("エラー: {e:#}");
            ExitCode::from(exit::RUNTIME)
        }
    }
}

fn run(cli: &Cli) -> Result<u8> {
    let eval = eval_file(cli.eval_file.clone())?;
    ensure_executable(&cli.baseline)?;
    if let Some(candidate) = &cli.candidate {
        ensure_executable(candidate)?;
    }

    let positions = match &cli.positions {
        Some(path) => read_positions(path)?,
        None => builtin_positions(),
    };

    println!("=== 機能検証（ADR-0074）: 固定深さ {} ===", cli.depth);
    println!("評価関数: {}", eval.display());
    println!("baseline : {}", cli.baseline.display());
    if let Some(candidate) = &cli.candidate {
        println!("candidate: {}", candidate.display());
    }
    if let Some(path) = &cli.positions {
        println!("局面     : {}（{}局面）", path.display(), positions.len());
    }
    println!();

    let Some(candidate) = &cli.candidate else {
        println!("| 局面 | ノード数 | 評価値 | 最善手 |");
        println!("|---|---|---|---|");
        for (i, pos) in positions.iter().enumerate() {
            let m = measure(cli, &eval, &cli.baseline, pos)?;
            println!(
                "| {} | {} | {} | {} |",
                i + 1,
                thousands(m.nodes),
                m.score,
                m.bestmove
            );
        }
        return Ok(0);
    };

    let mut same = true;
    println!("| 局面 | 変更前 | 変更後 | 変化 | 評価値 | 最善手 |");
    println!("|---|---|---|---|---|---|");
    for (i, pos) in positions.iter().enumerate() {
        // 局面ごとに交互へ測る。並びを変えると温度差が片方に乗る
        let base = measure(cli, &eval, &cli.baseline, pos)?;
        let cand = measure(cli, &eval, candidate, pos)?;
        let pct = percent_delta(base.nodes as f64, cand.nodes as f64, 0);
        if base.nodes != cand.nodes {
            same = false;
        }
        let score_cell = if base.score == cand.score {
            base.score.clone()
        } else {
            format!("{} → {}", base.score, cand.score)
        };
        let move_cell = if base.bestmove == cand.bestmove {
            "同じ".to_string()
        } else {
            format!("{} → {}", base.bestmove, cand.bestmove)
        };
        println!(
            "| {} | {} | {} | {pct} | {score_cell} | {move_cell} |",
            i + 1,
            thousands(base.nodes),
            thousands(cand.nodes)
        );
    }

    println!();
    if same {
        println!("全局面でノード数が一致した。この変更は探索に影響していない（ADR-0074）。");
        println!("SPRTにかけても中立にしかならない。");
        return Ok(exit::JUDGEMENT);
    }
    println!("ノード数が変わった。探索に影響している。SPRTへ進んでよい。");
    Ok(0)
}

/// 1局面を1エンジンで読む。局面ごとにプロセスを立て直し、TTを持ち越さない。
/// ノード数の比較は状態がそろっていないと意味がない。
fn measure(cli: &Cli, eval: &Path, engine: &Path, position: &str) -> Result<Measured> {
    let options = single_thread_options(eval);
    let mut eng = UsiEngine::launch(path_str(engine)?, &options).or_bail()?;
    eng.new_game().or_bail()?;
    let result = eng
        .go_depth(
            &format!("position {position}"),
            cli.depth,
            Duration::from_secs(cli.timeout),
        )
        .or_bail()?;
    eng.quit();

    // 指定深さへ届かずに探索が終わると比較できない（詰み発見など）
    let info = result.target_depth_info.ok_or_else(|| {
        anyhow!(
            "深さ{}のinfo行がない: {position}（{}）",
            cli.depth,
            engine.display()
        )
    })?;
    Ok(Measured {
        nodes: info.nodes.unwrap_or(0),
        score: info.score.unwrap_or_default(),
        bestmove: result.bestmove,
    })
}
