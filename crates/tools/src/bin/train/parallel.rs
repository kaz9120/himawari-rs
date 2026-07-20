//! バッチ並列の学習実行（ADR-0039のスループット改善）。
//!
//! 2相方式で1バッチを処理する。相1では各スレッドがバッチの
//! 担当分をデコード・順伝播・逆伝播し、密勾配（スレッドローカル）と
//! FT・利き塔の行勾配参照（行番号と共通勾配ベクトルの組）を作る。
//! 相2ではFT行空間を連続領域に分割し、各スレッドが自領域の行だけを
//! スラブに積んでそのままAdam更新する。領域が互いに素なのでロックは
//! 不要で、巨大な勾配バッファのゼロクリアも発生しない。

use himawari_core::packed_sfen::{PSV_BYTES, PackedSfenValue};
use himawari_engine::nnue::{EFFECT_IN, EFFECT_OUT, FT_IN, FT_OUT};

use crate::model::{Adam, DenseGrads, FloatNet, Sample};
use crate::to_sample;

/// 相1の1スレッド分の出力。
struct WorkerOut {
    dense: DenseGrads,
    /// サンプル×半盤ごとの共通勾配ベクトル。
    gz1s: Vec<[f32; FT_OUT]>,
    /// (FT行, gz1sインデックス)。
    ft_refs: Vec<(u32, u32)>,
    gzes: Vec<[f32; EFFECT_OUT]>,
    ef_refs: Vec<(u16, u32)>,
    loss: f64,
    n: usize,
    skipped: u64,
}

impl WorkerOut {
    fn new() -> WorkerOut {
        WorkerOut {
            dense: DenseGrads::new(),
            gz1s: Vec::new(),
            ft_refs: Vec::new(),
            gzes: Vec::new(),
            ef_refs: Vec::new(),
            loss: 0.0,
            n: 0,
            skipped: 0,
        }
    }
}

/// 1サンプルの順伝播＋逆伝播をWorkerOutへ積む。
fn accumulate(net: &FloatNet, s: &Sample, out: &mut WorkerOut) {
    let act = net.forward(s);
    let b = net.backward_dense(s, &act, &mut out.dense);
    out.loss += f64::from(b.loss);
    out.n += 1;
    for half in 0..2 {
        if b.gz1_any[half] {
            let gi = out.gz1s.len() as u32;
            out.gz1s.push(b.gz1[half]);
            for &f in &s.feats[half] {
                out.ft_refs.push((f, gi));
            }
        }
    }
    if b.gze_any {
        let gi = out.gzes.len() as u32;
        out.gzes.push(b.gze);
        for &f in &s.efeats {
            out.ef_refs.push((f, gi));
        }
    }
}

/// 可変スライスをregion_lenごとの連続領域に分割する。
fn split_regions(mut s: &mut [f32], region_len: usize) -> Vec<&mut [f32]> {
    let mut v = Vec::new();
    while s.len() > region_len {
        let (a, b) = s.split_at_mut(region_len);
        v.push(a);
        s = b;
    }
    v.push(s);
    v
}

pub struct ParallelTrainer {
    pub net: FloatNet,
    adam: Adam,
    threads: usize,
    lambda: f32,
    /// FT領域ごとの勾配スラブ（touched行だけ書き、更新後にゼロへ戻す）。
    slabs: Vec<Vec<f32>>,
    eslab: Vec<f32>,
    dsum: DenseGrads,
}

impl ParallelTrainer {
    pub fn new(net: FloatNet, lr: f32, lambda: f32, threads: usize) -> ParallelTrainer {
        let threads = threads.max(1);
        let rows_per = FT_IN.div_ceil(threads);
        ParallelTrainer {
            net,
            adam: Adam::new(lr),
            threads,
            lambda,
            slabs: (0..threads).map(|_| vec![0.0; rows_per * FT_OUT]).collect(),
            eslab: vec![0.0; EFFECT_IN * EFFECT_OUT],
            dsum: DenseGrads::new(),
        }
    }

