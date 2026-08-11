//! NNUEネット生成・変換ツール（ADR-0037）。
//!
//! 使い方:
//!   makenet [--seed N] [--out path]           乱数ネット生成（配線検証・ベンチ用）
//!   makenet --resize other.hmwr [--out path]  別の構成のネットを、いまの
//!                                             ビルド構成へ合わせる
//!
//! やねうら王形式（nn.bin）の取り込みは、玉位置を45バケットへ畳んだ
//! 時点で意味を失ったので落とした（ADR-0157）。
//!
//! 学習パイプライン（P5）ができるまでの検証用。

use himawari_engine::nnue::NnueNetwork;
use himawari_engine::nnue_io::{load_resized, save};

/// 別の構成のネットを、いまのビルド構成へ合わせる（ADR-0127）。
/// 広げる向きなら評価値が完全に一致するので、構成だけを変えて探索木を
/// 揃えた速度比較ができる。
fn resize_net(path: &str) -> (NnueNetwork, String) {
    let mut f = std::fs::File::open(path).unwrap_or_else(|e| {
        eprintln!("開けません: {path}: {e}");
        std::process::exit(1);
    });
    load_resized(&mut f).unwrap_or_else(|e| {
        eprintln!("構成の変換に失敗: {e}");
        std::process::exit(1);
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut seed = 1u64;
    let mut out = "random.hmwr".to_string();
    let mut resize: Option<String> = None;
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
            "--resize" => {
                i += 1;
                resize = args.get(i).cloned();
                if resize.is_none() {
                    eprintln!("--resize には元になる.hmwrのパスが必要です");
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
    let (net, lineage) = match &resize {
        Some(path) => resize_net(path),
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
