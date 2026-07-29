//! 時間管理（ADR-0021）。optimum/maximumの2段階。
//!
//! 制限は `TimeCtl` に原子変数で持つ（ADR-0106）。ponderhitで探索を
//! 止めずに制限だけ差し替えるため、探索中に書き換えられる必要がある。

use std::sync::atomic::{AtomicU64, Ordering};
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

/// 「無制限」を表す番兵。
const NO_LIMIT: u64 = u64::MAX;

/// optimum/maximumの計算（ADR-0021）。無制限ならNone。
fn compute(
    limits: &Limits,
    us: Color,
    game_ply: u16,
    delay_ms: u64,
    delay2_ms: u64,
) -> Option<(u64, u64)> {
    if limits.infinite {
        return None;
    }
    if limits.movetime > 0 {
        let t = limits.movetime.saturating_sub(delay_ms).max(10);
        return Some((t, t));
    }
    let (my_time, inc) = match us {
        Color::Black => (limits.btime, limits.binc),
        Color::White => (limits.wtime, limits.winc),
    };
    if my_time == 0 && limits.byoyomi == 0 && inc == 0 {
        // 時間指定なし（go単体）。無制限扱い
        return None;
    }
    // 残り想定手数で割り、秒読み・加算を足す（ADR-0021の初期式）
    let rem_moves = (48u64.saturating_sub(u64::from(game_ply) / 2)).max(16);
    let avail = my_time / rem_moves + limits.byoyomi + inc;
    let optimum = avail.saturating_sub(delay_ms).max(10);
    let hard_cap = (my_time + limits.byoyomi).saturating_sub(delay2_ms);
    let maximum = (avail * 3).min(hard_cap).saturating_sub(delay_ms).max(10);
    Some((optimum, maximum.max(optimum)))
}

/// 探索中に差し替えられる時間制限（ADR-0106）。
///
/// ponderhitは探索を止めずに制限だけを実時間へ切り替える。そのため
/// 起点と制限を原子変数で持ち、探索スレッドから読めるようにする。
/// `base` はプロセス内の基準時刻で、経過は `base` からのミリ秒差で扱う。
pub struct TimeCtl {
    base: Instant,
    start_ms: AtomicU64,
    optimum_ms: AtomicU64,
    maximum_ms: AtomicU64,
}

impl Default for TimeCtl {
    fn default() -> Self {
        TimeCtl {
            base: Instant::now(),
            start_ms: AtomicU64::new(0),
            optimum_ms: AtomicU64::new(NO_LIMIT),
            maximum_ms: AtomicU64::new(NO_LIMIT),
        }
    }
}

impl TimeCtl {
    #[inline]
    fn now_ms(&self) -> u64 {
        self.base.elapsed().as_millis() as u64
    }

    /// 計時を今から始め、制限を設定する。`None` なら無制限。
    pub fn arm(&self, limits: &Limits, us: Color, game_ply: u16, delay_ms: u64, delay2_ms: u64) {
        let (opt, max) = match compute(limits, us, game_ply, delay_ms, delay2_ms) {
            Some(v) => v,
            None => (NO_LIMIT, NO_LIMIT),
        };
        // 起点を先に置く。探索側が古い起点で新しい制限を見ても、
        // 経過が過大になるだけで時間切れ側へ倒れる
        self.start_ms.store(self.now_ms(), Ordering::Relaxed);
        self.optimum_ms.store(opt, Ordering::Relaxed);
        self.maximum_ms.store(max, Ordering::Relaxed);
    }

    /// 無制限にする（ponder探索とヘルパー）。
    pub fn disarm(&self) {
        self.start_ms.store(self.now_ms(), Ordering::Relaxed);
        self.optimum_ms.store(NO_LIMIT, Ordering::Relaxed);
        self.maximum_ms.store(NO_LIMIT, Ordering::Relaxed);
    }
}

/// `TimeCtl` を読む側。ワーカーが1つずつ持つ。
pub struct TimeManager {
    ctl: std::sync::Arc<TimeCtl>,
}

