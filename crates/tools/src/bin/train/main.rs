//! NNUE学習器v1（教師あり、ADR-0039）。
//!
//! 使い方:
//!   train --data train.psv --out net.hmwr
//!         [--valid valid.psv] [--batch 16384] [--lr 1e-3] [--lambda 0.7]
//!         [--epochs 1] [--seed 1] [--threads N]
//!         [--log-interval 100] [--valid-interval 2000]
//!
//! PSV（ADR-0038）を読み、f32モデルをAdamで学習、量子化して
//! 独自形式（ADR-0037）で書き出す。デコード・順伝播・逆伝播は
//! バッチ内スレッド並列、FT勾配は行領域分割で更新する（parallel.rs）。

mod model;
mod parallel;

use std::io::Read;

use himawari_core::packed_sfen::{PSV_BYTES, PackedSfenValue, unpack};
use himawari_engine::nnue::{effect_active, halfkp_active};
use himawari_engine::nnue_io::save;

use model::{FloatNet, SIGMOID_SCALE, Sample, bce, sigmoid};
use parallel::ParallelTrainer;

fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1)
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.iter().position(|a| a == key).map(|i| {
        args.get(i + 1)
            .unwrap_or_else(|| die(&format!("{key} に値がありません")))
            .clone()
    })
}

fn parse_or<T: std::str::FromStr>(v: Option<String>, default: T) -> T {
    v.and_then(|s| s.parse().ok()).unwrap_or(default)
}

/// PSVレコードをSampleへ変換する。score_limitを超える局面と
/// デコード不能はNone。score_limit=0でフィルタ無効。
pub(crate) fn to_sample(rec: &PackedSfenValue, lambda: f32, score_limit: i16) -> Option<Sample> {
    if score_limit > 0 && rec.score.abs() >= score_limit {
        return None;
    }
    let pos = unpack(&rec.sfen, rec.game_ply).ok()?;
    let stm = pos.side_to_move();
    let mut feats = [Vec::new(), Vec::new()];
    halfkp_active(&pos, stm, &mut feats[0]);
    halfkp_active(&pos, stm.flip(), &mut feats[1]);
    let mut efeats = Vec::new();
    effect_active(&pos, &mut efeats);
    let p_score = sigmoid(f32::from(rec.score) / SIGMOID_SCALE);
    let p_result = (f32::from(rec.game_result) + 1.0) / 2.0;
    Some(Sample {
        feats,
        efeats,
        target: lambda * p_score + (1.0 - lambda) * p_result,
    })
}

fn load_samples(path: &str, lambda: f32, score_limit: i16) -> Vec<Sample> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .unwrap_or_else(|e| die(&format!("開けません: {path}: {e}")))
        .read_to_end(&mut bytes)
        .unwrap_or_else(|e| die(&format!("読み込み失敗: {e}")));
    bytes
        .as_chunks::<PSV_BYTES>()
        .0
        .iter()
        .filter_map(|c| to_sample(&PackedSfenValue::from_bytes(c), lambda, score_limit))
        .collect()
}

