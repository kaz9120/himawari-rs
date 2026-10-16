//! 探索のプロファイルを取る（ADR-0081・ADR-0099・ADR-0122・ADR-0230）。
//!
//! samplyでサンプリングし、プロファイル本体を書き出す。集計は
//! `hmwr profile report` が行う。局面は `bench` と揃える。深さは25を既定に
//! する。19では探索が112ミリ秒で終わり、サンプルが集まらない。
//!
//! 行番号まで出すにはデバッグ情報が要る。次のようにビルドする。
//!
//! ```text
//! CARGO_PROFILE_RELEASE_DEBUG=1 RUSTFLAGS="-C target-cpu=native" cargo build --release
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Result, bail};
use clap::Parser;

use himawari_tools::positions::{builtin_positions, depth_at, read_positions};
use himawari_tools::usi_engine::UsiEngine;
use himawari_tools::{OrBail, ensure_executable, eval_file, exit, path_str, single_thread_options};

/// サンプリング周波数（Hz）。集計はプロファイルのmeta.intervalから
/// 周波数を読むので、変えても秒への換算はずれない。
const SAMPLE_RATE: &str = "2000";

/// 1局面を打ち切るまでの秒数。
const POSITION_TIMEOUT_SEC: u64 = 600;

/// quit後にsamplyがプロファイルを書き出すのを待つ秒数。
/// ここで打ち切るとプロファイルが残らない。
const SAVE_TIMEOUT_SEC: u64 = 120;

#[derive(Parser)]
#[command(
    about = "samplyで探索のプロファイルを取る（ADR-0099）",
    long_about = "samplyで探索のプロファイルを取る（ADR-0099）。

プロファイル本体を書き出す。集計は hmwr profile report が行い、
samply load <path> でUIからも見られる。

行番号を出すにはデバッグ情報が要る。
CARGO_PROFILE_RELEASE_DEBUG=1 RUSTFLAGS=\"-C target-cpu=native\" cargo build --release

samplyが要る（cargo install samply）。"
)]
struct Cli {
    /// 対象バイナリ
    #[arg(default_value = "target/release/himawari", value_name = "バイナリ")]
    engine: PathBuf,

    /// プロファイルの出力先
    #[arg(
        long,
        default_value = "data/profile/profile.json.gz",
        value_name = "ファイル"
    )]
    out: PathBuf,

    /// 探索の深さ（組み込みの局面3だけ3浅くする）
    #[arg(long, default_value_t = 25, value_parser = clap::value_parser!(u32).range(1..))]
    depth: u32,

    /// 探索スレッド数
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
    threads: u32,

    /// 局面リストのファイル。省略時は組み込みの4局面。
    /// 指定した場合は局面ごとの深さ補正を当てない
    #[arg(long, value_name = "パス")]
    positions: Option<PathBuf>,

    /// samplyの場所。省略時は SAMPLY、PATH、~/.cargo/bin の順で探す
    #[arg(long, value_name = "パス")]
    samply: Option<PathBuf>,

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
    ensure_executable(&cli.engine)?;
    let samply = match &cli.samply {
        Some(p) => p.clone(),
        None => find_samply()
            .ok_or_else(|| anyhow::anyhow!("samplyがない。cargo install samply で入れる"))?,
    };
    ensure_executable(&samply)?;
    if let Some(dir) = cli.out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let positions = match &cli.positions {
        Some(path) => read_positions(path)?,
        None => builtin_positions(),
    };

    match &cli.positions {
        Some(path) => println!(
            "=== プロファイル: 深さ {}、{}スレッド、{}（{}局面） ===",
            cli.depth,
            cli.threads,
            path.display(),
            positions.len()
        ),
        None => println!(
            "=== プロファイル: 深さ {}（局面3は {}）、{}スレッド ===",
            cli.depth,
            depth_at(cli.depth, 2),
            cli.threads
        ),
    }
    println!("対象: {}", cli.engine.display());
    println!();

    record(cli, &samply, &positions, &eval)?;
    if !cli.out.is_file() {
        bail!("プロファイルが書き出されなかった: {}", cli.out.display());
    }
    println!("プロファイル本体: {}", cli.out.display());
    println!("UIで見る: {} load {}", samply.display(), cli.out.display());
    Ok(())
}

/// samplyでエンジンを包んで起動し、局面を順に読ませる。
fn record(cli: &Cli, samply: &Path, positions: &[String], eval: &Path) -> Result<()> {
    let args = [
        "record",
        "--save-only",
        "--unstable-presymbolicate",
        "-r",
        SAMPLE_RATE,
        "-o",
        path_str(&cli.out)?,
        path_str(&cli.engine)?,
    ];
    let mut options = single_thread_options(eval);
    options.push(("Threads".to_string(), cli.threads.to_string()));
    let mut eng = UsiEngine::launch_with_args(path_str(samply)?, &args, &options).or_bail()?;
    eng.new_game().or_bail()?;
    for (i, pos) in positions.iter().enumerate() {
        // 深さ補正は組み込み4局面の枝の広さに合わせたもので、外から
        // 渡した局面には当てはまらない
        let depth = match cli.positions {
            Some(_) => cli.depth,
            None => depth_at(cli.depth, i),
        };
        eng.go_depth(
            &format!("position {pos}"),
            depth,
            Duration::from_secs(POSITION_TIMEOUT_SEC),
        )
        .or_bail()?;
    }
    // samplyはエンジンの終了後にプロファイルを書く。書き終わるまで待つ
    eng.quit_within(Duration::from_secs(SAVE_TIMEOUT_SEC));
    Ok(())
}

/// samplyの場所を探す。SAMPLY、PATH、~/.cargo/bin の順で見る。
fn find_samply() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SAMPLY") {
        return Some(PathBuf::from(path));
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("samply");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let home = std::env::var_os("HOME")?;
    let candidate = PathBuf::from(home).join(".cargo/bin/samply");
    candidate.is_file().then_some(candidate)
}
