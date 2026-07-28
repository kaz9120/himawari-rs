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
use crate::movepick::{ContinuationHistory, CorrectionHistory, CounterMoves, History};
use crate::nnue::NnueNetwork;
use crate::search::{SearchInfo, Shared, Worker};
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
    pub ponder: bool,
    pub eval_file: String,
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
            ponder: false,
            eval_file: String::new(),
        }
    }
}

struct SearchJob {
    pos: Position,
    limits: Limits,
    opts: EngineOptions,
    ponder: bool,
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

/// ponder中のbestmove保留状態（ADR-0033）。
/// go ponder中は探索が終わってもbestmoveを出さず、ponderhit/stopの
/// 解決を待つ（2手指し防御）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PonderState {
    /// ponderしていない（通常探索）。
    None,
    /// ponder探索中。
    Searching,
    /// ponder探索が自然終了し、bestmoveを保留して待機中。
    FinishedHolding,
    /// ponderhitで実時間探索へ切替（探索中なら無音キャンセル）。
    Hit,
    /// stopされた（bestmoveを出してよい）。
    Stopped,
}

struct PonderCtl {
    state: Mutex<PonderState>,
    cv: Condvar,
}

struct WorkerThread {
    ctl: Arc<Ctl>,
    handle: Option<JoinHandle<()>>,
}

