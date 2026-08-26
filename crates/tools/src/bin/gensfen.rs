//! 自己対局で教師データを作る（ADR-0144）。
//!
//! 平手初期局面からランダムに数手指して開始局面を散らし、そこから終局まで
//! 一定深さで指し継ぐ。各局面の (局面, 評価値, 指し手, 手数) を控え、決着が
//! ついた時点で勝敗を書き込んでPSV（ADR-0038）へ流す。
//!
//! 使い方:
//! 対局を並列に回す。--threads は同時に指す対局の数で、探索そのものは
//! 1スレッドである。1局面ずつLazy SMPで探索する作りだと並列効率が上がらず、
//! 8スレッドで494局面/秒しか出なかった。置換表はワーカーごとに持つので、
//! --hash は頭数で割られる。
//!
//!   gensfen --out <path> --eval <hmwr> [--games 1000] [--depth 8]
//!           [--openings <sfen列挙>] [--random-plies 8] [--min-ply 8]
//!           [--max-moves 320] [--resign 3000] [--threads N] [--hash 256]
//!           [--seed 1] [--save-every 100] [--stop-file <path>]
//!
//! --openings は開始局面の列挙（1行1 SFEN、`sfen ` 接頭辞は付いていてもよい）
//! で、各対局の開始局面をここから一様に引く。訓練の分布を測定（SPRT・実戦）
//! の分布へ合わせるための入口になる。無指定なら平手から始める。
//!
//! --random-plies は開始局面を散らすためにランダムへ指す手数。同じ局面ばかり
//! 生成しても学習の役に立たない。--min-ply より前の局面は記録しない。乱数で
//! 指した区間には教師にできる評価値が付かないためである。
//!
//! --resign を超える評価値が出たら投了する。既定は0（投了しない）である。
//! 3000で打ち切ると終盤が丸ごと欠け、詰みスコアの局面が0.19%しか出ない
//! （hao_depth9は8.24%）。投了をやめると6.95%まで戻り、分布がhaoに近づく。
//!
//! 速度は落ちない。投了で対局を早く終えても、そのぶん新しい対局の序盤を
//! 作り直すので、局面あたりのコストは変わらない。
//!
//! 記録するのはqsearchの静止局面である（--quiet-plies、既定1）。評価関数が
//! 探索中に見るのは静止局面なので、教師の局面もそこへ揃える（ADR-0136）。
//! 後からpsv quietをかけるより工程が1つ減り、費用も小さい。0を渡すと
//! 静止化せずに指した局面をそのまま記録する。
//!
//! 評価値と勝敗はどちらも手番視点で書く（ADR-0136で踏んだ落とし穴）。
//! 静止化で奇数手進んだら符号を戻す。詰みスコアは素通しにする（ADR-0126）。
//!
//! 途中で止めるときは停止ファイルを置く（ADR-0123）。1局の切れ目で書き出して
//! 終わるので、途中まで生成した教師はそのまま使える。
//!
//!   touch data/train/gen.psv.stop

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use himawari_core::packed_sfen::{PSV_BYTES, PackedSfenValue, pack};
use himawari_core::{Color, Move16, MoveList, Position, Repetition, SFEN_STARTPOS, generate_legal};
use himawari_engine::eval::Evaluator;
use himawari_engine::movepick::Histories;
use himawari_engine::search::{Shared, Worker};
use himawari_engine::timeman::{TimeManager, TimeOptions};
use himawari_engine::{EngineOptions, Limits, ThreadPool};
use himawari_tools::stop_file::StopFile;

struct Config {
    out: String,
    eval: String,
    games: usize,
    depth: u32,
    openings: Option<String>,
    random_plies: usize,
    min_ply: u16,
    quiet_plies: usize,
    max_moves: usize,
    resign: i32,
    threads: usize,
    hash_mb: usize,
    seed: u64,
    save_every: usize,
    stop_file: Option<String>,
}

/// xorshift64*。開始局面を散らすためだけに使うので、質より再現性を採る。
/// 同じ--seedで同じデータが出ることが、生成条件を比べる前提になる。
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

type Sink = Arc<Mutex<Vec<String>>>;

