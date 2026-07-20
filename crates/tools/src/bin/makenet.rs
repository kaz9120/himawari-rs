//! NNUEネット生成・変換ツール（ADR-0037）。
//!
//! 使い方:
//!   makenet [--seed N] [--out path]           乱数ネット生成（配線検証・ベンチ用）
//!   makenet --import nn.bin [--out path]      やねうら王形式HalfKPネットを
//!                                             独自形式へ変換（利き塔ゼロ）
//!
//! 学習パイプライン（P5）ができるまでの検証用。

use himawari_engine::nnue::NnueNetwork;
use himawari_engine::nnue_compat::load_nn_bin;
use himawari_engine::nnue_io::save;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut seed = 1u64;
    let mut out = "random.hmwr".to_string();
    let mut import: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--seed" => {
                i += 1;
                seed = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(1);
            }
            "--out" => {
                i += 1;
                out = args.get(i).cloned().unwrap_or(out);
            }
            "--import" => {
                i += 1;
                import = args.get(i).cloned();
                if import.is_none() {
                    eprintln!("--import にはnn.binのパスが必要です");
                    std::process::exit(1);
                }
            }
            other => {
                eprintln!("不明な引数: {other}");
                std::process::exit(1);
            }
        }
        i += 1;
    }
    let (net, lineage) = match &import {
        Some(path) => {
            let mut f = std::fs::File::open(path).unwrap_or_else(|e| {
                eprintln!("開けません: {path}: {e}");
                std::process::exit(1);
            });
            let (net, arch) = load_nn_bin(&mut f).unwrap_or_else(|e| {
                eprintln!("nn.bin読み込み失敗: {e}");
                std::process::exit(1);
            });
            (net, format!("imported from {path} ({arch})"))
        }
        None => (NnueNetwork::random(seed), format!("random seed={seed}")),
    };
    let mut f = std::fs::File::create(&out).unwrap_or_else(|e| {
        eprintln!("作成できません: {e}");
        std::process::exit(1);
    });
    save(&net, &lineage, &mut f).unwrap_or_else(|e| {
        eprintln!("書き出し失敗: {e}");
        std::process::exit(1);
    });
    println!("{out} を書き出しました ({lineage})");
}
