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

/// USI_Ponderが有効なときにoptimumへ足す割合の逆数（ADR-0104）。
/// Stockfish `src/timeman.cpp` の `optimumTime += optimumTime / 4`。
const PONDER_OPTIMUM_DIV: u64 = 4;

pub struct TimeManager {
    start: Instant,
    pub optimum: Option<Duration>,
    pub maximum: Option<Duration>,
}

impl TimeManager {
    /// `start` は計時の起点。ponderhitで再起動するときは `go ponder` の
    /// 受信時刻を渡し、ponderで読んだ分を予算に数える（ADR-0104）。
    /// `ponder_enabled` はUSI_Ponderの値で、真ならoptimumを1.25倍する。
    pub fn new(
        limits: &Limits,
        us: Color,
        game_ply: u16,
        delay_ms: u64,
        delay2_ms: u64,
        start: Instant,
        ponder_enabled: bool,
    ) -> Self {
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
        let mut optimum = avail.saturating_sub(delay_ms).max(10);
        // ponderが当たれば相手の時計で読めるぶん、自分の時計は厚く使える
        // （ADR-0104。Stockfish timeman.cppの1.25倍）
        if ponder_enabled {
            optimum += optimum / PONDER_OPTIMUM_DIV;
        }
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
        let tm = TimeManager::new(&limits, Color::Black, 50, 120, 1120, Instant::now(), false);
        // 残り時間0でも秒読み分は使える
        assert!(tm.optimum.unwrap() >= Duration::from_millis(2000));
        assert!(tm.maximum.unwrap() <= Duration::from_millis(3000));
    }

    #[test]
    fn ponder_enabled_widens_optimum() {
        let limits = Limits {
            btime: 300_000,
            binc: 10_000,
            ..Limits::default()
        };
        let off = TimeManager::new(&limits, Color::Black, 0, 120, 1120, Instant::now(), false);
        let on = TimeManager::new(&limits, Color::Black, 0, 120, 1120, Instant::now(), true);
        let (o, n) = (off.optimum.unwrap(), on.optimum.unwrap());
        // 1.25倍。ミリ秒の整数除算で1msまでずれる
        let want = o * 5 / 4;
        let slack = Duration::from_millis(1);
        assert!(n + slack >= want && n <= want + slack, "{n:?} vs {want:?}");
    }

    #[test]
    fn start_carries_the_ponder_elapsed() {
        let limits = Limits {
            btime: 300_000,
            binc: 10_000,
            ..Limits::default()
        };
        // go ponderを1時間前に受けた想定。optimumはとうに超えている
        let past = Instant::now() - Duration::from_secs(3600);
        let tm = TimeManager::new(&limits, Color::Black, 0, 120, 1120, past, false);
        assert!(tm.elapsed() >= Duration::from_secs(3600));
        assert!(tm.over_total(1.0) && tm.over_maximum());
    }

    #[test]
    fn infinite_has_no_limits() {
        let limits = Limits {
            infinite: true,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, Color::Black, 1, 120, 1120, Instant::now(), false);
        assert!(tm.optimum.is_none() && tm.maximum.is_none());
    }

    #[test]
    fn movetime_is_fixed() {
        let limits = Limits {
            movetime: 1000,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, Color::White, 1, 120, 1120, Instant::now(), false);
        assert_eq!(tm.optimum, tm.maximum);
        assert!(tm.maximum.unwrap() < Duration::from_millis(1000));
    }
}