pub struct ThreadPool {
    workers: Vec<WorkerThread>,
    shared: Arc<Shared>,
    ponder: Arc<PonderCtl>,
    /// ponderhit時に実時間で再起動するためのジョブ控え。
    pending: Mutex<Option<SearchJob>>,
    /// 生成時のパラメータ（isreadyでの再生成判定用）。
    pub hash_mb: usize,
    pub threads: usize,
    /// 読み込み済みの評価関数の識別（EvalFileパス）。
    pub eval_file: String,
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
    ponder: Arc<PonderCtl>,
    net: Option<Arc<NnueNetwork>>,
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
        let mut corr = CorrectionHistory::default();
        // 約13.4MB（ADR-0047）。mem::takeの往復でgoごとに空テーブルの
        // 生成が入るが、ゼロ初期化は数msでtc 10+0.1でも無視できる
        let mut cont = ContinuationHistory::default();
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
                    corr.clear();
                    cont.clear();
                }
                Job::Search(j) => {
                    // ヘルパーとponder探索は時間制限を持たずstopフラグで止まる
                    let (limits, tm) = if is_main && !j.ponder {
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
                        let tm =
                            TimeManager::new(&inf, j.pos.side_to_move(), j.pos.game_ply(), 0, 0);
                        (inf, tm)
                    };
                    let was_ponder = j.ponder;
                    let evaluator = match &net {
                        Some(n) => Evaluator::nnue(Arc::clone(n)),
                        None => Evaluator::material(),
                    };
                    let mut worker = Worker::new(
                        j.pos,
                        Arc::clone(&shared),
                        limits,
                        tm,
                        j.opts.max_moves_to_draw,
                        j.opts.multi_pv,
                        evaluator,
                        std::mem::take(&mut history),
                        std::mem::take(&mut counters),
                        std::mem::take(&mut corr),
                        std::mem::take(&mut cont),
                    );
                    let result = worker.iterate(&mut |info| {
                        let Some(out) = &on_line else { return };
                        match info {
                            SearchInfo::CurrMove { depth, mv } => {
                                out(&format!(
                                    "info depth {} currmove {}",
                                    depth,
                                    mv.to_usi()
                                ));
                            }
                            SearchInfo::Iteration(info) => {
                                let pv: Vec<String> =
                                    info.pv.iter().map(|m| m.to_usi()).collect();
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
                                    "info depth {} seldepth {} {}score {} nodes {} nps {} time {} hashfull {} pv {}",
                                    info.depth,
                                    info.seldepth,
                                    mpv,
                                    format_score(info.score),
                                    info.nodes,
                                    nps,
                                    info.elapsed_ms,
                                    info.hashfull,
                                    pv.join(" ")
                                ));
                            }
                        }
                    });
                    // history類を回収して次のgoへ持ち越す
                    history = std::mem::take(&mut worker.history);
                    counters = std::mem::take(&mut worker.counters);
                    corr = std::mem::take(&mut worker.corr);
                    cont = std::mem::take(&mut worker.cont);
                    if is_main {
                        // メインの結論が出たらヘルパーも止める
                        shared.stop.store(true, Ordering::Relaxed);
                        // ponder中はbestmoveを保留し、ponderhit/stopの解決を
                        // 待つ（ADR-0033の2手指し防御）
                        let emit = if was_ponder {
                            let mut st = ponder.state.lock().expect("ponder lock");
                            if *st == PonderState::Searching {
                                *st = PonderState::FinishedHolding;
                                ponder.cv.notify_all();
                                while *st == PonderState::FinishedHolding {
                                    st = ponder.cv.wait(st).expect("ponder wait");
                                }
                            }
                            // Hit（無音キャンセル→実時間で再起動）ならbestmoveを
                            // 出さない。Stoppedなら出す
                            *st == PonderState::Stopped
                        } else {
                            true
                        };
                        if emit && let Some(out) = &on_line {
                            let ponder_hint =
                                if j.opts.ponder && result.ponder != himawari_core::Move::NONE {
                                    format!(" ponder {}", result.ponder.to_usi())
                                } else {
                                    String::new()
                                };
                            out(&format!("bestmove {}{}", result.best.to_usi(), ponder_hint));
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
    /// netがSomeならNNUE評価、Noneなら駒割評価で探索する。
    pub fn new(
        hash_mb: usize,
        threads: usize,
        net: Option<(String, Arc<NnueNetwork>)>,
        on_line: OnLine,
    ) -> ThreadPool {
        let shared = Arc::new(Shared::new(hash_mb));
        let ponder = Arc::new(PonderCtl {
            state: Mutex::new(PonderState::None),
            cv: Condvar::new(),
        });
        let (eval_file, net_arc) = match net {
            Some((path, n)) => (path, Some(n)),
            None => (String::new(), None),
        };
        let n = threads.max(1);
        let workers = (0..n)
            .map(|i| {
                spawn_worker(
                    Arc::clone(&shared),
                    Arc::clone(&ponder),
                    net_arc.clone(),
                    i == 0,
                    if i == 0 {
                        Some(Arc::clone(&on_line))
                    } else {
                        None
                    },
                )
            })
            .collect();
        ThreadPool {
            workers,
            shared,
            ponder,
            pending: Mutex::new(None),
            hash_mb,
            threads: n,
            eval_file,
        }
    }

    pub fn go(&self, pos: Position, limits: Limits, opts: EngineOptions) {
        self.dispatch(pos, limits, opts, false);
    }

    /// go ponder（ADR-0033）。時間制限なしで探索し、bestmoveは
    /// ponderhit/stopまで保留される。
    pub fn go_ponder(&self, pos: Position, limits: Limits, opts: EngineOptions) {
        *self.pending.lock().expect("pending lock") = Some(SearchJob {
            pos: pos.clone(),
            limits: limits.clone(),
            opts: opts.clone(),
            ponder: false,
        });
        self.dispatch(pos, limits, opts, true);
    }

    fn dispatch(&self, pos: Position, limits: Limits, opts: EngineOptions, ponder: bool) {
        self.wait_idle();
        *self.ponder.state.lock().expect("ponder lock") = if ponder {
            PonderState::Searching
        } else {
            PonderState::None
        };
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
                ponder,
            })));
            w.ctl.cv.notify_all();
        }
    }

    /// ponderhit（ADR-0033）。探索中なら無音キャンセルして実時間で
    /// 再起動する（TTが木を即復元する）。保留中なら即bestmove。
    pub fn ponderhit(&self) {
        let relaunch = {
            let mut st = self.ponder.state.lock().expect("ponder lock");
            match *st {
                PonderState::Searching => {
                    *st = PonderState::Hit;
                    self.shared.stop.store(true, Ordering::Relaxed);
                    true
                }
                PonderState::FinishedHolding => {
                    *st = PonderState::Stopped;
                    self.ponder.cv.notify_all();
                    false
                }
                _ => false,
            }
        };
        if relaunch {
            self.wait_idle();
            if let Some(job) = self.pending.lock().expect("pending lock").take() {
                self.dispatch(job.pos, job.limits, job.opts, false);
            }
        }
    }

    pub fn stop(&self) {
        {
            let mut st = self.ponder.state.lock().expect("ponder lock");
            match *st {
                PonderState::Searching | PonderState::FinishedHolding => {
                    *st = PonderState::Stopped;
                    self.ponder.cv.notify_all();
                }
                _ => {}
            }
        }
        self.shared.stop.store(true, Ordering::Relaxed);
    }

    /// 対局間のリセット。TTは1回だけ消し、各ワーカーのhistoryを消す。
    /// ジョブが取り出されるまで待つので、直後のgoに上書きされない。
    pub fn new_game(&self) {
        self.wait_idle();
        self.shared.tt.clear();
        self.shared.eval_hash.clear();
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
