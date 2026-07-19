//! SPRT統計（ADR-0027）。
//!
//! pentanomial（ペア得点の5値）度数から、GSPRTの正規近似でLLRを
//! 逐次計算する。式はfishtestの簡易近似と同じ。

/// ペア得点 {0, 0.5, 1, 1.5, 2} の度数。添字は得点×2。
#[derive(Default, Clone, Copy)]
pub struct Pentanomial(pub [u64; 5]);

/// ペア得点を[0,1]へ正規化した値（1局あたりの得点）。
const VALUES: [f64; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

impl Pentanomial {
    pub fn add(&mut self, bin: usize) {
        self.0[bin] += 1;
    }

    pub fn total(&self) -> u64 {
        self.0.iter().sum()
    }

    /// (ペア数, 正規化得点の平均, 分散)。ゼロ度数はε=1e-3で正則化する。
    fn mean_var(&self) -> (f64, f64, f64) {
        let c: Vec<f64> = self.0.iter().map(|&x| (x as f64).max(1e-3)).collect();
        let n: f64 = c.iter().sum();
        let mean = c.iter().zip(VALUES).map(|(c, v)| c * v).sum::<f64>() / n;
        let var = c.iter().zip(VALUES).map(|(c, v)| c * v * v).sum::<f64>() / n - mean * mean;
        (n, mean, var)
    }
}

/// Elo差を期待得点[0,1]へ変換する。
fn logistic(elo: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf(-elo / 400.0))
}

/// 期待得点[0,1]をElo差へ変換する。
fn elo_of(score: f64) -> f64 {
    let s = score.clamp(1e-9, 1.0 - 1e-9);
    -400.0 * (1.0 / s - 1.0).log10()
}

pub struct Sprt {
    pub elo0: f64,
    pub elo1: f64,
    pub alpha: f64,
    pub beta: f64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Decision {
    AcceptH1,
    AcceptH0,
    Continue,
}

impl Sprt {
    /// LLRの打ち切り閾値 (下限, 上限)。
    pub fn bounds(&self) -> (f64, f64) {
        (
            (self.beta / (1.0 - self.alpha)).ln(),
            ((1.0 - self.beta) / self.alpha).ln(),
        )
    }

    /// GSPRTの正規近似によるLLR。
    pub fn llr(&self, p: &Pentanomial) -> f64 {
        if p.total() == 0 {
            return 0.0;
        }
        let (n, mean, var) = p.mean_var();
        if var <= 0.0 {
            return 0.0;
        }
        let s0 = logistic(self.elo0);
        let s1 = logistic(self.elo1);
        (s1 - s0) * (2.0 * mean - s0 - s1) * n / (2.0 * var)
    }

    pub fn decision(&self, llr: f64) -> Decision {
        let (lower, upper) = self.bounds();
        if llr >= upper {
            Decision::AcceptH1
        } else if llr <= lower {
            Decision::AcceptH0
        } else {
            Decision::Continue
        }
    }
}

/// Elo点推定と95%信頼区間 (推定値, 下限, 上限)。
pub fn elo_estimate(p: &Pentanomial) -> (f64, f64, f64) {
    let (n, mean, var) = p.mean_var();
    let se = (var.max(0.0) / n).sqrt();
    (
        elo_of(mean),
        elo_of(mean - 1.96 * se),
        elo_of(mean + 1.96 * se),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sprt() -> Sprt {
        Sprt {
            elo0: 0.0,
            elo1: 5.0,
            alpha: 0.05,
            beta: 0.05,
        }
    }

    #[test]
    fn bounds_are_symmetric() {
        let (lower, upper) = sprt().bounds();
        assert!((upper - 2.944).abs() < 0.01);
        assert!((lower + 2.944).abs() < 0.01);
    }

    #[test]
    fn llr_increases_with_wins() {
        let s = sprt();
        // 勝ち越しペアが多いほどLLRは大きい
        let strong = Pentanomial([10, 20, 100, 200, 100]);
        let even = Pentanomial([50, 100, 200, 100, 50]);
        let weak = Pentanomial([100, 200, 100, 20, 10]);
        assert!(s.llr(&strong) > s.llr(&even));
        assert!(s.llr(&even) > s.llr(&weak));
        assert!(s.llr(&strong) > 0.0);
        assert!(s.llr(&weak) < 0.0);
    }

    #[test]
    fn llr_handles_degenerate_counts() {
        let s = sprt();
        assert_eq!(s.llr(&Pentanomial::default()), 0.0);
        // 全ペア同一結果でも発散しない（ε正則化）
        let all_wins = Pentanomial([0, 0, 0, 0, 100]);
        assert!(s.llr(&all_wins).is_finite());
        assert!(s.llr(&all_wins) > 0.0);
    }

    #[test]
    fn elo_estimate_sign() {
        let (elo, lo, hi) = elo_estimate(&Pentanomial([10, 20, 100, 200, 100]));
        assert!(elo > 0.0);
        assert!(lo < elo && elo < hi);
        let (elo, _, _) = elo_estimate(&Pentanomial([50, 100, 200, 100, 50]));
        assert!(elo.abs() < 1e-9);
    }

    #[test]
    fn decision_thresholds() {
        let s = sprt();
        assert_eq!(s.decision(3.0), Decision::AcceptH1);
        assert_eq!(s.decision(-3.0), Decision::AcceptH0);
        assert_eq!(s.decision(0.0), Decision::Continue);
    }
}
