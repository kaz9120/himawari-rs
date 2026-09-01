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

use himawari_tools::positions::{builtin_positions, depth_at, read_positions};
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
    /// 測るバイナリ。2本以上を並べると交互に測る。
    /// `パス=評価ファイル` と書くと、そのバイナリだけ別の評価関数で測る
    /// （ネットワーク構成ごとにネットが違うとき。ADR-0127）
    #[arg(required = true, value_name = "バイナリ[=評価ファイル]")]
    binaries: Vec<String>,

    /// 探索の深さ（局面3だけ3浅くする）
    #[arg(long, default_value_t = 19, value_parser = clap::value_parser!(u32).range(1..))]
    depth: u32,

    /// 深さの代わりにノード数で打ち切る。評価関数が違うと同じ深さでも
    /// 探索木の大きさが変わるため、評価関数をまたいで比べるときに使う
    /// （ADR-0127）
    #[arg(long, value_name = "ノード数", conflicts_with = "depth")]
    nodes: Option<u64>,

    /// 1本を何周測るか
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..))]
    runs: u32,

    /// 1周を打ち切るまでの秒数
    #[arg(long, default_value_t = 600)]
    timeout: u64,

    /// 評価関数。省略時は環境変数 EVAL_FILE
    #[arg(long, value_name = "パス")]
    eval_file: Option<PathBuf>,

    /// 局面リストのファイル。省略時は組み込みの4局面。
    /// 指定した場合は局面ごとの深さ補正を当てない
    #[arg(long, value_name = "パス")]
    positions: Option<PathBuf>,
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

/// 測る対象1本。評価関数はバイナリごとに変えられる。
struct Target {
    bin: PathBuf,
    eval: PathBuf,
}

/// `パス` または `パス=評価ファイル` を解く。
/// 評価ファイルを書かなかったバイナリは共通の指定（--eval-file か
/// 環境変数EVAL_FILE）を使う。
fn targets(cli: &Cli) -> Result<Vec<Target>> {
    // 全部に個別指定があるときは共通の評価関数を要求しない
    let needs_default = cli.binaries.iter().any(|s| !s.contains('='));
    let default_eval = if needs_default {
        Some(eval_file(cli.eval_file.clone())?)
    } else {
        None
    };
    let mut out = Vec::with_capacity(cli.binaries.len());
    for spec in &cli.binaries {
        let (bin, eval) = match spec.split_once('=') {
            Some((b, e)) => (PathBuf::from(b), eval_file(Some(PathBuf::from(e)))?),
            None => (PathBuf::from(spec), default_eval.clone().unwrap()),
        };
        ensure_executable(&bin)?;
        out.push(Target { bin, eval });
    }
    Ok(out)
}

fn run(cli: &Cli) -> Result<()> {
    let targets = targets(cli)?;
    let positions = match &cli.positions {
        Some(path) => read_positions(path)?,
        None => builtin_positions(),
    };

    match cli.nodes {
        Some(n) => println!(
            "=== NPS計測: {}ノード、{}周、1スレッド ===",
            thousands(n),
            cli.runs
        ),
        None if cli.positions.is_some() => println!(
            "=== NPS計測: 深さ {}、{}周、1スレッド ===",
            cli.depth, cli.runs
        ),
        None => println!(
            "=== NPS計測: 深さ {}（局面3は {}）、{}周、1スレッド ===",
            cli.depth,
            depth_at(cli.depth, 2),
            cli.runs
        ),
    }
    if let Some(path) = &cli.positions {
        println!("局面: {}（{}局面）", path.display(), positions.len());
    }
    for t in &targets {
        println!("{} の評価関数: {}", basename(&t.bin), t.eval.display());
    }
    println!();

    let mut sums = vec![0u64; targets.len()];
    // 交互に測る。1本ずつまとめて測ると温度差が系統誤差になる
    for run in 1..=cli.runs {
        for (i, t) in targets.iter().enumerate() {
            let (nodes, ms) = measure(cli, &positions, &t.eval, &t.bin)?;
            let speed = nps(nodes, ms);
            println!(
                "  {:<28} run{run}: {} nps（{} nodes / {}ms）",
                basename(&t.bin),
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
    for (i, t) in targets.iter().enumerate() {
        let avg = sums[i] / u64::from(cli.runs);
        let ratio = if i == 0 {
            "—".to_string()
        } else {
            percent_delta(base as f64, avg as f64, 2)
        };
        println!("| {} | {} | {ratio} |", basename(&t.bin), thousands(avg));
    }
    Ok(())
}

/// 1本を1周ぶん測り、(合計ノード数, 合計ミリ秒) を返す。
/// 4局面を1プロセスで続けて読む（TTは局面をまたいで温まる）。
fn measure(
    cli: &Cli,
    positions: &[String],
    eval: &std::path::Path,
    engine: &std::path::Path,
) -> Result<(u64, u64)> {
    let options = single_thread_options(eval);
    let mut eng = UsiEngine::launch(path_str(engine)?, &options).or_bail()?;
    eng.new_game().or_bail()?;
    let timeout = Duration::from_secs(cli.timeout);

    let mut nodes = 0u64;
    let mut ms = 0u64;
    for (i, pos) in positions.iter().enumerate() {
        let cmd = format!("position {pos}");
        // 深さ補正は組み込み4局面の枝の広さに合わせたもので、外から
        // 渡した局面には当てはまらない
        let depth = match cli.positions {
            Some(_) => cli.depth,
            None => depth_at(cli.depth, i),
        };
        let result = match cli.nodes {
            Some(n) => eng
                .think(&cmd, &format!("go nodes {n}"), timeout)
                .or_bail()?,
            None => eng.go_depth(&cmd, depth, timeout).or_bail()?,
        };
        // 最後のinfo行がその局面の読み切り時点の累計
        nodes += result.last_info.nodes.unwrap_or(0);
        ms += result.last_info.time_ms.unwrap_or(0);
    }
    eng.quit();
    Ok((nodes, ms))
}
