//! 時間管理（ADR-0021、ADR-0109のG7）。参照実装 `source/timeman.cpp` の移植。
//!
//! 上限を超えてもその場では止めない。秒単位で切り上げた終了時刻を
//! `search_end` に予約し、最小思考時間がその下支えをする。

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

impl Limits {
    /// 時間制御を行うか（search.h:195）。思考時間固定・深さ指定・
    /// ノード指定・無制限のときは時間制御に意味がないのでやらない。
    /// 参照実装の `mate` と `perft` に対応する制限は本エンジンにない
    #[inline]
    pub fn use_time_management(&self) -> bool {
        !(self.movetime > 0 || self.depth > 0 || self.nodes > 0 || self.infinite)
    }
}

/// 時間管理が読むエンジンオプション（timeman.cpp:26-57）。
#[derive(Clone, Copy, Debug)]
pub struct TimeOptions {
    /// 指し手がGUIへ届くまでの平均遅延[ms]（timeman.cpp:39）
    pub network_delay: i64,
    /// 最大遅延[ms]。切れ負けの瞬間だけこの分早く指す（timeman.cpp:44）
    pub network_delay2: i64,
    /// 最小思考時間[ms]（timeman.cpp:47）
    pub minimum_thinking_time: i64,
    /// 序盤重視率。百分率で、optimumの係数として働く（timeman.cpp:52）
    pub slow_mover: i64,
    /// 秒未満を切り上げるか（timeman.cpp:55）
    pub round_up_to_full_second: bool,
    /// 引き分けになる手数。0（指定なし）は100000として扱う
    /// （yaneuraou-search.cpp:72-77）
    pub max_moves_to_draw: i64,
}

impl Default for TimeOptions {
    fn default() -> Self {
        TimeOptions {
            network_delay: 120,
            network_delay2: 1120,
            minimum_thinking_time: 2000,
            slow_mover: 100,
            round_up_to_full_second: true,
            max_moves_to_draw: 100_000,
        }
    }
}

pub struct TimeManager {
    start: Instant,
    /// 時間制御を行うか（search.h:195）。falseなら時刻での停止をしない
    use_time_management: bool,
    /// `ponderhitTime - startTime`[ms]（timeman.h:120）。"ponderhit" を
    /// 受けるまでは0で、goからの経過時間とponderhitからの経過時間が
    /// 一致する。ponderの会計はG8で入れるので、この群では常に0
    ponderhit_offset: i64,
    /// 探索終了予定時刻。startからの経過時間[ms]で、0なら未確定
    /// （timeman.h:93）
    pub search_end: i64,
    /// 秒読みに入り持ち時間を使い切るべきか（timeman.h:117）
    is_final_push: bool,
    minimum_time: i64,
    optimum_time: i64,
    maximum_time: i64,
    minimum_thinking_time: i64,
    network_delay: i64,
    /// 今回の最大残り時間。これを超えてはならない（timeman.cpp:154-160）
    remain_time: i64,
    round_up_to_full_second: bool,
}

impl TimeManager {
    pub fn new(limits: &Limits, us: Color, game_ply: u16, opts: &TimeOptions) -> Self {
        let mut tm = TimeManager {
            start: Instant::now(),
            use_time_management: limits.use_time_management(),
            ponderhit_offset: 0,
            search_end: 0,
            is_final_push: false,
            minimum_time: 0,
            optimum_time: 0,
            maximum_time: 0,
            minimum_thinking_time: opts.minimum_thinking_time,
            network_delay: opts.network_delay,
            remain_time: 0,
            round_up_to_full_second: opts.round_up_to_full_second,
        };
        if limits.infinite || limits.depth > 0 || limits.nodes > 0 {
            // 時刻で止めない経路。ヘルパースレッドとgo ponderもここを通る
            return tm;
        }
        tm.init(limits, us, game_ply, opts);
        tm
    }

    /// 今回の思考時間を計算する（timeman.cpp:94-308）。
    fn init(&mut self, limits: &Limits, us: Color, game_ply: u16, opts: &TimeOptions) {
        let (my_time, inc) = match us {
            Color::Black => (limits.btime, limits.binc),
            Color::White => (limits.wtime, limits.winc),
        };
        let my_time = my_time as i64;
        let inc = inc as i64;
        let byoyomi = limits.byoyomi as i64;

        // 今回の最大残り時間（timeman.cpp:154-160）。秒読みは残り時間へ
        // 加算して考える。加算時間はこの指し手のあとに増えるので足さない。
        // 0にすると時間切れのあと自爆するので下限を置く
        self.remain_time = my_time + byoyomi - opts.network_delay2;
        self.remain_time = self
            .remain_time
            .max(if self.round_up_to_full_second { 100 } else { 1 });

        // 時間固定モード（timeman.cpp:189-193）
        if limits.movetime > 0 {
            let t = limits.movetime as i64;
            self.remain_time = t;
            self.minimum_time = t;
            self.optimum_time = t;
            self.maximum_time = t;
            return;
        }

        // 最小思考時間（timeman.cpp:235-236）。秒未満を切り上げないときは
        // 秒未満での戦いなので下限を1にする
        self.minimum_time = (opts.minimum_thinking_time - opts.network_delay).max(
            if self.round_up_to_full_second {
                1000
            } else {
                1
            },
        );

        // 残り想定手数で割り、秒読み・加算を足す（ADR-0021の初期式）。
        // 配分式の参照実装への差し替えはこの群の最後で行う
        let rem_moves = (48i64 - i64::from(game_ply) / 2).max(16);
        let avail = my_time / rem_moves + byoyomi + inc;
        self.optimum_time = (avail - opts.network_delay).max(10);
        self.maximum_time = ((avail * 3).min(self.remain_time) - opts.network_delay).max(10);
        self.maximum_time = self.maximum_time.max(self.optimum_time);
    }