/// info行から (深さ, 評価値, 最善手) を取り出す。mateは評価値へ直す。
fn parse_info(line: &str) -> Option<(u32, i32, String)> {
    let t: Vec<&str> = line.split_whitespace().collect();
    let at = |k: &str| t.iter().position(|&x| x == k);
    let depth: u32 = t.get(at("depth")? + 1)?.parse().ok()?;
    let si = at("score")?;
    let score = match *t.get(si + 1)? {
        "cp" => t.get(si + 2)?.parse().ok()?,
        "mate" => {
            let plies: i32 = t.get(si + 2)?.parse().ok()?;
            if plies >= 0 {
                30000 - plies
            } else {
                -30000 - plies
            }
        }
        _ => return None,
    };
    let mv = t.get(at("pv")? + 1)?.to_string();
    Some((depth, score, mv))
}

/// 1局面を探索し、(最善手, 手番視点の評価値) を返す。
fn search(pos: &Position, cfg: &Config, pool: &ThreadPool, sink: &Sink) -> Option<(String, i32)> {
    sink.lock().expect("sink").clear();
    let limits = Limits {
        depth: cfg.depth,
        ..Limits::default()
    };
    let opts = EngineOptions {
        multi_pv: 1,
        threads: cfg.threads,
        hash_mb: cfg.hash_mb,
        ..EngineOptions::default()
    };
    pool.go(pos.clone(), limits, opts);
    pool.wait_idle();

    let mut best: Option<(u32, i32, String)> = None;
    for line in sink.lock().expect("sink").iter() {
        let Some(got) = parse_info(line) else {
            continue;
        };
        if best.as_ref().is_none_or(|b| got.0 >= b.0) {
            best = Some(got);
        }
    }
    best.map(|(_, score, mv)| (mv, score))
}

/// 1局ぶんの記録。勝敗は決着してから埋めるので、手番だけ控えておく。
struct Pending {
    rec: PackedSfenValue,
    stm: Color,
}

/// 1局を指し切り、記録した局面を返す。勝者はNoneが引き分け。
fn play_one(
    cfg: &Config,
    openings: &[String],
    pool: &ThreadPool,
    sink: &Sink,
    quiet: &mut Worker,
    rng: &mut Rng,
) -> (Vec<Pending>, Option<Color>, &'static str) {
    // 開始局面集があればそこから引く。訓練の分布を測定（SPRT・実戦）の
    // 分布へ合わせるための入口で、無ければ従来どおり平手から始める
    let mut pos = if openings.is_empty() {
        Position::from_sfen(SFEN_STARTPOS).expect("startpos")
    } else {
        let pick = rng.below(openings.len());
        Position::from_sfen(&openings[pick]).expect("openings validated at load")
    };
    let mut pending: Vec<Pending> = Vec::new();

    // 開始局面を散らす。ここで指した手には教師にできる評価値が付かない
    for _ in 0..cfg.random_plies {
        let mut list = MoveList::default();
        generate_legal(&pos, true, &mut list);
        if list.is_empty() {
            return (pending, Some(pos.side_to_move().flip()), "mate");
        }
        let pick = rng.below(list.len());
        // MoveListは添字で引けないのでイテレータで取り出す
        let Some(&m) = list.into_iter().nth(pick) else {
            return (pending, None, "nomove");
        };
        pos.do_move(m);
    }

    let mut plies = 0usize;
    loop {
        if plies >= cfg.max_moves {
            return (pending, None, "maxmoves");
        }
        let stm = pos.side_to_move();
        let mut list = MoveList::default();
        generate_legal(&pos, true, &mut list);
        if list.is_empty() {
            return (pending, Some(stm.flip()), "mate");
        }
        match pos.repetition_state_all() {
            Repetition::Draw => return (pending, None, "repetition"),
            // 連続王手の千日手。王手をかけ続けた側が負ける
            Repetition::Win => return (pending, Some(stm), "repetition_win"),
            Repetition::Lose => return (pending, Some(stm.flip()), "repetition_lose"),
            _ => {}
        }
        if pos.can_declare_win() {
            return (pending, Some(stm), "declaration");
        }

        let Some((mv, score)) = search(&pos, cfg, pool, sink) else {
            return (pending, None, "nosearch");
        };
        let Some(m) = pos.move_from_usi(&mv) else {
            return (pending, Some(stm.flip()), "illegal");
        };

        // 記録するのはランダム区間を抜けてからにする。詰みスコアは素通しで
        // よい（ADR-0126）。評価値・勝敗はどちらも手番視点で書く
        if pos.game_ply() >= cfg.min_ply {
            // 記録する前に静止局面へ進める（ADR-0136）。評価関数が探索中に
            // 見るのは静止局面なので、後からpsv quietで直すより、ここで
            // 済ませたほうが工程が1つ減る
            let mut rec_pos = pos.clone();
            let mut rec_score = score;
            let mut rec_stm = stm;
            let mut rec_ply = pos.game_ply();
            let mut rec_move = Move16::from_usi(&mv).unwrap_or(Move16::NONE).0;
            if cfg.quiet_plies > 0 {
                quiet.set_position(pos.clone());
                let plies = quiet.walk_to_quiet(cfg.quiet_plies);
                if plies > 0 {
                    // 奇数手進めると手番が入れ替わる。評価値と勝敗はどちらも
                    // 手番視点なので符号を戻す
                    if plies % 2 == 1 {
                        rec_score = -rec_score;
                        rec_stm = stm.flip();
                    }
                    rec_ply = rec_ply.saturating_add(plies as u16);
                    // PVの初手は元局面のもので、葉では指せない
                    rec_move = Move16::NONE.0;
                    rec_pos = quiet.pos.clone();
                }
            }
            if let Ok(packed) = pack(&rec_pos) {
                pending.push(Pending {
                    rec: PackedSfenValue {
                        sfen: packed,
                        score: rec_score.clamp(-32000, 32000) as i16,
                        move16: rec_move,
                        game_ply: rec_ply,
                        game_result: 0,
                    },
                    stm: rec_stm,
                });
            }
        }

        if cfg.resign > 0 && score <= -cfg.resign {
            return (pending, Some(stm.flip()), "resign");
        }
        pos.do_move(m);
        plies += 1;
    }
}

