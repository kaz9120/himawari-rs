//! 学習器v1のf32モデル（ADR-0039）。
//!
//! 整数推論（ADR-0036）と一対一対応するfloat意味論を持つ。
//! 活性はclamp(x,0,1)（ClippedReLU 0..127のスケール1/127）。
//! 量子化スケールはFT系=127、隠れ層の重み=64・バイアス=64×127、
//! 出力層はシグモイドスケール600とFV_SCALE=16から導出する。

use himawari_engine::nnue::{CONCAT, EFFECT_IN, EFFECT_OUT, FT_IN, FT_OUT, HIDDEN, NnueNetwork};

/// 勝率変換のシグモイドスケール（cp）。
pub const SIGMOID_SCALE: f32 = 600.0;
/// 出力層の量子化スケール。整数推論の out/FV_SCALE = cp に合わせ、
/// float出力 v（勝率ロジット）→ cp = 600*v から
/// w4_i8 = round(w4_f × 600×16/127)、b4_i32 = round(b4_f × 600×16)。
pub const OUT_W_SCALE: f32 = SIGMOID_SCALE * 16.0 / 127.0;
pub const OUT_B_SCALE: f32 = SIGMOID_SCALE * 16.0;

/// 隠れ層重みの量子化可能域（i8/スケール64）。
const HIDDEN_W_LIMIT: f32 = 127.0 / 64.0;
/// 出力層重みの量子化可能域。
const OUT_W_LIMIT: f32 = 127.0 / OUT_W_SCALE;

/// 教師1局面。特徴は抽出済み（手番視点が先）。
pub struct Sample {
    pub feats: [Vec<u32>; 2],
    pub efeats: Vec<u16>,
    /// 勝率ターゲット（scoreとresultのλ混合）。
    pub target: f32,
}

/// f32の重み一式。レイアウトは推論側（NnueNetwork）と同じ。
pub struct FloatNet {
    pub ft_w: Vec<f32>,
    pub ft_b: Vec<f32>,
    pub ef_w: Vec<f32>,
    pub ef_b: Vec<f32>,
    pub w2: Vec<f32>,
    pub b2: Vec<f32>,
    pub w3: Vec<f32>,
    pub b3: Vec<f32>,
    pub w4: Vec<f32>,
    pub b4: f32,
}

/// 勾配バッファ。FT・利き塔はtouched行のみ意味を持つ。
pub struct Grads {
    pub ft_w: Vec<f32>,
    pub ft_b: Vec<f32>,
    pub ef_w: Vec<f32>,
    pub ef_b: Vec<f32>,
    pub w2: Vec<f32>,
    pub b2: Vec<f32>,
    pub w3: Vec<f32>,
    pub b3: Vec<f32>,
    pub w4: Vec<f32>,
    pub b4: f32,
    /// このバッチで触れたFT行・利き塔行（重複あり。step時にdedup）。
    pub touched_ft: Vec<u32>,
    pub touched_ef: Vec<u16>,
}

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
    /// [-1, 1) の一様乱数。
    fn uniform(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0
    }
}

impl FloatNet {
    /// 乱数初期化。FTバイアスは活性の線形域（0.5）に置く。
    pub fn random(seed: u64) -> FloatNet {
        let mut r = Rng(seed.max(1));
        let mut vf = |n: usize, s: f32| -> Vec<f32> { (0..n).map(|_| r.uniform() * s).collect() };
        FloatNet {
            ft_w: vf(FT_IN * FT_OUT, 0.05),
            ft_b: vec![0.5; FT_OUT],
            ef_w: vf(EFFECT_IN * EFFECT_OUT, 0.05),
            ef_b: vec![0.5; EFFECT_OUT],
            w2: vf(HIDDEN * CONCAT, 0.1),
            b2: vec![0.0; HIDDEN],
            w3: vf(HIDDEN * HIDDEN, 0.3),
            b3: vec![0.0; HIDDEN],
            w4: vf(HIDDEN, 0.3),
            b4: 0.0,
        }
    }

