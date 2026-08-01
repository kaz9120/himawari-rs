//! 固定深さでNPSを測る（ADR-0081・ADR-0122）。
//!
//! 速度改善の効果は固定深さのノード数では見えない（ADR-0074の機能検証は
//! 「変わらない」を確かめるもの）。同じノード数を何秒で読むかを測る。
//!
//! 局面と深さは `verify` と揃える。局面3だけ枝が広く、同じ深さでは
//! 時間を独占するため深さを3浅くする。

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;

use himawari_tools::positions::{POSITIONS, depth_at};
use himawari_tools::usi_engine::UsiEngine;
use himawari_tools::{
    OrBail, basename, ensure_executable, eval_file, exit, nps, path_str, percent_delta,
    single_thread_options, thousands,
};

#[derive(Parser)]
#[command(
    about = "固定深さでNPSを測る（ADR-0081）",
    long_about = "固定深さでNPSを測る（ADR-0081）。

複数指定すると交互に測る。機体の温度や背景の負荷でNPSは数%動くため、
続けて測って比べる。1本ずつ別々に測った値を比べない。

出力はADRへ転記できるmarkdown表。評価関数は EVAL_FILE か --eval-file で渡す。"
)]
struct Cli {
    /// 測るバイナリ。2本以上を並べると交互に測る
    #[arg(required = true, value_name = "バイナリ")]
    binaries: Vec<PathBuf>,

    /// 探索の深さ（局面3だけ3浅くする）
    #[arg(long, default_value_t = 19, value_parser = clap::value_parser!(u32).range(1..))]
    depth: u32,

    /// 1本を何周測るか
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..))]
    runs: u32,

    /// 1周を打ち切るまでの秒数
    #[arg(long, default_value_t = 600)]
    timeout: u64,

    /// 評価関数。省略時は環境変数 EVAL_FILE
    #[arg(long, value_name = "パス")]
    eval_file: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("エラー: {e:#}");
            ExitCode::from(exit::RUNTIME)
        }
    }
}

fn run(cli: &Cli) -> Result<()> {
    let eval = eval_file(cli.eval_file.clone())?;
    for path in &cli.binaries {
        ensure_executable(path)?;
    }

    println!(
        "=== NPS計測: 深さ {}（局面3は {}）、{}周、1スレッド ===",
        cli.depth,
        depth_at(cli.depth, 2),
        cli.runs
    );
    println!("評価関数: {}", eval.display());
    println!();

    let mut sums = vec![0u64; cli.binaries.len()];
    // 交互に測る。1本ずつまとめて測ると温度差が系統誤差になる
    for run in 1..=cli.runs {
        for (i, path) in cli.binaries.iter().enumerate() {
            let (nodes, ms) = measure(cli, &eval, path)?;
            let speed = nps(nodes, ms);
            println!(
                "  {:<28} run{run}: {} nps（{} nodes / {}ms）",
                basename(path),
                thousands(speed),
                thousands(nodes),
                thousands(ms)
            );
            sums[i] += speed;
        }
    }

    println!();
    println!("| バイナリ | NPS（{}周の平均） | 1本目比 |", cli.runs);
    println!("|---|---|---|");
    let base = sums[0] / u64::from(cli.runs);
    for (i, path) in cli.binaries.iter().enumerate() {
        let avg = sums[i] / u64::from(cli.runs);
        let ratio = if i == 0 {
            "—".to_string()
        } else {
            percent_delta(base as f64, avg as f64, 2)
        };
        println!("| {} | {} | {ratio} |", basename(path), thousands(avg));
    }
    Ok(())
}

/// 1本を1周ぶん測り、(合計ノード数, 合計ミリ秒) を返す。
/// 4局面を1プロセスで続けて読む（TTは局面をまたいで温まる）。
fn measure(cli: &Cli, eval: &std::path::Path, engine: &std::path::Path) -> Result<(u64, u64)> {
    let options = single_thread_options(eval);
    let mut eng = UsiEngine::launch(path_str(engine)?, &options).or_bail()?;
    eng.new_game().or_bail()?;
    let timeout = Duration::from_secs(cli.timeout);

    let mut nodes = 0u64;
    let mut ms = 0u64;
    for (i, pos) in POSITIONS.iter().enumerate() {
        let result = eng
            .go_depth(&format!("position {pos}"), depth_at(cli.depth, i), timeout)
            .or_bail()?;
        // 最後のinfo行がその局面の読み切り時点の累計
        nodes += result.last_info.nodes.unwrap_or(0);
        ms += result.last_info.time_ms.unwrap_or(0);
    }
    eng.quit();
    Ok((nodes, ms))
}