impl TimeManager {
    pub fn new(ctl: std::sync::Arc<TimeCtl>) -> Self {
        TimeManager { ctl }
    }

    /// 無制限の使い捨て（ヘルパースレッド・テスト用）。
    pub fn unlimited() -> Self {
        TimeManager {
            ctl: std::sync::Arc::new(TimeCtl::default()),
        }
    }

    #[inline]
    fn optimum_ms(&self) -> Option<u64> {
        match self.ctl.optimum_ms.load(Ordering::Relaxed) {
            NO_LIMIT => None,
            v => Some(v),
        }
    }

    #[inline]
    fn maximum_ms(&self) -> Option<u64> {
        match self.ctl.maximum_ms.load(Ordering::Relaxed) {
            NO_LIMIT => None,
            v => Some(v),
        }
    }

    #[inline]
    pub fn elapsed(&self) -> Duration {
        let start = self.ctl.start_ms.load(Ordering::Relaxed);
        Duration::from_millis(self.ctl.now_ms().saturating_sub(start))
    }

    /// optimumのscale倍とmaximumの小さいほうを超えたか（ADR-0059）。
    /// scaleは局面の難易度による伸縮係数で、1.0なら従来のoptimum判定。
    #[inline]
    pub fn over_total(&self, scale: f64) -> bool {
        let Some(opt) = self.optimum_ms() else {
            return false;
        };
        let mut t = opt as f64 * scale;
        if let Some(m) = self.maximum_ms() {
            t = t.min(m as f64);
        }
        self.elapsed().as_millis() as f64 >= t
    }

    #[inline]
    pub fn over_maximum(&self) -> bool {
        self.maximum_ms()
            .is_some_and(|t| self.elapsed().as_millis() as u64 >= t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn armed(limits: &Limits, us: Color, ply: u16) -> (TimeManager, Arc<TimeCtl>) {
        let ctl = Arc::new(TimeCtl::default());
        ctl.arm(limits, us, ply, 120, 1120);
        (TimeManager::new(Arc::clone(&ctl)), ctl)
    }

    #[test]
    fn byoyomi_only() {
        let limits = Limits {
            byoyomi: 3000,
            ..Limits::default()
        };
        let (tm, _c) = armed(&limits, Color::Black, 50);
        // 残り時間0でも秒読み分は使える
        assert!(!tm.over_total(1.0));
        assert!(tm.optimum_ms().unwrap() >= 2000);
        assert!(tm.maximum_ms().unwrap() <= 3000);
    }

    #[test]
    fn infinite_has_no_limits() {
        let limits = Limits {
            infinite: true,
            ..Limits::default()
        };
        let (tm, _c) = armed(&limits, Color::Black, 1);
        assert!(tm.optimum_ms().is_none() && tm.maximum_ms().is_none());
        assert!(!tm.over_total(1.0) && !tm.over_maximum());
    }

    #[test]
    fn movetime_is_fixed() {
        let limits = Limits {
            movetime: 1000,
            ..Limits::default()
        };
        let (tm, _c) = armed(&limits, Color::White, 1);
        assert_eq!(tm.optimum_ms(), tm.maximum_ms());
        assert!(tm.maximum_ms().unwrap() < 1000);
    }

    /// ponderhitの差し替え（ADR-0106）。無制限で走っている探索に
    /// 実時間の制限を後から入れられる。
    #[test]
    fn arm_switches_a_running_search_from_unlimited() {
        let ctl = Arc::new(TimeCtl::default());
        ctl.disarm();
        let tm = TimeManager::new(Arc::clone(&ctl));
        assert!(!tm.over_total(1.0), "ponder中は無制限");

        let limits = Limits {
            movetime: 10,
            ..Limits::default()
        };
        ctl.arm(&limits, Color::Black, 1, 0, 0);
        assert_eq!(tm.optimum_ms(), Some(10));
        std::thread::sleep(Duration::from_millis(30));
        assert!(tm.over_total(1.0) && tm.over_maximum(), "ponderhit後は効く");
    }
}