    /// 順伝播。中間活性を返す（逆伝播で使う）。
    pub fn forward(&self, s: &Sample) -> Activations {
        let mut z1 = [[0f32; FT_OUT]; 2];
        for (half, feats) in s.feats.iter().enumerate() {
            let z = &mut z1[half];
            z.copy_from_slice(&self.ft_b);
            for &f in feats {
                let row = &self.ft_w[f as usize * FT_OUT..(f as usize + 1) * FT_OUT];
                for (o, w) in z.iter_mut().zip(row) {
                    *o += w;
                }
            }
        }
        let mut ze = [0f32; EFFECT_OUT];
        ze.copy_from_slice(&self.ef_b);
        for &f in &s.efeats {
            let row = &self.ef_w[f as usize * EFFECT_OUT..(f as usize + 1) * EFFECT_OUT];
            for (o, w) in ze.iter_mut().zip(row) {
                *o += w;
            }
        }

        let mut x = [0f32; CONCAT];
        for half in 0..2 {
            for o in 0..FT_OUT {
                x[half * FT_OUT + o] = z1[half][o].clamp(0.0, 1.0);
            }
        }
        for o in 0..EFFECT_OUT {
            x[FT_OUT * 2 + o] = ze[o].clamp(0.0, 1.0);
        }

        let mut z2 = [0f32; HIDDEN];
        for o in 0..HIDDEN {
            let row = &self.w2[o * CONCAT..(o + 1) * CONCAT];
            let mut sum = self.b2[o];
            for (w, xv) in row.iter().zip(x.iter()) {
                sum += w * xv;
            }
            z2[o] = sum;
        }
        let h2 = z2.map(|v| v.clamp(0.0, 1.0));

        let mut z3 = [0f32; HIDDEN];
        for o in 0..HIDDEN {
            let row = &self.w3[o * HIDDEN..(o + 1) * HIDDEN];
            let mut sum = self.b3[o];
            for (w, xv) in row.iter().zip(h2.iter()) {
                sum += w * xv;
            }
            z3[o] = sum;
        }
        let h3 = z3.map(|v| v.clamp(0.0, 1.0));

        let mut v = self.b4;
        for (w, xv) in self.w4.iter().zip(h3.iter()) {
            v += w * xv;
        }

        Activations {
            z1,
            ze,
            x,
            z2,
            h2,
            z3,
            h3,
            v,
        }
    }

    /// 1サンプルの逆伝播。損失は BCE(sigmoid(v), target)。
    /// 損失値を返し、勾配をgに加算する。
    pub fn backward(&self, s: &Sample, act: &Activations, g: &mut Grads) -> f32 {
        let p = sigmoid(act.v);
        let t = s.target;
        let loss = bce(p, t);
        let gv = p - t;

        g.b4 += gv;
        let mut gh3 = [0f32; HIDDEN];
        for o in 0..HIDDEN {
            g.w4[o] += gv * act.h3[o];
            gh3[o] = gv * self.w4[o];
        }
        let mut gz3 = [0f32; HIDDEN];
        for o in 0..HIDDEN {
            gz3[o] = if 0.0 < act.z3[o] && act.z3[o] < 1.0 {
                gh3[o]
            } else {
                0.0
            };
            g.b3[o] += gz3[o];
        }
        let mut gh2 = [0f32; HIDDEN];
        for o in 0..HIDDEN {
            if gz3[o] == 0.0 {
                continue;
            }
            let row = &mut g.w3[o * HIDDEN..(o + 1) * HIDDEN];
            for i in 0..HIDDEN {
                row[i] += gz3[o] * act.h2[i];
                gh2[i] += gz3[o] * self.w3[o * HIDDEN + i];
            }
        }
        let mut gx = [0f32; CONCAT];
        for o in 0..HIDDEN {
            let gz2 = if 0.0 < act.z2[o] && act.z2[o] < 1.0 {
                gh2[o]
            } else {
                0.0
            };
            if gz2 == 0.0 {
                continue;
            }
            g.b2[o] += gz2;
            let row = &mut g.w2[o * CONCAT..(o + 1) * CONCAT];
            let wrow = &self.w2[o * CONCAT..(o + 1) * CONCAT];
            for i in 0..CONCAT {
                row[i] += gz2 * act.x[i];
                gx[i] += gz2 * wrow[i];
            }
        }

        for half in 0..2 {
            let mut gz1 = [0f32; FT_OUT];
            let mut any = false;
            for o in 0..FT_OUT {
                let z = act.z1[half][o];
                if 0.0 < z && z < 1.0 {
                    gz1[o] = gx[half * FT_OUT + o];
                    any |= gz1[o] != 0.0;
                }
            }
            if !any {
                continue;
            }
            for (o, gb) in g.ft_b.iter_mut().enumerate() {
                *gb += gz1[o];
            }
            for &f in &s.feats[half] {
                let row = &mut g.ft_w[f as usize * FT_OUT..(f as usize + 1) * FT_OUT];
                for (o, gw) in row.iter_mut().enumerate() {
                    *gw += gz1[o];
                }
                g.touched_ft.push(f);
            }
        }
        {
            let mut gze = [0f32; EFFECT_OUT];
            let mut any = false;
            for o in 0..EFFECT_OUT {
                let z = act.ze[o];
                if 0.0 < z && z < 1.0 {
                    gze[o] = gx[FT_OUT * 2 + o];
                    any |= gze[o] != 0.0;
                }
            }
            if any {
                for (o, gb) in g.ef_b.iter_mut().enumerate() {
                    *gb += gze[o];
                }
                for &f in &s.efeats {
                    let row = &mut g.ef_w[f as usize * EFFECT_OUT..(f as usize + 1) * EFFECT_OUT];
                    for (o, gw) in row.iter_mut().enumerate() {
                        *gw += gze[o];
                    }
                    g.touched_ef.push(f);
                }
            }
        }
        loss
    }