    /// 生PSVレコード列を1バッチとして学習する。
    /// 戻り値は (損失合計, 学習サンプル数, デコードskip数)。
    pub fn train_batch(&mut self, records: &[[u8; PSV_BYTES]]) -> (f64, usize, u64) {
        let t = self.threads;
        let lambda = self.lambda;
        let net = &self.net;
        let chunk = records.len().div_ceil(t).max(1);
        let workers: Vec<WorkerOut> = std::thread::scope(|s| {
            let handles: Vec<_> = records
                .chunks(chunk)
                .map(|recs| {
                    s.spawn(move || {
                        let mut out = WorkerOut::new();
                        for raw in recs {
                            let rec = PackedSfenValue::from_bytes(raw);
                            match to_sample(&rec, lambda) {
                                Some(sample) => accumulate(net, &sample, &mut out),
                                None => out.skipped += 1,
                            }
                        }
                        out
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("学習ワーカスレッド"))
                .collect()
        });
        self.apply(workers)
    }

    /// 展開済みサンプル列を1バッチとして学習する（テスト用）。
    #[cfg(test)]
    pub fn train_samples(&mut self, samples: &[Sample]) -> (f64, usize, u64) {
        let t = self.threads;
        let net = &self.net;
        let chunk = samples.len().div_ceil(t).max(1);
        let workers: Vec<WorkerOut> = std::thread::scope(|s| {
            let handles: Vec<_> = samples
                .chunks(chunk)
                .map(|ss| {
                    s.spawn(move || {
                        let mut out = WorkerOut::new();
                        for sample in ss {
                            accumulate(net, sample, &mut out);
                        }
                        out
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("学習ワーカスレッド"))
                .collect()
        });
        self.apply(workers)
    }

    /// 相2: 勾配を集約してAdamで1ステップ更新する。
    fn apply(&mut self, workers: Vec<WorkerOut>) -> (f64, usize, u64) {
        let n: usize = workers.iter().map(|w| w.n).sum();
        let loss: f64 = workers.iter().map(|w| w.loss).sum();
        let skipped: u64 = workers.iter().map(|w| w.skipped).sum();
        if n == 0 {
            return (loss, 0, skipped);
        }
        let scale = 1.0 / n as f32;

        let (bc1, bc2) = self.adam.begin_step();

        // 密パラメータ（合算して単スレッドで更新。総量は小さい）
        self.dsum.clear();
        for w in &workers {
            self.dsum.add(&w.dense);
        }
        self.adam
            .step_dense(&mut self.net, &self.dsum, scale, bc1, bc2);

        // FT行: 連続領域に分割し、各スレッドが自領域を積んで更新する
        let hyper = self.adam.hyper();
        let rows_per = FT_IN.div_ceil(self.threads);
        let region_len = rows_per * FT_OUT;
        let (m_ft, v_ft) = self.adam.ft_moments_mut();
        let w_regions = split_regions(&mut self.net.ft_w, region_len);
        let m_regions = split_regions(m_ft, region_len);
        let v_regions = split_regions(v_ft, region_len);
        let workers_ref = &workers;
        std::thread::scope(|s| {
            for (idx, ((wreg, mreg), (vreg, slab))) in w_regions
                .into_iter()
                .zip(m_regions)
                .zip(v_regions.into_iter().zip(self.slabs.iter_mut()))
                .enumerate()
            {
                s.spawn(move || {
                    let lo = idx * rows_per;
                    let rows_here = wreg.len() / FT_OUT;
                    let mut touched: Vec<u32> = Vec::new();
                    for w in workers_ref {
                        for &(row, gi) in &w.ft_refs {
                            let row = row as usize;
                            if row < lo || row >= lo + rows_here {
                                continue;
                            }
                            let rel = row - lo;
                            let dst = &mut slab[rel * FT_OUT..(rel + 1) * FT_OUT];
                            for (d, g) in dst.iter_mut().zip(&w.gz1s[gi as usize]) {
                                *d += g;
                            }
                            touched.push(rel as u32);
                        }
                    }
                    touched.sort_unstable();
                    touched.dedup();
                    for &rel in &touched {
                        let r = rel as usize * FT_OUT..(rel as usize + 1) * FT_OUT;
                        Adam::step_row(
                            hyper,
                            (bc1, bc2),
                            &mut wreg[r.clone()],
                            &slab[r.clone()],
                            &mut mreg[r.clone()],
                            &mut vreg[r.clone()],
                            scale,
                        );
                        slab[r].fill(0.0);
                    }
                });
            }
        });

        // 利き塔行: 行数が少ないので単スレッドで同じ方式
        let (m_ef, v_ef) = self.adam.ef_moments_mut();
        let mut touched: Vec<u16> = Vec::new();
        for w in &workers {
            for &(row, gi) in &w.ef_refs {
                let rel = row as usize;
                let dst = &mut self.eslab[rel * EFFECT_OUT..(rel + 1) * EFFECT_OUT];
                for (d, g) in dst.iter_mut().zip(&w.gzes[gi as usize]) {
                    *d += g;
                }
                touched.push(row);
            }
        }
        touched.sort_unstable();
        touched.dedup();
        for &rel in &touched {
            let r = rel as usize * EFFECT_OUT..(rel as usize + 1) * EFFECT_OUT;
            Adam::step_row(
                hyper,
                (bc1, bc2),
                &mut self.net.ef_w[r.clone()],
                &self.eslab[r.clone()],
                &mut m_ef[r.clone()],
                &mut v_ef[r.clone()],
                scale,
            );
            self.eslab[r].fill(0.0);
        }

        (loss, n, skipped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Grads;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
    }

    fn tiny_sample(seed: u64) -> Sample {
        let mut r = Rng(seed.max(1));
        let mut feats = [Vec::new(), Vec::new()];
        for half in &mut feats {
            for _ in 0..38 {
                half.push((r.next() % FT_IN as u64) as u32);
            }
        }
        let mut efeats = Vec::new();
        for _ in 0..30 {
            efeats.push((r.next() % EFFECT_IN as u64) as u16);
        }
        Sample {
            feats,
            efeats,
            target: (r.next() % 1000) as f32 / 1000.0,
        }
    }

    /// 並列学習が単スレッド基準実装と同じ重みに到達する
    /// （f32の加算順序差のみ許容）。
    #[test]
    fn parallel_matches_serial() {
        let batches: Vec<Vec<Sample>> = (0..4)
            .map(|b| (0..64).map(|i| tiny_sample(b * 64 + i + 1)).collect())
            .collect();

        // 単スレッド基準
        let mut net_s = FloatNet::random(5);
        let mut adam = Adam::new(1e-3);
        let mut g = Grads::new();
        for batch in &batches {
            for s in batch {
                let act = net_s.forward(s);
                net_s.backward(s, &act, &mut g);
            }
            adam.step(&mut net_s, &g, 1.0 / batch.len() as f32);
            g.clear();
        }

        // 並列（3スレッド）
        let mut par = ParallelTrainer::new(FloatNet::random(5), 1e-3, 0.7, 3);
        for batch in &batches {
            par.train_samples(batch);
        }

        let max_diff = |a: &[f32], b: &[f32]| -> f32 {
            a.iter()
                .zip(b)
                .map(|(x, y)| (x - y).abs())
                .fold(0.0, f32::max)
        };
        assert!(max_diff(&net_s.ft_w, &par.net.ft_w) < 1e-4);
        assert!(max_diff(&net_s.ef_w, &par.net.ef_w) < 1e-4);
        assert!(max_diff(&net_s.w2, &par.net.w2) < 1e-4);
        assert!(max_diff(&net_s.w4, &par.net.w4) < 1e-4);
        assert!((net_s.b4 - par.net.b4).abs() < 1e-4);
    }
}
