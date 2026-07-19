//! 常駐探索スレッド群（ADR-0020, 0031）。
//!
//! Lazy SMP: 全ワーカーが同じ局面をTT共有で探索する。多様化は
//! せず（ADR-0031の案B）、TT到着順の揺らぎに任せる。
//! メインワーカー（index 0）だけが時間管理・info出力・bestmoveを
//! 担い、ヘルパーはstopフラグに従って止まる。
//! history等のスレッドローカル状態は対局を通じて各スレッドに保持する。

use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use himawari_core::Position;

use crate::eval::Evaluator;
use crate::movepick::{CounterMoves, History};
use crate::search::{Shared, Worker};
use crate::timeman::{Limits, TimeManager};
use crate::value::{VALUE_MATE, Value};

/// メインワーカーへのUSI出力コールバック。
pub type OnLine = Arc<dyn Fn(&str) + Send + Sync>;

/// エンジン設定（setoption由来）。
#[derive(Clone, Debug)]
pub struct EngineOptions {
    pub hash_mb: usize,
    pub threads: usize,
    pub network_delay: u64,
    pub network_delay2: u64,
    pub max_moves_to_draw: u16,
    pub multi_pv: usize,
}

impl Default for EngineOptions {
    fn default() -> Self {
        EngineOptions {
            hash_mb: 256,
            threads: 1,
            network_delay: 120,
            network_delay2: 1120,
            max_moves_to_draw: 0,
            multi_pv: 1,
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

struct WorkerThread {
    ctl: Arc<Ctl>,
    handle: Option<JoinHandle<()>>,
}

pub struct ThreadPool {
    workers: Vec<WorkerThread>,
    shared: Arc<Shared>,
    /// 生成時のパラメータ（isreadyでの再生成判定用）。
    pub hash_mb: usize,
    pub threads: usize,
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

fn spawn_worker(
    shared: Arc<Shared>,
    is_main: bool,
    on_line: Option<OnLine>,
) -> WorkerThread {
    let ctl = Arc::new(Ctl {
        job: Mutex::new(None),
        cv: Condvar::new(),
        idle: Mutex::new(true),
        idle_cv: Condvar::new(),
    });
    let ctl2 = Arc::clone(&ctl);
    let handle = std::thread::spawn(move || {
        // スレッドローカル状態（対局を通じて保持。ADR-0020）
        let mut history = History::default();
        let mut counters = CounterMoves::default();
        loop {
            let job = {
                let mut guard = ctl2.job.lock().expect("job lock");
                loop {
                    if let Some(job) = guard.take() {
                        // 取り出しを同期待ちしている側（new_game）に知らせる
                        ctl2.cv.notify_all();
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
                }
                Job::Search(j) => {
                    // ヘルパーは時間・ノード制限を持たずstopフラグで止まる
                    let (limits, tm) = if is_main {
                        let tm = TimeManager::new(
                            &j.limits,
                            j.pos.side_to_move(),
                            j.pos.game_ply(),
                            j.opts.network_delay,
                            j.opts.network_delay2,
                        );
                        (j.limits.clone(), tm)
                    } else {
                        let inf = Limits {
                            infinite: true,
                            nodes: 0,
                            movetime: 0,
                            depth: j.limits.depth,
                            ..Limits::default()
                        };
                        let tm = TimeManager::new(
                            &inf,
                            j.pos.side_to_move(),
                            j.pos.game_ply(),
                            0,
                            0,
                        );
                        (inf, tm)
                    };
                    let mut worker = Worker::new(
                        j.pos,
                        Arc::clone(&shared),
                        limits,
                        tm,
                        j.opts.max_moves_to_draw,
                        j.opts.multi_pv,
                        Evaluator::material(),
                        std::mem::take(&mut history),
                        std::mem::take(&mut counters),
                    );
                    let result = worker.iterate(&mut |info| {
                        let Some(out) = &on_line else { return };
                        let pv: Vec<String> = info.pv.iter().map(|m| m.to_usi()).collect();
                        let nps = (info.nodes * 1000)
                            .checked_div(info.elapsed_ms)
                            .unwrap_or(0);
                        // MultiPV>1のときだけmultipvを出す（現行互換）
                        let mpv = if info.multipv > 0 {
                            format!("multipv {} ", info.multipv)
                        } else {
                            String::new()
                        };
                        out(&format!(
                            "info depth {} {}score {} nodes {} nps {} time {} hashfull {} pv {}",
                            info.depth,
                            mpv,
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
                    if is_main {
                        // メインの結論が出たらヘルパーも止める
                        shared.stop.store(true, Ordering::Relaxed);
                        if let Some(out) = &on_line {
                            out(&format!("bestmove {}", result.best.to_usi()));
                        }
                    }
                    let mut idle = ctl2.idle.lock().expect("idle lock");
                    *idle = true;
                    ctl2.idle_cv.notify_all();
                }
            }
        }
    });
    WorkerThread {
        ctl,
        handle: Some(handle),
    }
}

impl ThreadPool {
    /// on_lineはメインワーカーからのUSI出力行（info/bestmove）を受け取る。
    pub fn new(hash_mb: usize, threads: usize, on_line: OnLine) -> ThreadPool {
        let shared = Arc::new(Shared::new(hash_mb));
        let n = threads.max(1);
        let workers = (0..n)
            .map(|i| {
                spawn_worker(
                    Arc::clone(&shared),
                    i == 0,
                    if i == 0 { Some(Arc::clone(&on_line)) } else { None },
                )
            })
            .collect();
        ThreadPool {
            workers,
            shared,
            hash_mb,
            threads: n,
        }
    }

    pub fn go(&self, pos: Position, limits: Limits, opts: EngineOptions) {
        self.wait_idle();
        self.shared.stop.store(false, Ordering::Relaxed);
        self.shared.nodes.store(0, Ordering::Relaxed);
        for w in &self.workers {
            // idleはgo側で同期的に下ろす。workerが起きる前にquit/stopが
            // 来ても探索ジョブが破棄されない（bestmoveを必ず返す）
            *w.ctl.idle.lock().expect("idle lock") = false;
            let mut guard = w.ctl.job.lock().expect("job lock");
            *guard = Some(Job::Search(Box::new(SearchJob {
                pos: pos.clone(),
                limits: limits.clone(),
                opts: opts.clone(),
            })));
            w.ctl.cv.notify_all();
        }
    }

    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }

    /// 対局間のリセット。TTは1回だけ消し、各ワーカーのhistoryを消す。
    /// ジョブが取り出されるまで待つので、直後のgoに上書きされない。
    pub fn new_game(&self) {
        self.wait_idle();
        self.shared.tt.clear();
        for w in &self.workers {
            let mut guard = w.ctl.job.lock().expect("job lock");
            *guard = Some(Job::NewGame);
            w.ctl.cv.notify_all();
            while guard.is_some() {
                guard = w.ctl.cv.wait(guard).expect("job wait");
            }
        }
    }

    /// 探索が終わる（bestmoveを出す）まで待つ。
    pub fn wait_idle(&self) {
        for w in &self.workers {
            let mut idle = w.ctl.idle.lock().expect("idle lock");
            while !*idle {
                idle = w.ctl.idle_cv.wait(idle).expect("idle wait");
            }
        }
    }

    pub fn quit(mut self) {
        self.stop();
        self.wait_idle();
        for w in &mut self.workers {
            {
                let mut guard = w.ctl.job.lock().expect("job lock");
                *guard = Some(Job::Quit);
                w.ctl.cv.notify_all();
            }
            if let Some(h) = w.handle.take() {
                let _ = h.join();
            }
        }
    }
}
