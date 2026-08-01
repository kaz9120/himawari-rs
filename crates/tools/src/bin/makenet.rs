//! NNUEネット生成・変換ツール（ADR-0037）。
//!
//! 使い方:
//!   makenet [--seed N] [--out path]           乱数ネット生成（配線検証・ベンチ用）
//!   makenet --import nn.bin [--out path]      やねうら王形式HalfKPネットを
//!                                             独自形式へ変換（利き塔ゼロ）
//!   makenet --expand small.hmwr [--out path]  小さい構成のネットを、いまの
//!                                             ビルド構成へゼロ埋めで広げる
//!
//! 学習パイプライン（P5）ができるまでの検証用。

use himawari_engine::nnue::NnueNetwork;
use himawari_engine::nnue_io::{load_expanding, save};

/// nn.bin（やねうら王形式）を読む。FT 256専用（ADR-0067）。
fn import_net(path: &str) -> (NnueNetwork, String) {
    let mut f = std::fs::File::open(path).unwrap_or_else(|e| {
        eprintln!("開けません: {path}: {e}");
        std::process::exit(1);
    });
    let (net, arch) = himawari_engine::nnue_compat::load_nn_bin(&mut f).unwrap_or_else(|e| {
        eprintln!("nn.bin読み込み失敗: {e}");
        std::process::exit(1);
    });
    (net, format!("imported from {path} ({arch})"))
}

/// 小さい構成のネットを、いまのビルド構成へ広げる（ADR-0127）。
/// 評価値は元のネットと完全に一致するので、構成だけを変えて探索木を
/// 揃えた速度比較ができる。
fn expand_net(path: &str) -> (NnueNetwork, String) {
    let mut f = std::fs::File::open(path).unwrap_or_else(|e| {
        eprintln!("開けません: {path}: {e}");
        std::process::exit(1);
    });
    load_expanding(&mut f).unwrap_or_else(|e| {
        eprintln!("拡張に失敗: {e}");
        std::process::exit(1);
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut seed = 1u64;
    let mut out = "random.hmwr".to_string();
    let mut import: Option<String> = None;
    let mut expand: Option<String> = None;
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
            "--expand" => {
                i += 1;
                expand = args.get(i).cloned();
                if expand.is_none() {
                    eprintln!("--expand には元になる.hmwrのパスが必要です");
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
    let (net, lineage) = match (&import, &expand) {
        (Some(_), Some(_)) => {
            eprintln!("--import と --expand は同時に指定できません");
            std::process::exit(1);
        }
        (Some(path), None) => import_net(path),
        (None, Some(path)) => expand_net(path),
        (None, None) => (NnueNetwork::random(seed), format!("random seed={seed}")),
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
