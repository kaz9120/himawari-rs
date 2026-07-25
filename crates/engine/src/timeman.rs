//! 時間管理（ADR-0021）。optimum/maximumの2段階。

use std::time::{Duration, Instant};

use himawari_core::Color;

/// goコマンドの探索制限。時間はミリ秒。
#[derive(Clone, Default, Debug)]
pub struct Limits {
    pub btime: u64,
    pub wtime: u64,
    pub byoyomi: u64,
    pub binc: u64,
    pub winc: u64,
    pub movetime: u64,
    pub depth: u32,
    pub nodes: u64,
    pub infinite: bool,
}

pub struct TimeManager {
    start: Instant,
    pub optimum: Option<Duration>,
    pub maximum: Option<Duration>,
}

impl TimeManager {
    pub fn new(limits: &Limits, us: Color, game_ply: u16, delay_ms: u64, delay2_ms: u64) -> Self {
        let start = Instant::now();
        if limits.infinite {
            return TimeManager {
                start,
                optimum: None,
                maximum: None,
            };
        }
        if limits.movetime > 0 {
            let t = limits.movetime.saturating_sub(delay_ms).max(10);
            return TimeManager {
                start,
                optimum: Some(Duration::from_millis(t)),
                maximum: Some(Duration::from_millis(t)),
            };
        }
        let (my_time, inc) = match us {
            Color::Black => (limits.btime, limits.binc),
            Color::White => (limits.wtime, limits.winc),
        };
        if my_time == 0 && limits.byoyomi == 0 && inc == 0 {
            // 時間指定なし（go単体）。無制限扱い
            return TimeManager {
                start,
                optimum: None,
                maximum: None,
            };
        }
        // 残り想定手数で割り、秒読み・加算を足す（ADR-0021の初期式）
        let rem_moves = (48u64.saturating_sub(u64::from(game_ply) / 2)).max(16);
        let avail = my_time / rem_moves + limits.byoyomi + inc;
        let optimum = avail.saturating_sub(delay_ms).max(10);
        let hard_cap = (my_time + limits.byoyomi).saturating_sub(delay2_ms);
        let maximum = (avail * 3).min(hard_cap).saturating_sub(delay_ms).max(10);
        TimeManager {
            start,
            optimum: Some(Duration::from_millis(optimum)),
            maximum: Some(Duration::from_millis(maximum.max(optimum))),
        }
    }

    #[inline]
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    #[inline]
    pub fn over_optimum(&self) -> bool {
        self.optimum.is_some_and(|t| self.elapsed() >= t)
    }

    /// optimumのscale倍とmaximumの小さいほうを超えたか（ADR-0059）。
    /// scaleは局面の難易度による伸縮係数で、1.0なら従来のoptimum判定。
    #[inline]
    pub fn over_total(&self, scale: f64) -> bool {
        let Some(opt) = self.optimum else {
            return false;
        };
        let mut t = opt.as_secs_f64() * scale;
        if let Some(m) = self.maximum {
            t = t.min(m.as_secs_f64());
        }
        self.elapsed().as_secs_f64() >= t
    }

    #[inline]
    pub fn over_maximum(&self) -> bool {
        self.maximum.is_some_and(|t| self.elapsed() >= t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byoyomi_only() {
        let limits = Limits {
            byoyomi: 3000,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, Color::Black, 50, 120, 1120);
        // 残り時間0でも秒読み分は使える
        assert!(tm.optimum.unwrap() >= Duration::from_millis(2000));
        assert!(tm.maximum.unwrap() <= Duration::from_millis(3000));
    }

    #[test]
    fn infinite_has_no_limits() {
        let limits = Limits {
            infinite: true,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, Color::Black, 1, 120, 1120);
        assert!(tm.optimum.is_none() && tm.maximum.is_none());
    }

    #[test]
    fn movetime_is_fixed() {
        let limits = Limits {
            movetime: 1000,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, Color::White, 1, 120, 1120);
        assert_eq!(tm.optimum, tm.maximum);
        assert!(tm.maximum.unwrap() < Duration::from_millis(1000));
    }
}
