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
use std::time::Instant;

use himawari_core::{Move, Position};

use crate::eval::Evaluator;
use crate::movepick::Histories;
use crate::nnue::NnueNetwork;
use crate::search::{IterInfo, MainMemory, ScoreBound, SearchInfo, SearchResult, Shared, Worker};
use crate::timeman::{Limits, TimeManager, TimeOptions};
use crate::value::{
    VALUE_INFINITE, VALUE_MATE, VALUE_MATE_IN_MAX_PLY, VALUE_MATED_IN_MAX_PLY, VALUE_ZERO, Value,
};

/// メインワーカーへのUSI出力コールバック。
pub type OnLine = Arc<dyn Fn(&str) + Send + Sync>;

/// エンジン設定（setoption由来）。
#[derive(Clone, Debug)]
pub struct EngineOptions {
    pub hash_mb: usize,
    pub threads: usize,
    pub network_delay: u64,
    pub network_delay2: u64,
    /// 最小思考時間[ms]（timeman.cpp:47）
    pub minimum_thinking_time: u64,
    /// 序盤重視率。百分率（timeman.cpp:52）
    pub slow_mover: u64,
    /// 持ち時間の各秒をぎりぎりまで使うか（timeman.cpp:55）
    pub round_up_to_full_second: bool,
    pub max_moves_to_draw: u16,
    pub multi_pv: usize,
    pub ponder: bool,
    /// 投了スコア（S:155）。GUIへ出す評価値がこの値の符号違いを
    /// 下回ったら投了する。既定99999は「投了しない」の意味
    pub resign_value: i32,
    /// 先手番のときの引き分けの評価値（S:151）。歩を100とした百分率で、
    /// 既定の-2は千日手をわずかに嫌う設定である
    pub draw_value_black: i32,
    /// 後手番のときの引き分けの評価値（S:152）
    pub draw_value_white: i32,
    pub eval_file: String,
}

impl Default for EngineOptions {
    fn default() -> Self {
        EngineOptions {
            hash_mb: 256,
            threads: 1,
            network_delay: 120,
            network_delay2: 1120,
            minimum_thinking_time: 2000,
            slow_mover: 100,
            round_up_to_full_second: true,
            max_moves_to_draw: 0,
            multi_pv: 1,
            ponder: false,
            resign_value: 99999,
            draw_value_black: -2,
            draw_value_white: -2,
            eval_file: String::new(),
        }
    }
}