    /// 量子化して推論用ネットワークに変換する（ADR-0036のスケール）。
    pub fn quantize(&self) -> NnueNetwork {
        let q16 = |v: &[f32], s: f32| -> Vec<i16> {
            v.iter()
                .map(|&x| (x * s).round().clamp(-32768.0, 32767.0) as i16)
                .collect()
        };
        let q8 = |v: &[f32], s: f32| -> Vec<i8> {
            v.iter()
                .map(|&x| (x * s).round().clamp(-128.0, 127.0) as i8)
                .collect()
        };
        let q32 =
            |v: &[f32], s: f32| -> Vec<i32> { v.iter().map(|&x| (x * s).round() as i32).collect() };
        NnueNetwork {
            ft_w: q16(&self.ft_w, 127.0),
            ft_b: q16(&self.ft_b, 127.0),
            ef_w: q16(&self.ef_w, 127.0),
            ef_b: q16(&self.ef_b, 127.0),
            w2: q8(&self.w2, 64.0),
            b2: q32(&self.b2, 64.0 * 127.0),
            w3: q8(&self.w3, 64.0),
            b3: q32(&self.b3, 64.0 * 127.0),
            w4: q8(&self.w4, OUT_W_SCALE),
            b4: (self.b4 * OUT_B_SCALE).round() as i32,
        }
    }

    /// 量子化可能域へのクリップ（step後に呼ぶ）。
    pub fn clip_weights(&mut self) {
        for w in &mut self.w2 {
            *w = w.clamp(-HIDDEN_W_LIMIT, HIDDEN_W_LIMIT);
        }
        for w in &mut self.w3 {
            *w = w.clamp(-HIDDEN_W_LIMIT, HIDDEN_W_LIMIT);
        }
        for w in &mut self.w4 {
            *w = w.clamp(-OUT_W_LIMIT, OUT_W_LIMIT);
        }
    }
}

/// 順伝播の中間活性。
pub struct Activations {
    pub z1: [[f32; FT_OUT]; 2],
    pub ze: [f32; EFFECT_OUT],
    pub x: [f32; CONCAT],
    pub z2: [f32; HIDDEN],
    pub h2: [f32; HIDDEN],
    pub z3: [f32; HIDDEN],
    pub h3: [f32; HIDDEN],
    pub v: f32,
}

pub fn sigmoid(v: f32) -> f32 {
    1.0 / (1.0 + (-v).exp())
}

/// 2値交差エントロピー（数値安定化つき）。
pub fn bce(p: f32, t: f32) -> f32 {
    let eps = 1e-7;
    let p = p.clamp(eps, 1.0 - eps);
    -(t * p.ln() + (1.0 - t) * (1.0 - p).ln())
}

impl Grads {
    pub fn new() -> Grads {
        Grads {
            ft_w: vec![0.0; FT_IN * FT_OUT],
            ft_b: vec![0.0; FT_OUT],
            ef_w: vec![0.0; EFFECT_IN * EFFECT_OUT],
            ef_b: vec![0.0; EFFECT_OUT],
            w2: vec![0.0; HIDDEN * CONCAT],
            b2: vec![0.0; HIDDEN],
            w3: vec![0.0; HIDDEN * HIDDEN],
            b3: vec![0.0; HIDDEN],
            w4: vec![0.0; HIDDEN],
            b4: 0.0,
            touched_ft: Vec::new(),
            touched_ef: Vec::new(),
        }
    }

