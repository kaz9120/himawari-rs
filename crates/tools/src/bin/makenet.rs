//! 開発用の乱数NNUEネット生成ツール（ADR-0037）。
//!
//! 使い方: makenet [--seed N] [--out path]
//! 学習パイプライン（P5）ができるまでの配線検証・ベンチ用。

use himawari_engine::nnue::NnueNetwork;
use himawari_engine::nnue_io::save;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut seed = 1u64;
    let mut out = "random.hmwr".to_string();
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
            other => {
                eprintln!("不明な引数: {other}");
                std::process::exit(1);
            }
        }
        i += 1;
    }
    let net = NnueNetwork::random(seed);
    let mut f = std::fs::File::create(&out).unwrap_or_else(|e| {
        eprintln!("作成できません: {e}");
        std::process::exit(1);
    });
    save(&net, &format!("random seed={seed}"), &mut f).unwrap_or_else(|e| {
        eprintln!("書き出し失敗: {e}");
        std::process::exit(1);
    });
    println!("{out} を書き出しました (seed={seed})");
}
