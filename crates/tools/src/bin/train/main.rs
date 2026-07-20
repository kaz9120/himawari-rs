//! NNUE学習器v1（教師あり、ADR-0039）。
//!
//! 使い方:
//!   train --data train.psv --out net.hmwr
//!         [--valid valid.psv] [--batch 8192] [--lr 1e-3] [--lambda 0.7]
//!         [--epochs 1] [--seed 1] [--log-interval 100] [--valid-interval 2000]
//!
//! PSV（ADR-0038）をストリーミング読みし、f32モデルをAdamで学習、
//! 量子化して独自形式（ADR-0037）で書き出す。デコードと特徴抽出は
//! 別スレッドで行い、学習スレッドとパイプラインにする。

mod model;

use std::io::Read;
use std::sync::mpsc;

use himawari_core::packed_sfen::{PSV_BYTES, PackedSfenValue, unpack};
use himawari_engine::nnue::{effect_active, halfkp_active};
use himawari_engine::nnue_io::save;

use model::{Adam, FloatNet, Grads, SIGMOID_SCALE, Sample, bce, sigmoid};

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

/// PSVレコードをSampleへ変換する。デコード不能はNone。
fn to_sample(rec: &PackedSfenValue, lambda: f32) -> Option<Sample> {
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

fn load_samples(path: &str, lambda: f32) -> Vec<Sample> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .unwrap_or_else(|e| die(&format!("開けません: {path}: {e}")))
        .read_to_end(&mut bytes)
        .unwrap_or_else(|e| die(&format!("読み込み失敗: {e}")));
    bytes
        .as_chunks::<PSV_BYTES>()
        .0
        .iter()
        .filter_map(|c| to_sample(&PackedSfenValue::from_bytes(c), lambda))
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
    let batch: usize = parse_or(arg_value(&args, "--batch"), 8192);
    let lr: f32 = parse_or(arg_value(&args, "--lr"), 1e-3);
    let lambda: f32 = parse_or(arg_value(&args, "--lambda"), 0.7);
    let epochs: u32 = parse_or(arg_value(&args, "--epochs"), 1);
    let seed: u64 = parse_or(arg_value(&args, "--seed"), 1);
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
    let n_records = train_bytes.len() / PSV_BYTES;
    if n_records == 0 {
        die("学習データが空です");
    }
    eprintln!("学習データ: {n_records}局面 × {epochs}エポック, batch={batch}, lr={lr}, λ={lambda}");

    let valid = valid_path.as_deref().map(|p| load_samples(p, lambda));
    if let Some(v) = &valid {
        eprintln!("検証データ: {}局面", v.len());
    }

    // デコードスレッド: PSV→Sampleのバッチを学習スレッドへ送る
    let (tx, rx) = mpsc::sync_channel::<Vec<Sample>>(4);
    let decode_lambda = lambda;
    let decoder = std::thread::spawn(move || {
        let mut skipped = 0u64;
        for _ in 0..epochs {
            let mut buf = Vec::with_capacity(batch);
            for c in train_bytes.as_chunks::<PSV_BYTES>().0 {
                let rec = PackedSfenValue::from_bytes(c);
                match to_sample(&rec, decode_lambda) {
                    Some(s) => buf.push(s),
                    None => skipped += 1,
                }
                if buf.len() == batch {
                    if tx.send(std::mem::take(&mut buf)).is_err() {
                        return skipped;
                    }
                    buf = Vec::with_capacity(batch);
                }
            }
            if !buf.is_empty() && tx.send(buf).is_err() {
                return skipped;
            }
        }
        skipped
    });

    let mut net = FloatNet::random(seed);
    let mut adam = Adam::new(lr);
    let mut g = Grads::new();
    let mut step = 0u64;
    let mut samples_done = 0u64;
    let mut loss_acc = 0.0f64;
    let mut loss_n = 0u64;
    let t0 = std::time::Instant::now();
    let mut t_log = std::time::Instant::now();
    let mut samples_log = 0u64;

    while let Ok(batch_samples) = rx.recv() {
        let bs = batch_samples.len();
        let mut sum = 0.0f64;
        for s in &batch_samples {
            let act = net.forward(s);
            sum += f64::from(net.backward(s, &act, &mut g));
        }
        adam.step(&mut net, &g, 1.0 / bs as f32);
        g.clear();

        step += 1;
        samples_done += bs as u64;
        samples_log += bs as u64;
        loss_acc += sum;
        loss_n += bs as u64;

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
            eprintln!("  valid loss {:.5}", validate(&net, v));
        }
    }
    let skipped = decoder.join().unwrap_or(0);

    if let Some(v) = &valid {
        eprintln!("最終valid loss {:.5}", validate(&net, v));
    }
    eprintln!(
        "学習完了: {step}ステップ {samples_done}局面 {:.1}秒 (デコードskip {skipped})",
        t0.elapsed().as_secs_f64()
    );

    let q = net.quantize();
    let lineage = format!(
        "train-v1 data={data} n={n_records} epochs={epochs} batch={batch} lr={lr} lambda={lambda} seed={seed} steps={step}"
    );
    let mut f =
        std::fs::File::create(&out).unwrap_or_else(|e| die(&format!("作成できません: {e}")));
    save(&q, &lineage, &mut f).unwrap_or_else(|e| die(&format!("書き出し失敗: {e}")));
    println!("{out} を書き出しました ({lineage})");
}