fn validate(net: &FloatNet, samples: &[Sample]) -> f32 {
    let mut sum = 0.0f64;
    for s in samples {
        let v = net.forward(s).v;
        sum += f64::from(bce(sigmoid(v), s.target));
    }
    (sum / samples.len().max(1) as f64) as f32
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let data = arg_value(&args, "--data").unwrap_or_else(|| die("--data が必要です"));
    let out = arg_value(&args, "--out").unwrap_or_else(|| die("--out が必要です"));
    let valid_path = arg_value(&args, "--valid");
    let batch: usize = parse_or(arg_value(&args, "--batch"), 16384);
    let lr: f32 = parse_or(arg_value(&args, "--lr"), 1e-3);
    let lambda: f32 = parse_or(arg_value(&args, "--lambda"), 0.7);
    let lr_gamma: f32 = parse_or(arg_value(&args, "--lr-gamma"), 1.0);
    let score_limit: i16 = parse_or(arg_value(&args, "--score-limit"), 0);
    let epochs: u32 = parse_or(arg_value(&args, "--epochs"), 1);
    let seed: u64 = parse_or(arg_value(&args, "--seed"), 1);
    let default_threads = std::thread::available_parallelism()
        .map(|n| n.get().saturating_sub(1).max(1))
        .unwrap_or(1);
    let threads: usize = parse_or(arg_value(&args, "--threads"), default_threads);
    let log_interval: u64 = parse_or(arg_value(&args, "--log-interval"), 100);
    let valid_interval: u64 = parse_or(arg_value(&args, "--valid-interval"), 2000);

    let train_bytes = {
        let mut b = Vec::new();
        std::fs::File::open(&data)
            .unwrap_or_else(|e| die(&format!("開けません: {data}: {e}")))
            .read_to_end(&mut b)
            .unwrap_or_else(|e| die(&format!("読み込み失敗: {e}")));
        b
    };
    let records = train_bytes.as_chunks::<PSV_BYTES>().0;
    if records.is_empty() {
        die("学習データが空です");
    }
    eprintln!(
        "学習データ: {}局面 × {epochs}エポック, batch={batch}, lr={lr}, λ={lambda}, lr_gamma={lr_gamma}, score_limit={score_limit}, threads={threads}",
        records.len()
    );

    let valid = valid_path
        .as_deref()
        .map(|p| load_samples(p, lambda, score_limit));
    if let Some(v) = &valid {
        eprintln!("検証データ: {}局面", v.len());
    }

    let mut trainer =
        ParallelTrainer::new(FloatNet::random(seed), lr, lambda, score_limit, threads);
    let mut step = 0u64;
    let mut samples_done = 0u64;
    let mut skipped = 0u64;
    let mut loss_acc = 0.0f64;
    let mut loss_n = 0u64;
    let t0 = std::time::Instant::now();
    let mut t_log = std::time::Instant::now();
    let mut samples_log = 0u64;

    for epoch in 0..epochs {
        for batch_records in records.chunks(batch) {
            let (lsum, n, skip) = trainer.train_batch(batch_records);
            skipped += skip;
            if n == 0 {
                continue;
            }
            step += 1;
            samples_done += n as u64;
            samples_log += n as u64;
            loss_acc += lsum;
            loss_n += n as u64;

            if step.is_multiple_of(log_interval) {
                let sps = samples_log as f64 / t_log.elapsed().as_secs_f64();
                eprintln!(
                    "step {step} samples {samples_done} loss {:.5} ({:.0} samples/s)",
                    loss_acc / loss_n as f64,
                    sps
                );
                loss_acc = 0.0;
                loss_n = 0;
                t_log = std::time::Instant::now();
                samples_log = 0;
            }
            if let Some(v) = &valid
                && step.is_multiple_of(valid_interval)
            {
                eprintln!("  valid loss {:.5}", validate(&trainer.net, v));
            }
        }
        // エポック末: lr減衰
        trainer.scale_lr(lr_gamma);
        eprintln!("epoch {} 完了 (lr={:.6})", epoch + 1, trainer.current_lr());
    }

    if let Some(v) = &valid {
        eprintln!("最終valid loss {:.5}", validate(&trainer.net, v));
    }
    eprintln!(
        "学習完了: {step}ステップ {samples_done}局面 {:.1}秒 (デコードskip {skipped})",
        t0.elapsed().as_secs_f64()
    );

    let q = trainer.net.quantize();
    let lineage = format!(
        "train-v2 data={data} n={} epochs={epochs} batch={batch} lr={lr} lambda={lambda} lr_gamma={lr_gamma} score_limit={score_limit} seed={seed} steps={step}",
        records.len()
    );
    let mut f =
        std::fs::File::create(&out).unwrap_or_else(|e| die(&format!("作成できません: {e}")));
    save(&q, &lineage, &mut f).unwrap_or_else(|e| die(&format!("書き出し失敗: {e}")));
    println!("{out} を書き出しました ({lineage})");
}