fn generate(cfg: &Config) -> std::io::Result<()> {
    // 生成条件をログの先頭に残す。教師データは条件を変えて何度も作るので、
    // どの設定で作ったかを後から追えないと混ぜる判断ができない
    eprintln!(
        "GenSfen: games={} depth={} openings={} random_plies={} min_ply={} quiet_plies={} max_moves={} resign={} workers={} hash={}MB seed={}",
        cfg.games,
        cfg.depth,
        cfg.openings.as_deref().unwrap_or("-"),
        cfg.random_plies,
        cfg.min_ply,
        cfg.quiet_plies,
        cfg.max_moves,
        cfg.resign,
        cfg.threads,
        cfg.hash_mb,
        cfg.seed,
    );
    let (net, lineage) = {
        let mut f = std::fs::File::open(&cfg.eval)?;
        himawari_engine::nnue_io::load(&mut f).map_err(|e| std::io::Error::other(e.to_string()))?
    };
    eprintln!("EvalFile: {} ({lineage})", cfg.eval);
    let net = Arc::new(net);

    // 開始局面集は起動時に全行を検証する。生成の途中で不正な行を踏むと、
    // 数時間の走行が無駄になる
    let openings: Arc<Vec<String>> = Arc::new(match &cfg.openings {
        None => Vec::new(),
        Some(path) => {
            let text = std::fs::read_to_string(path)?;
            let mut v = Vec::new();
            for (i, line) in text.lines().enumerate() {
                let s = line.trim().trim_start_matches("sfen ").trim();
                if s.is_empty() {
                    continue;
                }
                if Position::from_sfen(s).is_err() {
                    return Err(std::io::Error::other(format!(
                        "{path}:{} をSFENとして読めません",
                        i + 1
                    )));
                }
                v.push(s.to_string());
            }
            if v.is_empty() {
                return Err(std::io::Error::other(format!(
                    "{path} に開始局面がありません"
                )));
            }
            eprintln!("Openings: {path}（{}局面）", v.len());
            v
        }
    });

    let stop = match &cfg.stop_file {
        Some(p) => StopFile::at(std::path::PathBuf::from(p)),
        None => StopFile::beside(std::path::Path::new(&cfg.out)),
    };
    stop.clear_stale();
    eprintln!("止めるには: touch {}", stop.path().display());

    // 追記で開く。同じ条件で足したいときに、前の生成を消さずに済む
    let w = std::io::BufWriter::new(
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&cfg.out)?,
    );
    let writer = Mutex::new(w);
    let started = std::time::Instant::now();
    let claimed = AtomicUsize::new(0);
    let finished = AtomicUsize::new(0);
    let written = AtomicU64::new(0);
    let stopped = AtomicBool::new(false);

    // 1スレッドで1対局を回し、対局のほうを並列にする。1局面ずつLazy SMPで
    // 探索すると並列効率が上がらず、8スレッドで494局面/秒しか出なかった。
    // 探索を1スレッドに落として対局を並べれば、待ち合わせがなくなる。
    // 置換表はワーカーごとに持つので、指定量を頭数で割る
    let hash_each = (cfg.hash_mb / cfg.threads.max(1)).max(16);
    std::thread::scope(|scope| {
        for t in 0..cfg.threads {
            let net = Arc::clone(&net);
            let openings = Arc::clone(&openings);
            let eval_path = cfg.eval.clone();
            let writer = &writer;
            let claimed = &claimed;
            let finished = &finished;
            let written = &written;
            let stopped = &stopped;
            let stop = &stop;
            scope.spawn(move || {
                let sink: Sink = Arc::new(Mutex::new(Vec::new()));
                let on_line = {
                    let s = Arc::clone(&sink);
                    Arc::new(move |line: &str| {
                        if line.starts_with("info depth") {
                            s.lock().expect("sink").push(line.to_string());
                        }
                    })
                };
                let pool =
                    ThreadPool::new(hash_each, 1, Some((eval_path, Arc::clone(&net))), on_line);
                // 静止化用のWorker。局面ごとに作るとhistory一式の確保が
                // 律速になるので、1つ作って使い回す（ADR-0136で実測）
                let shared = Arc::new(Shared::new(16));
                let start = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
                let limits = Limits::default();
                let tm = TimeManager::new(
                    &limits,
                    start.side_to_move(),
                    start.game_ply(),
                    &TimeOptions::default(),
                );
                let mut quiet = Worker::new(
                    start,
                    shared,
                    limits,
                    tm,
                    0,
                    1,
                    Evaluator::nnue(Arc::clone(&net)),
                    Histories::default(),
                );
                // 種はワーカーごとにずらす。同じ開始局面を8本で作っても
                // 学習の役に立たない
                let mut rng = Rng(
                    (cfg.seed | 1).wrapping_add((t as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
                );
                loop {
                    if stopped.load(Ordering::Relaxed) {
                        break;
                    }
                    // 1局の切れ目で見る。対局の途中では止めない（ADR-0123）
                    if stop.requested() {
                        stopped.store(true, Ordering::Relaxed);
                        break;
                    }
                    if claimed.fetch_add(1, Ordering::Relaxed) >= cfg.games {
                        break;
                    }
                    let (pending, winner, reason) =
                        play_one(cfg, &openings, &pool, &sink, &mut quiet, &mut rng);
                    let mut buf: Vec<u8> = Vec::with_capacity(pending.len() * PSV_BYTES);
                    for p in &pending {
                        let mut rec = p.rec;
                        rec.game_result = match winner {
                            None => 0,
                            Some(c) if c == p.stm => 1,
                            Some(_) => -1,
                        };
                        buf.extend_from_slice(&rec.to_bytes());
                    }
                    {
                        let mut g = writer.lock().expect("writer");
                        if g.write_all(&buf).is_err() {
                            stopped.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                    let n = written.fetch_add(pending.len() as u64, Ordering::Relaxed)
                        + pending.len() as u64;
                    let done = finished.fetch_add(1, Ordering::Relaxed) + 1;
                    if done.is_multiple_of(cfg.save_every) {
                        let mut g = writer.lock().expect("writer");
                        let _ = g.flush();
                        drop(g);
                        let secs = started.elapsed().as_secs_f64().max(0.001);
                        eprintln!(
                            "[{done:>6}局 {:>6.0}s] {n}局面 {:.0}局面/秒 直近={reason}",
                            secs,
                            n as f64 / secs,
                        );
                    }
                }
                pool.quit();
            });
        }
    });

    {
        let mut g = writer.lock().expect("writer");
        g.flush()?;
    }
    let secs = started.elapsed().as_secs_f64().max(0.001);
    let total = written.load(Ordering::Relaxed);
    eprintln!(
        "{} へ{total}局面を書きました（{}局、{:.0}秒、{:.0}局面/秒）",
        cfg.out,
        finished.load(Ordering::Relaxed),
        secs,
        total as f64 / secs,
    );
    if stopped.load(Ordering::Relaxed) {
        // 消しておかないと、次回が何もせずに終わる
        stop.consume();
        eprintln!("停止した。同じコマンドで続きを足せる");
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cfg = Config {
        out: "data/train/gen.psv".to_string(),
        eval: String::new(),
        games: 1000,
        depth: 8,
        openings: None,
        random_plies: 8,
        min_ply: 8,
        quiet_plies: 1,
        max_moves: 320,
        resign: 0,
        threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
        hash_mb: 256,
        seed: 1,
        save_every: 100,
        stop_file: None,
    };
    let mut i = 0;
    while i < args.len() {
        let val = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--out" => cfg.out = val,
            "--eval" => cfg.eval = val,
            "--openings" => cfg.openings = Some(val),
            "--games" => cfg.games = val.parse().unwrap_or(cfg.games),
            "--depth" => cfg.depth = val.parse().unwrap_or(cfg.depth),
            "--random-plies" => cfg.random_plies = val.parse().unwrap_or(cfg.random_plies),
            "--min-ply" => cfg.min_ply = val.parse().unwrap_or(cfg.min_ply),
            "--quiet-plies" => cfg.quiet_plies = val.parse().unwrap_or(cfg.quiet_plies),
            "--max-moves" => cfg.max_moves = val.parse().unwrap_or(cfg.max_moves),
            "--resign" => cfg.resign = val.parse().unwrap_or(cfg.resign),
            "--threads" => cfg.threads = val.parse::<usize>().unwrap_or(cfg.threads).max(1),
            "--hash" => cfg.hash_mb = val.parse().unwrap_or(cfg.hash_mb),
            "--seed" => cfg.seed = val.parse().unwrap_or(cfg.seed),
            "--save-every" => {
                cfg.save_every = val.parse::<usize>().unwrap_or(cfg.save_every).max(1);
            }
            "--stop-file" => cfg.stop_file = Some(val),
            other => {
                eprintln!("不明な引数: {other}");
                std::process::exit(3);
            }
        }
        i += 2;
    }
    if cfg.eval.is_empty() {
        eprintln!("--eval が要る（教師の質は生成側の評価関数で決まる）");
        std::process::exit(3);
    }
    if let Some(dir) = std::path::Path::new(&cfg.out).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = generate(&cfg) {
        eprintln!("エラー: {e}");
        std::process::exit(3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_for_the_same_seed() {
        let mut a = Rng(1);
        let mut b = Rng(1);
        for _ in 0..8 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn rng_below_stays_in_range() {
        let mut r = Rng(42);
        for _ in 0..100 {
            assert!(r.below(30) < 30);
        }
    }

    #[test]
    fn parse_info_reads_mate_as_score() {
        let (depth, score, mv) =
            parse_info("info depth 9 score mate 3 pv 7g7f 3c3d").expect("parse");
        assert_eq!(depth, 9);
        assert_eq!(score, 29997);
        assert_eq!(mv, "7g7f");
    }

    #[test]
    fn parse_info_reads_cp() {
        let (_, score, mv) = parse_info("info depth 8 score cp -42 pv 2g2f").expect("parse");
        assert_eq!(score, -42);
        assert_eq!(mv, "2g2f");
    }

    #[test]
    fn psv_record_size_is_unchanged() {
        assert_eq!(PSV_BYTES, 40);
    }
}
