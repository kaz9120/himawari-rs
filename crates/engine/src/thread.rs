//! 常駐探索スレッド（ADR-0020）。
//!
//! P2はThreads=1運用。goで起こしてcondvarで待機に戻る。
//! history等のスレッドローカル状態は対局を通じてスレッド内に保持する。

use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use himawari_core::Position;

use crate::eval::Evaluator;
use crate::movepick::{CounterMoves, History};
use crate::search::{Shared, Worker};
use crate::timeman::{Limits, TimeManager};
use crate::value::{VALUE_MATE, Value};

/// エンジン設定（setoption由来）。
#[derive(Clone, Debug)]
pub struct EngineOptions {
    pub hash_mb: usize,
    pub threads: usize,
    pub network_delay: u64,
    pub network_delay2: u64,
    pub max_moves_to_draw: u16,
}

impl Default for EngineOptions {
    fn default() -> Self {
        EngineOptions {
            hash_mb: 256,
            threads: 1,
            network_delay: 120,
            network_delay2: 1120,
            max_moves_to_draw: 0,
        }
    }
}

struct SearchJob {
    pos: Position,
    limits: Limits,
    opts: EngineOptions,
}

enum Job {
    Search(Box<SearchJob>),
    NewGame,
    Quit,
}

struct Ctl {
    job: Mutex<Option<Job>>,
    cv: Condvar,
    idle: Mutex<bool>,
    idle_cv: Condvar,
}

pub struct ThreadPool {
    ctl: Arc<Ctl>,
    shared: Arc<Shared>,
    handle: Option<JoinHandle<()>>,
}

/// USIのscore表記（cp / mate）を組み立てる。
fn format_score(v: Value) -> String {
    if v.abs() >= VALUE_MATE - 256 {
        let plies = VALUE_MATE - v.abs();
        let signed = if v > 0 { plies } else { -plies };
        format!("mate {signed}")
    } else {
        format!("cp {v}")
    }
}

impl ThreadPool {
    /// on_lineは探索スレッドからのUSI出力行（info/bestmove）を受け取る。
    pub fn new(hash_mb: usize, on_line: Box<dyn Fn(&str) + Send>) -> ThreadPool {
        let ctl = Arc::new(Ctl {
            job: Mutex::new(None),
            cv: Condvar::new(),
            idle: Mutex::new(true),
            idle_cv: Condvar::new(),
        });
        let shared = Arc::new(Shared::new(hash_mb));
        let ctl2 = Arc::clone(&ctl);
        let shared2 = Arc::clone(&shared);
        let handle = std::thread::spawn(move || {
            // スレッドローカル状態（対局を通じて保持。ADR-0020）
            let mut history = History::default();
            let mut counters = CounterMoves::default();
            loop {
                let job = {
                    let mut guard = ctl2.job.lock().expect("job lock");
                    loop {
                        if let Some(job) = guard.take() {
                            break job;
                        }
                        guard = ctl2.cv.wait(guard).expect("job wait");
                    }
                };
                match job {
                    Job::Quit => break,
                    Job::NewGame => {
                        history.clear();
                        counters.clear();
                        shared2.tt.clear();
                    }
                    Job::Search(j) => {
                        let tm = TimeManager::new(
                            &j.limits,
                            j.pos.side_to_move(),
                            j.pos.game_ply(),
                            j.opts.network_delay,
                            j.opts.network_delay2,
                        );
                        let mut worker = Worker::new(
                            j.pos,
                            Arc::clone(&shared2),
                            j.limits,
                            tm,
                            j.opts.max_moves_to_draw,
                            Evaluator::material(),
                            std::mem::take(&mut history),
                            std::mem::take(&mut counters),
                        );
                        let result = worker.iterate(&mut |info| {
                            let pv: Vec<String> = info.pv.iter().map(|m| m.to_usi()).collect();
                            let nps = (info.nodes * 1000)
                                .checked_div(info.elapsed_ms)
                                .unwrap_or(0);
                            on_line(&format!(
                                "info depth {} score {} nodes {} nps {} time {} hashfull {} pv {}",
                                info.depth,
                                format_score(info.score),
                                info.nodes,
                                nps,
                                info.elapsed_ms,
                                info.hashfull,
                                pv.join(" ")
                            ));
                        });
                        // history類を回収して次のgoへ持ち越す
                        history = std::mem::take(&mut worker.history);
                        counters = std::mem::take(&mut worker.counters);
                        let best = result.best;
                        on_line(&format!("bestmove {}", best.to_usi()));
                        let mut idle = ctl2.idle.lock().expect("idle lock");
                        *idle = true;
                        ctl2.idle_cv.notify_all();
                    }
                }
            }
        });
        ThreadPool {
            ctl,
            shared,
            handle: Some(handle),
        }
    }

    pub fn go(&self, pos: Position, limits: Limits, opts: EngineOptions) {
        self.wait_idle();
        // idleはgo側で同期的に下ろす。workerが起きる前にquit/stopが来ても
        // 探索ジョブが破棄されない（bestmoveを必ず返す）
        *self.ctl.idle.lock().expect("idle lock") = false;
        self.shared.stop.store(false, Ordering::Relaxed);
        let mut guard = self.ctl.job.lock().expect("job lock");
        *guard = Some(Job::Search(Box::new(SearchJob { pos, limits, opts })));
        self.ctl.cv.notify_all();
    }

    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }

    pub fn new_game(&self) {
        self.wait_idle();
        let mut guard = self.ctl.job.lock().expect("job lock");
        *guard = Some(Job::NewGame);
        self.ctl.cv.notify_all();
    }

    /// 探索が終わる（bestmoveを出す）まで待つ。
    pub fn wait_idle(&self) {
        let mut idle = self.ctl.idle.lock().expect("idle lock");
        while !*idle {
            idle = self.ctl.idle_cv.wait(idle).expect("idle wait");
        }
    }

    pub fn quit(mut self) {
        self.stop();
        self.wait_idle();
        {
            let mut guard = self.ctl.job.lock().expect("job lock");
            *guard = Some(Job::Quit);
            self.ctl.cv.notify_all();
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
