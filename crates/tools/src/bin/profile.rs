//! 探索のプロファイルを取る（ADR-0081・ADR-0099・ADR-0122）。
//!
//! samplyでサンプリングし、self時間の上位をソース行まで落として出す。
//! 局面は `bench` と揃える。深さは25を既定にする。19では探索が112ミリ秒で
//! 終わり、サンプルが集まらない（深さ25で26050サンプル、約13秒ぶん）。
//!
//! 行番号まで出すにはデバッグ情報が要る。次のようにビルドする。
//!
//! ```text
//! CARGO_PROFILE_RELEASE_DEBUG=1 RUSTFLAGS="-C target-cpu=native" cargo build --release
//! ```

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

use anyhow::{Result, bail};
use clap::Parser;

use himawari_tools::positions::{POSITIONS, depth_at};
use himawari_tools::usi_engine::UsiEngine;
use himawari_tools::{OrBail, ensure_executable, eval_file, exit, path_str, single_thread_options};

/// サンプリング周波数（Hz）。`scripts/profile-report.py` が総サンプル数を
/// 秒へ直すときに同じ値を前提にしている。
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

出力は3つ。
  1. self時間の上位（関数）
  2. self時間の上位（ソース行）
  3. プロファイル本体（samply load <path> でUIから見る）

行番号を出すにはデバッグ情報が要る。
CARGO_PROFILE_RELEASE_DEBUG=1 RUSTFLAGS=\"-C target-cpu=native\" cargo build --release

samplyが要る（cargo install samply）。"
)]
struct Cli {
    /// 対象バイナリ
    #[arg(default_value = "target/release/himawari", value_name = "バイナリ")]
    engine: PathBuf,

    /// プロファイルの出力先ディレクトリ
    #[arg(default_value = "data/profile", value_name = "出力先")]
    out_dir: PathBuf,

    /// 探索の深さ（局面3だけ3浅くする）
    #[arg(long, default_value_t = 25, value_parser = clap::value_parser!(u32).range(1..))]
    depth: u32,

    /// 集計スクリプト
    #[arg(long, default_value = "scripts/profile-report.py", value_name = "パス")]
    report: PathBuf,

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
    std::fs::create_dir_all(&cli.out_dir)?;
    let profile = cli.out_dir.join("profile.json.gz");

    println!(
        "=== プロファイル: 深さ {}（局面3は {}）、1スレッド ===",
        cli.depth,
        depth_at(cli.depth, 2)
    );
    println!("対象: {}", cli.engine.display());
    println!();

    record(cli, &samply, &profile, &eval)?;
    if !profile.is_file() {
        bail!("プロファイルが書き出されなかった: {}", profile.display());
    }

    println!();
    let status = Command::new("python3")
        .arg(&cli.report)
        .arg(&profile)
        .arg(&cli.engine)
        .status()?;
    if !status.success() {
        bail!("{} が失敗した ({status})", cli.report.display());
    }
    println!();
    println!("プロファイル本体: {}", profile.display());
    println!("UIで見る: {} load {}", samply.display(), profile.display());
    Ok(())
}

/// samplyでエンジンを包んで起動し、4局面を読ませる。
fn record(cli: &Cli, samply: &Path, profile: &Path, eval: &Path) -> Result<()> {
    let args = [
        "record",
        "--save-only",
        "--unstable-presymbolicate",
        "-r",
        SAMPLE_RATE,
        "-o",
        path_str(profile)?,
        path_str(&cli.engine)?,
    ];
    let options = single_thread_options(eval);
    let mut eng = UsiEngine::launch_with_args(path_str(samply)?, &args, &options).or_bail()?;
    eng.new_game().or_bail()?;
    for (i, pos) in POSITIONS.iter().enumerate() {
        eng.go_depth(
            &format!("position {pos}"),
            depth_at(cli.depth, i),
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