    /// 1秒単位で繰り上げてdelayを引く（timeman.cpp:312-344）。
    /// `remain_time` よりは小さくなるように制限する
    fn round_up(&self, t0: i64) -> i64 {
        if self.round_up_to_full_second {
            // 1000で繰り上げる。MinimumThinkingTimeが最低値
            let mut t = (((t0 + 999) / 1000) * 1000).max(self.minimum_thinking_time);
            t -= self.network_delay;
            // 元の値より小さいなら、もう1秒使わないともったいない
            if t < t0 {
                t += 1000;
            }
            t.min(self.remain_time)
        } else {
            let mut t = t0.max(self.minimum_thinking_time);
            t -= self.network_delay;
            t.min(self.remain_time)
        }
    }

    /// 探索の終了が確定したので、秒単位で切り上げた時刻を予約する
    /// （timeman.cpp:347-373）。
    ///
    /// 引数 `e` はstartからの経過時間[ms]。呼び出し側が既に持っている
    /// 値を渡してもらい、二度測るのを避ける。
    /// ponderhitからの経過時間で切り上げつつ、goから数えて `minimum()`
    /// の分は思考させる。`is_final_push` のときは切り上げの基準も
    /// ponderhitの時刻から数える
    pub fn set_search_end(&mut self, e: i64) {
        // 1. ponderhitからの経過時間（go ponderしていないならgoからの経過）
        let t1 = e - self.ponderhit_offset;
        // 2. goした時刻からminimum()を足し、ponderhitからの経過へ換算した値
        let t2 = if self.is_final_push {
            self.minimum_time
        } else {
            self.minimum_time - self.ponderhit_offset
        };
        // 大きいほうを秒単位で切り上げ、startからの経過時間へ戻す
        self.search_end = self.round_up(t1.max(t2)) + self.ponderhit_offset;
    }

    #[inline]
    pub fn use_time_management(&self) -> bool {
        self.use_time_management
    }

    #[inline]
    pub fn minimum(&self) -> i64 {
        self.minimum_time
    }

    #[inline]
    pub fn optimum(&self) -> i64 {
        self.optimum_time
    }

    #[inline]
    pub fn maximum(&self) -> i64 {
        self.maximum_time
    }

    #[inline]
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// startからの経過時間[ms]（timeman.h:58）。
    #[inline]
    pub fn elapsed_ms(&self) -> i64 {
        self.start.elapsed().as_millis() as i64
    }

    /// optimumのscale倍とmaximumの小さいほうを超えたか（ADR-0059、S:2019）。
    /// scaleは局面の難易度による伸縮係数で、1.0なら素のoptimum判定。
    /// 参照実装の `totalTime` に相当する値との比較で、判定は狭義の超過
    #[inline]
    pub fn over_total(&self, scale: f64) -> bool {
        if !self.use_time_management {
            return false;
        }
        let total = (self.optimum_time as f64 * scale).min(self.maximum_time as f64);
        self.elapsed_ms() as f64 > total
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
        let tm = TimeManager::new(&limits, Color::Black, 50, &TimeOptions::default());
        // 残り時間0でも秒読み分は使える。ただしNetworkDelay2の1120msは
        // 切れ負け防止に残すので、remain_timeは3000-1120=1880になる
        assert_eq!(tm.remain_time, 1880);
        // 最小思考時間 MinimumThinkingTime - NetworkDelay = 1880（T:235-236）
        assert_eq!(tm.minimum(), 1880);
        assert!(tm.optimum() >= 2000);
        assert!(tm.maximum() <= 3000);
    }

    #[test]
    fn infinite_has_no_limits() {
        let limits = Limits {
            infinite: true,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, Color::Black, 1, &TimeOptions::default());
        // 無制限では時刻による停止をしない
        assert!(!tm.use_time_management());
        assert!(!tm.over_total(1.0));
    }

    #[test]
    fn movetime_is_fixed() {
        let limits = Limits {
            movetime: 1000,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, Color::White, 1, &TimeOptions::default());
        // 参照実装はmovetimeからNetworkDelayを引かない（T:189-193）。
        // 超過の判定はcheck_timeがlimits.movetimeと直接比べる
        assert_eq!(tm.optimum(), 1000);
        assert_eq!(tm.maximum(), 1000);
        assert_eq!(tm.minimum(), 1000);
        assert!(!tm.use_time_management());
    }

    #[test]
    fn round_up_lifts_to_full_second() {
        let limits = Limits {
            btime: 300_000,
            binc: 10_000,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, Color::Black, 1, &TimeOptions::default());
        // 1200ms → 2000msへ繰り上げ、NetworkDelayの120msを引く
        assert_eq!(tm.round_up(1200), 1880);
        // 2000ms → 2000ms、120msを引くと元より小さいのでもう1秒使う
        assert_eq!(tm.round_up(2000), 2880);
        // MinimumThinkingTimeが下限として働く
        assert_eq!(tm.round_up(10), 1880);
    }

    #[test]
    fn search_end_respects_minimum() {
        let limits = Limits {
            btime: 300_000,
            binc: 10_000,
            ..Limits::default()
        };
        let mut tm = TimeManager::new(&limits, Color::Black, 1, &TimeOptions::default());
        assert_eq!(tm.search_end, 0);
        // 経過時間が最小思考時間より短くても、最小思考時間まで予約する
        tm.set_search_end(300);
        assert_eq!(tm.search_end, 1880);
    }
}