    /// touched行と密部分をゼロクリアする（次バッチへ）。
    pub fn clear(&mut self) {
        self.touched_ft.sort_unstable();
        self.touched_ft.dedup();
        for &f in &self.touched_ft {
            self.ft_w[f as usize * FT_OUT..(f as usize + 1) * FT_OUT].fill(0.0);
        }
        self.touched_ef.sort_unstable();
        self.touched_ef.dedup();
        for &f in &self.touched_ef {
            self.ef_w[f as usize * EFFECT_OUT..(f as usize + 1) * EFFECT_OUT].fill(0.0);
        }
        self.touched_ft.clear();
        self.touched_ef.clear();
        self.ft_b.fill(0.0);
        self.ef_b.fill(0.0);
        self.w2.fill(0.0);
        self.b2.fill(0.0);
        self.w3.fill(0.0);
        self.b3.fill(0.0);
        self.w4.fill(0.0);
        self.b4 = 0.0;
    }
}

/// Adam。FT・利き塔の行はtouchedのみ更新する（lazy）。
pub struct Adam {
    lr: f32,
    beta1: f32,
    beta2: f32,
    eps: f32,
    t: i32,
    m: FloatNet,
    v: FloatNet,
}

impl Adam {
    pub fn new(lr: f32) -> Adam {
        let zero = || FloatNet {
            ft_w: vec![0.0; FT_IN * FT_OUT],
            ft_b: vec![0.0; FT_OUT],
            ef_w: vec![0.0; EFFECT_IN * EFFECT_OUT],
            ef_b: vec![0.0; EFFECT_OUT],
            w2: vec![0.0; HIDDEN * CONCAT],
            b2: vec![0.0; HIDDEN],
            w3: vec![0.0; HIDDEN * HIDDEN],
            b3: vec![0.0; HIDDEN],
            w4: vec![0.0; HIDDEN],
            b4: 0.0,
        };
        Adam {
            lr,
            beta1: 0.9,
            beta2: 0.999,
            eps: 1e-8,
            t: 0,
            m: zero(),
            v: zero(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn step_slice(
        lr: f32,
        beta1: f32,
        beta2: f32,
        eps: f32,
        bc1: f32,
        bc2: f32,
        w: &mut [f32],
        g: &[f32],
        m: &mut [f32],
        v: &mut [f32],
        scale: f32,
    ) {
        for i in 0..w.len() {
            let gi = g[i] * scale;
            m[i] = beta1 * m[i] + (1.0 - beta1) * gi;
            v[i] = beta2 * v[i] + (1.0 - beta2) * gi * gi;
            let mh = m[i] / bc1;
            let vh = v[i] / bc2;
            w[i] -= lr * mh / (vh.sqrt() + eps);
        }
    }

    /// バッチ勾配で1ステップ更新する。scaleは1/バッチサイズ。
    pub fn step(&mut self, net: &mut FloatNet, g: &Grads, scale: f32) {
        self.t += 1;
        let bc1 = 1.0 - self.beta1.powi(self.t);
        let bc2 = 1.0 - self.beta2.powi(self.t);
        let (lr, b1, b2e, eps) = (self.lr, self.beta1, self.beta2, self.eps);

        // touchedのFT・利き塔行（clear()でdedup済みの想定はしない）
        let mut rows: Vec<u32> = g.touched_ft.clone();
        rows.sort_unstable();
        rows.dedup();
        for &f in &rows {
            let r = f as usize * FT_OUT..(f as usize + 1) * FT_OUT;
            Self::step_slice(
                lr,
                b1,
                b2e,
                eps,
                bc1,
                bc2,
                &mut net.ft_w[r.clone()],
                &g.ft_w[r.clone()],
                &mut self.m.ft_w[r.clone()],
                &mut self.v.ft_w[r],
                scale,
            );
        }
        let mut erows: Vec<u16> = g.touched_ef.clone();
        erows.sort_unstable();
        erows.dedup();
        for &f in &erows {
            let r = f as usize * EFFECT_OUT..(f as usize + 1) * EFFECT_OUT;
            Self::step_slice(
                lr,
                b1,
                b2e,
                eps,
                bc1,
                bc2,
                &mut net.ef_w[r.clone()],
                &g.ef_w[r.clone()],
                &mut self.m.ef_w[r.clone()],
                &mut self.v.ef_w[r],
                scale,
            );
        }
        macro_rules! dense {
            ($f:ident) => {
                Self::step_slice(
                    lr,
                    b1,
                    b2e,
                    eps,
                    bc1,
                    bc2,
                    &mut net.$f,
                    &g.$f,
                    &mut self.m.$f,
                    &mut self.v.$f,
                    scale,
                );
            };
        }
        dense!(ft_b);
        dense!(ef_b);
        dense!(w2);
        dense!(b2);
        dense!(w3);
        dense!(b3);
        dense!(w4);
        let gb = g.b4 * scale;
        self.m.b4 = b1 * self.m.b4 + (1.0 - b1) * gb;
        self.v.b4 = b2e * self.v.b4 + (1.0 - b2e) * gb * gb;
        net.b4 -= lr * (self.m.b4 / bc1) / ((self.v.b4 / bc2).sqrt() + eps);

        net.clip_weights();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// 解析勾配と数値微分の一致（代表パラメータを抽出して比較）。
    #[test]
    fn gradient_matches_numerical() {
        let mut net = FloatNet::random(7);
        let samples: Vec<Sample> = (1..=3).map(tiny_sample).collect();
        let mut g = Grads::new();
        for s in &samples {
            let act = net.forward(s);
            net.backward(s, &act, &mut g);
        }

        let loss_of = |net: &FloatNet| -> f64 {
            samples
                .iter()
                .map(|s| f64::from(bce(sigmoid(net.forward(s).v), s.target)))
                .sum()
        };

        // 代表パラメータ: 触れたFT行・利き塔行・各密パラメータ
        let f0 = samples[0].feats[0][0] as usize * FT_OUT + 3;
        let e0 = samples[0].efeats[0] as usize * EFFECT_OUT + 5;
        let checks: Vec<(f32, *mut f32)> = vec![
            (g.ft_w[f0], &mut net.ft_w[f0]),
            (g.ft_b[3], &mut net.ft_b[3]),
            (g.ef_w[e0], &mut net.ef_w[e0]),
            (g.w2[100], &mut net.w2[100]),
            (g.b2[0], &mut net.b2[0]),
            (g.w3[50], &mut net.w3[50]),
            (g.w4[7], &mut net.w4[7]),
            (g.b4, &mut net.b4),
        ];
        for (analytic, ptr) in checks {
            let h = 3e-3f32;
            // SAFETY: ptrはnetの生存中のみ使い、他の参照と同時に使わない
            let numeric = unsafe {
                let orig = *ptr;
                *ptr = orig + h;
                let lp = loss_of(&net);
                *ptr = orig - h;
                let lm = loss_of(&net);
                *ptr = orig;
                (lp - lm) / (2.0 * f64::from(h))
            };
            let a = f64::from(analytic);
            assert!(
                (a - numeric).abs() <= 0.02 * numeric.abs().max(0.05),
                "勾配不一致: 解析{a} 数値{numeric}"
            );
        }
    }

    /// 小データで損失がほぼゼロまで下がる（過学習スモーク）。
    /// ターゲットは0/1のみにしてBCEの下限を0にする。
    #[test]
    fn overfits_small_dataset() {
        let mut net = FloatNet::random(11);
        let samples: Vec<Sample> = (1..=16)
            .map(|i| {
                let mut s = tiny_sample(i);
                s.target = (i % 2) as f32;
                s
            })
            .collect();
        let mut adam = Adam::new(3e-3);
        let mut g = Grads::new();
        let mut last = f32::MAX;
        for _ in 0..600 {
            let mut sum = 0.0;
            for s in &samples {
                let act = net.forward(s);
                sum += net.backward(s, &act, &mut g);
            }
            adam.step(&mut net, &g, 1.0 / samples.len() as f32);
            g.clear();
            last = sum / samples.len() as f32;
        }
        assert!(last < 0.05, "過学習スモーク失敗: loss={last}");
    }

    /// 量子化後の整数推論がfloat推論とcp単位で近い。
    #[test]
    fn quantized_matches_float() {
        use himawari_core::{Position, SFEN_STARTPOS};
        use himawari_engine::nnue::{effect_active, evaluate_scalar, halfkp_active};

        let net = FloatNet::random(13);
        let q = net.quantize();
        let pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
        let stm = pos.side_to_move();
        let mut feats = [Vec::new(), Vec::new()];
        halfkp_active(&pos, stm, &mut feats[0]);
        halfkp_active(&pos, stm.flip(), &mut feats[1]);
        let mut efeats = Vec::new();
        effect_active(&pos, &mut efeats);
        let s = Sample {
            feats,
            efeats,
            target: 0.5,
        };
        let float_cp = f64::from(net.forward(&s).v) * f64::from(SIGMOID_SCALE);
        let int_cp = f64::from(evaluate_scalar(&q, &pos));
        assert!(
            (float_cp - int_cp).abs() < 30.0,
            "量子化誤差が大きすぎる: float {float_cp:.1}cp int {int_cp}cp"
        );
    }
}