impl EngineOptions {
    /// 時間管理へ渡す値を切り出す。`MaxMovesToDraw` の0は指定なしの
    /// 意味なので100000として扱う（yaneuraou-search.cpp:72-77）
    pub fn time_options(&self) -> TimeOptions {
        TimeOptions {
            network_delay: self.network_delay as i64,
            network_delay2: self.network_delay2 as i64,
            minimum_thinking_time: self.minimum_thinking_time as i64,
            slow_mover: self.slow_mover as i64,
            round_up_to_full_second: self.round_up_to_full_second,
            max_moves_to_draw: if self.max_moves_to_draw == 0 {
                100_000
            } else {
                i64::from(self.max_moves_to_draw)
            },
            ponder: self.ponder,
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

/// ponder中のbestmove保留状態（ADR-0033、ADR-0109のG8）。
/// go ponder中は探索が終わってもbestmoveを出さず、ponderhit/stopの
/// 解決を待つ（2手指し防御。S:1162-1187）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PonderState {
    /// ponderしていない（通常探索）。
    None,
    /// ponder探索中。bestmoveは保留する。
    Searching,
    /// ponder探索が自然終了し、bestmoveを保留して待機中。
    FinishedHolding,
    /// ponderhitを受けた。探索は止めずに続け、終わったらbestmoveを出す。
    Hit,
    /// stopされた（bestmoveを出してよい）。
    Stopped,
}

struct PonderCtl {
    state: Mutex<PonderState>,
    cv: Condvar,
}

/// 各スレッドの結論を集める場所（G10）。best thread votingが読む。
/// メインは全スレッドが書き終わるのを待ってから投票する
/// （S:1195-1197の `wait_for_search_finished`）。
struct Results {
    slots: Mutex<Vec<Option<SearchResult>>>,
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
    /// 各スレッドの結論（G10）。goのたびに空へ戻す
    results: Arc<Results>,
    /// 今回のgoの受領時刻。ponderhitの時刻をここからの経過へ換算する。
    start: Mutex<Instant>,
    /// 生成時のパラメータ（isreadyでの再生成判定用）。
    pub hash_mb: usize,
    pub threads: usize,
    /// 読み込み済みの評価関数の識別（EvalFileパス）。
    pub eval_file: String,
}

/// USIのscore表記（cp / mate）を組み立てる。
/// USIのinfo行を組み立てる（ADR-0086, 0091）。suffixは `lowerbound` などの
/// スコア修飾子で、確定値なら空文字を渡す。
fn format_pv_line(info: &IterInfo, score_suffix: &str) -> String {
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
    format!(
        "info depth {} seldepth {} {}score {}{} nodes {} nps {} time {} hashfull {} pv {}",
        info.depth,
        info.seldepth,
        mpv,
        format_score(info.score),
        score_suffix,
        info.nodes,
        nps,
        info.elapsed_ms,
        info.hashfull,
        pv.join(" ")
    )
}

/// 詰み確定のスコアか（types.h:513-516の `is_win`）。
#[inline]
fn is_win(v: Value) -> bool {
    v >= VALUE_MATE_IN_MAX_PLY
}

/// 詰まされ確定のスコアか（types.h:519-522の `is_loss`）。
#[inline]
fn is_loss(v: Value) -> bool {
    v <= VALUE_MATED_IN_MAX_PLY
}

/// 並列探索でいちばん良い思考をしたスレッドを選ぶ（S:599-671の
/// `get_best_thread`）。呼び出し側は全スレッドのPVが空でないことを保証する。
///
/// 得票は「最小スコアからの差に14を足した値 × 確定深さ」の合計で、同じ手を
/// 選んだスレッドの分を足し合わせる。優先順位は3段ある。勝ち確定なら短い
/// 詰みへ、負け確定なら短い詰まされへ、それ以外は得票数で選ぶ。
fn get_best_thread(results: &[SearchResult]) -> usize {
    // 全スレッドのスコアの最小値（S:609-611）
    let min_score = results
        .iter()
        .map(|r| r.root_score)
        .min()
        .unwrap_or(VALUE_INFINITE);
    // スコアと深さで投票する（S:613-618）
    let voting_value = |r: &SearchResult| -> i64 {
        i64::from(r.root_score - min_score + 14) * i64::from(r.completed_depth)
    };
    // Moveはハッシュを持たないので連想リストで数える。スレッド数は
    // 高々数十なので線形探索で足りる
    let mut votes: Vec<(Move, i64)> = Vec::with_capacity(results.len());
    for r in results {
        let v = voting_value(r);
        match votes.iter_mut().find(|(m, _)| *m == r.pv[0]) {
            Some((_, acc)) => *acc += v,
            None => votes.push((r.pv[0], v)),
        }
    }
    let vote_of = |m: Move| -> i64 {
        votes
            .iter()
            .find(|(k, _)| *k == m)
            .map(|(_, v)| *v)
            .unwrap_or(0)
    };
    let mut best = 0usize;
    for (i, r) in results.iter().enumerate() {
        let b = &results[best];
        let best_score = b.root_score;
        let new_score = r.root_score;
        let best_vote = vote_of(b.pv[0]);
        let new_vote = vote_of(r.pv[0]);
        let best_in_win = is_win(best_score);
        let new_in_win = is_win(new_score);
        // -VALUE_INFINITEはまだ値がない番兵なので負け確定とは見ない
        let best_in_loss = best_score != -VALUE_INFINITE && is_loss(best_score);
        let new_in_loss = new_score != -VALUE_INFINITE && is_loss(new_score);
        // PVが2手以下のスレッドは投票値を0扱いにする（S:643-646）。
        // 途中で切れたPVを持つスレッドを選ばないための細工である
        let better_voting_value = voting_value(r) * i64::from(r.pv.len() > 2)
            > voting_value(b) * i64::from(b.pv.len() > 2);
        if best_in_win {
            // 詰みは短いほうを選ぶ（S:648-653）
            if new_score > best_score {
                best = i;
            }
        } else if best_in_loss {
            // 詰まされる側も短いほうを選ぶ（S:654-659）。
            // 原典のコメントは "pick the shortest mated" で、
            // より小さいスコア（＝早く詰まされる）を採るコードと一致する
            if new_in_loss && new_score < best_score {
                best = i;
            }
        } else if new_in_win
            || new_in_loss
            || (!is_loss(new_score)
                && (new_vote > best_vote || (new_vote == best_vote && better_voting_value)))
        {
            best = i;
        }
    }
    best
}

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
    results: Arc<Results>,
    net: Option<Arc<NnueNetwork>>,
    thread_idx: usize,
    thread_count: usize,
    on_line: Option<OnLine>,
) -> WorkerThread {
    let is_main = thread_idx == 0;
    let ctl = Arc::new(Ctl {
        job: Mutex::new(None),
        cv: Condvar::new(),
        idle: Mutex::new(true),
        idle_cv: Condvar::new(),
    });
    let ctl2 = Arc::clone(&ctl);
    let handle = std::thread::spawn(move || {
        // スレッドローカル状態（対局を通じて保持。ADR-0020, 0109）。
        // 約100MiBあるので、goごとに作り直さずWorkerへ渡して回収する
        let mut hist = Histories::default();
        // goをまたぐ記憶（G9）。参照実装がSearchManagerに置く値で、
        // 対局開始時にだけクリアする
        let mut memory = MainMemory::default();
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
                    hist.clear();
                    memory = MainMemory::default();
                }
                Job::Search(j) => {
                    // ヘルパーは時間制限を持たずstopフラグで止まる。
                    // メインはgo ponderでも実際の持ち時間で時間管理する
                    // （S:960-975。参照実装の `use_time_management()` は
                    // ponderMode を見ない）。ponder中に止めないのは
                    // check_limitsのponderガードが担う（S:5502-5507）
                    let (limits, tm) = if is_main {
                        let tm = TimeManager::new(
                            &j.limits,
                            j.pos.side_to_move(),
                            j.pos.game_ply(),
                            &j.opts.time_options(),
                        );
                        (j.limits.clone(), tm)
                    } else {
                        let inf = Limits {
                            infinite: true,
                            nodes: 0,
                            movetime: 0,
                            depth: j.limits.depth,
                            // 計時の起点はgo受領時刻のまま引き継ぐ
                            start: j.limits.start,
                            ..Limits::default()
                        };
                        let tm = TimeManager::new(
                            &inf,
                            j.pos.side_to_move(),
                            j.pos.game_ply(),
                            &TimeOptions::default(),
                        );
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
                        hist,
                    );
                    worker.set_thread(thread_idx, thread_count);
                    worker.set_draw_value(j.opts.draw_value_black, j.opts.draw_value_white);
                    worker.memory = memory;
                    let result = worker.iterate(&mut |info| {
                        let Some(out) = &on_line else { return };
                        match info {
                            SearchInfo::CurrMove { depth, mv } => {
                                out(&format!("info depth {} currmove {}", depth, mv.to_usi()));
                            }
                            SearchInfo::Iteration(info) => {
                                out(&format_pv_line(&info, ""));
                            }
                            // fail high/lowは確定値でないことを示す（ADR-0091）
                            SearchInfo::Bound(info, b) => {
                                let suffix = match b {
                                    ScoreBound::Lower => " lowerbound",
                                    ScoreBound::Upper => " upperbound",
                                };
                                out(&format_pv_line(&info, suffix));
                            }
                        }
                    });
                    // history類とgoをまたぐ記憶を回収して次のgoへ持ち越す
                    hist = worker.hist;
                    memory = worker.memory;
                    // 自分の結論を投票用に置く（G10）
                    {
                        let mut g = results.slots.lock().expect("results lock");
                        g[thread_idx] = Some(result);
                        results.cv.notify_all();
                    }
                    if is_main {
                        // メインの結論が出たらヘルパーも止める
                        shared.stop.store(true, Ordering::Relaxed);
                        // go ponder中に探索が自然終了したら、ponderhit/stopが
                        // 来るまでbestmoveを保留する（2手指し防御。S:1162-1187の
                        // `while (!threads.stop && (ponder || limits.infinite))`）
                        if was_ponder {
                            let mut st = ponder.state.lock().expect("ponder lock");
                            if *st == PonderState::Searching {
                                *st = PonderState::FinishedHolding;
                                ponder.cv.notify_all();
                                while *st == PonderState::FinishedHolding {
                                    st = ponder.cv.wait(st).expect("ponder wait");
                                }
                            }
                        }
                        // 全スレッドの結論が揃うのを待つ（S:1195-1197の
                        // `wait_for_search_finished`）。stopは既に立てたので
                        // ヘルパーはすぐ抜ける
                        let all: Vec<SearchResult> = {
                            let mut g = results.slots.lock().expect("results lock");
                            while g.iter().any(|r| r.is_none()) {
                                g = results.cv.wait(g).expect("results wait");
                            }
                            g.iter_mut().filter_map(|r| r.take()).collect()
                        };
                        // 投票で最終手を選ぶ（S:1239-1246）。MultiPVや
                        // go depthのときは参照実装も投票しない
                        let chosen = if thread_count > 1
                            && j.opts.multi_pv == 1
                            && j.limits.depth == 0
                            && all.iter().all(|r| !r.pv.is_empty())
                        {
                            get_best_thread(&all)
                        } else {
                            0
                        };
                        let result = &all[chosen];
                        if chosen != 0 {
                            // 次のgoへ持ち越す値もbest threadのものにする
                            // （S:1249-1253）
                            memory.best_previous_score = result.root_score;
                            memory.best_previous_average_score = result.root_average_score;
                        }
                        if let Some(out) = &on_line {
                            // メイン以外が選ばれたら、そのスレッドのPVを
                            // 出し直す（S:1332-1348）。既に出したPVは
                            // メインのものなので指し手と食い違う
                            if chosen != 0 {
                                out(&format!("info string best thread = {chosen}"));
                                out(&format_pv_line(
                                    &IterInfo {
                                        depth: result.completed_depth,
                                        seldepth: result.completed_depth,
                                        multipv: 0,
                                        score: result.score,
                                        pv: result.pv.clone(),
                                        nodes: shared.nodes.load(Ordering::Relaxed),
                                        elapsed_ms: j
                                            .limits
                                            .start
                                            .map(|s| s.elapsed().as_millis() as u64)
                                            .unwrap_or(0),
                                        hashfull: shared.tt.hashfull(),
                                    },
                                    "",
                                ));
                            }
                            // 投了スコアを下回っていたら投了する
                            // （S:1289-1298、S:1337-1342）。定跡ヒットなどの
                            // search_skipped経路では判定しないが、本エンジンは
                            // 定跡をUSI層で引くのでここは常に通常探索である
                            let resign_score = if result.score == -VALUE_INFINITE {
                                VALUE_ZERO
                            } else {
                                result.score
                            };
                            if result.root_score != -VALUE_INFINITE
                                && resign_score <= -j.opts.resign_value
                            {
                                out(&format!(
                                    "info string resign by ResignValue: score {resign_score}"
                                ));
                                out("bestmove resign");
                            } else {
                                // ponderhitでも探索を継続するので、ここで得た結論が
                                // そのまま本番の結論になる。常に出してよい
                                let ponder_hint = if j.opts.ponder
                                    && result.ponder != himawari_core::Move::NONE
                                {
                                    format!(" ponder {}", result.ponder.to_usi())
                                } else {
                                    String::new()
                                };
                                out(&format!("bestmove {}{}", result.best.to_usi(), ponder_hint));
                            }
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
        let results = Arc::new(Results {
            slots: Mutex::new((0..n).map(|_| None).collect()),
            cv: Condvar::new(),
        });
        let workers = (0..n)
            .map(|i| {
                spawn_worker(
                    Arc::clone(&shared),
                    Arc::clone(&ponder),
                    Arc::clone(&results),
                    net_arc.clone(),
                    i,
                    n,
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
            results,
            start: Mutex::new(Instant::now()),
            hash_mb,
            threads: n,
            eval_file,
        }
    }

    pub fn go(&self, pos: Position, limits: Limits, opts: EngineOptions) {
        self.dispatch(pos, limits, opts, false);
    }

    /// go ponder（ADR-0033、ADR-0109のG8）。実際の持ち時間で時間管理しつつ
    /// 時間では止まらず、bestmoveはponderhit/stopまで保留される。
    pub fn go_ponder(&self, pos: Position, limits: Limits, opts: EngineOptions) {
        self.dispatch(pos, limits, opts, true);
    }

    fn dispatch(&self, pos: Position, limits: Limits, opts: EngineOptions, ponder: bool) {
        self.wait_idle();
        *self.ponder.state.lock().expect("ponder lock") = if ponder {
            PonderState::Searching
        } else {
            PonderState::None
        };
        // 計時の起点をponderhitの換算用に控える
        *self.start.lock().expect("start lock") = limits.start.unwrap_or_else(Instant::now);
        // 前回の結論を捨てる（G10）。wait_idle済みなので書き手はいない
        for slot in self.results.slots.lock().expect("results lock").iter_mut() {
            *slot = None;
        }
        self.shared.stop.store(false, Ordering::Relaxed);
        self.shared.aborted_search.store(false, Ordering::Relaxed);
        // go受領時点の初期化（S:114-120 pre_start_searching）
        self.shared.ponder.store(ponder, Ordering::SeqCst);
        self.shared.ponderhit_offset.store(0, Ordering::SeqCst);
        self.shared.nodes.store(0, Ordering::Relaxed);
        self.shared.increase_depth.store(true, Ordering::Relaxed);
        self.shared.best_move_changes.store(0, Ordering::Relaxed);
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

    /// ponderhit（ADR-0109のG8）。**探索は止めない。** ponderフラグを
    /// 下ろすだけで、探索スレッドは次の判定から時間で止まるようになる
    /// （S:299-308）。保留中ならその場で解放してbestmoveを出させる。
    pub fn ponderhit(&self) {
        // ponderhitの時刻を先に書き、そのあとponderフラグを下ろす。
        // 順序が逆だと、他スレッドがponderフラグを見て古いponderhitTimeで
        // 計算してしまう（S:299-308の原典コメント）
        let off = self.start.lock().expect("start lock").elapsed().as_millis() as i64;
        self.shared.ponderhit_offset.store(off, Ordering::SeqCst);
        self.shared.ponder.store(false, Ordering::SeqCst);
        let mut st = self.ponder.state.lock().expect("ponder lock");
        match *st {
            // 探索中。止めずに続け、終わったらbestmoveを出す
            PonderState::Searching => *st = PonderState::Hit,
            // 既に読み終えて保留している。解放する
            PonderState::FinishedHolding => {
                *st = PonderState::Hit;
                self.ponder.cv.notify_all();
            }
            _ => {}
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{mate_in, mated_in};

    // 投票の区別に使う適当な指し手。合法性は問わない
    const A: Move = Move::WIN;
    const B: Move = Move::RESIGN;

    fn res(score: Value, depth: u32, pv: Vec<Move>) -> SearchResult {
        SearchResult {
            best: pv[0],
            score,
            ponder: pv.get(1).copied().unwrap_or(Move::NONE),
            root_score: score,
            root_average_score: score,
            pv,
            completed_depth: depth,
        }
    }

    #[test]
    fn single_thread_picks_itself() {
        let all = vec![res(10, 12, vec![A, B, A])];
        assert_eq!(get_best_thread(&all), 0);
    }

    #[test]
    fn deeper_thread_wins_on_equal_score() {
        // 同じスコアなら確定深さの大きいほうが得票を伸ばす
        let all = vec![res(0, 10, vec![A, B, A]), res(0, 20, vec![B, A, B])];
        assert_eq!(get_best_thread(&all), 1);
    }

    #[test]
    fn shorter_mate_wins() {
        // 勝ち確定なら短い詰みを選ぶ（深さや得票では選ばない）
        let all = vec![
            res(mate_in(10), 30, vec![A, B, A]),
            res(mate_in(4), 5, vec![B, A, B]),
        ];
        assert_eq!(get_best_thread(&all), 1);
    }

    #[test]
    fn shortest_mated_line_wins() {
        // 負け確定ならスコアの小さいほう（＝早く詰まされるほう）を選ぶ。
        // 原典のコメントも "pick the shortest mated" で、コードと一致する
        let all = vec![
            res(mated_in(10), 20, vec![A, B, A]),
            res(mated_in(4), 20, vec![B, A, B]),
        ];
        assert_eq!(get_best_thread(&all), 1);
    }

    #[test]
    fn truncated_pv_loses_the_tie() {
        // 得票が同じなら、PVが2手以下のスレッドは選ばれない
        let short = res(0, 14, vec![A, B]);
        let long = res(0, 14, vec![B, A, B]);
        assert_eq!(get_best_thread(&[short, long]), 1);
        let short = res(0, 14, vec![B, A]);
        let long = res(0, 14, vec![A, B, A]);
        assert_eq!(get_best_thread(&[long, short]), 0);
    }

    #[test]
    fn same_move_accumulates_votes() {
        // 同じ手を選んだ2スレッドの票が合算され、単独の深いスレッドを上回る
        let all = vec![
            res(0, 12, vec![A, B, A]),
            res(0, 12, vec![A, B, A]),
            res(0, 20, vec![B, A, B]),
        ];
        assert_eq!(get_best_thread(&all), 0);
    }
}
