//! 自己対局で教師データを作る（ADR-0144）。
//!
//! 平手初期局面からランダムに数手指して開始局面を散らし、そこから終局まで
//! 一定深さで指し継ぐ。各局面の (局面, 評価値, 指し手, 手数) を控え、決着が
//! ついた時点で勝敗を書き込んでPSV（ADR-0038）へ流す。
//!
//! 使い方:
//!   gensfen --out <path> --eval <hmwr> [--games 1000] [--depth 8]
//!           [--random-plies 8] [--min-ply 8] [--max-moves 320]
//!           [--resign 3000] [--threads N] [--hash 256] [--seed 1]
//!           [--save-every 100] [--stop-file <path>]
//!
//! --random-plies は開始局面を散らすためにランダムへ指す手数。同じ局面ばかり
//! 生成しても学習の役に立たない。--min-ply より前の局面は記録しない。乱数で
//! 指した区間には教師にできる評価値が付かないためである。
//!
//! --resign を超える評価値が出たら投了する。決着の見えた局面を指し継いでも
//! 教師の情報量は増えず、生成速度だけが落ちる。0を渡すと投了しない。
//!
//! 評価値と勝敗はどちらも手番視点で書く（ADR-0136で踏んだ落とし穴）。
//! 詰みスコアは素通しにする（ADR-0126）。
//!
//! 途中で止めるときは停止ファイルを置く（ADR-0123）。1局の切れ目で書き出して
//! 終わるので、途中まで生成した教師はそのまま使える。
//!
//!   touch data/train/gen.psv.stop

use std::io::Write;
use std::sync::{Arc, Mutex};

use himawari_core::packed_sfen::{PackedSfenValue, pack};
use himawari_core::{Color, Move16, MoveList, Position, Repetition, SFEN_STARTPOS, generate_legal};
use himawari_engine::{EngineOptions, Limits, ThreadPool};
use himawari_tools::stop_file::StopFile;

struct Config {
    out: String,
    eval: String,
    games: usize,
    depth: u32,
    random_plies: usize,
    min_ply: u16,
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
    pool: &ThreadPool,
    sink: &Sink,
    rng: &mut Rng,
) -> (Vec<Pending>, Option<Color>, &'static str) {
    let mut pos = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
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
        match pos.repetition_state() {
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
        if pos.game_ply() >= cfg.min_ply
            && let Ok(packed) = pack(&pos)
        {
            pending.push(Pending {
                rec: PackedSfenValue {
                    sfen: packed,
                    score: score.clamp(-32000, 32000) as i16,
                    move16: Move16::from_usi(&mv).unwrap_or(Move16::NONE).0,
                    game_ply: pos.game_ply(),
                    game_result: 0,
                },
                stm,
            });
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
        "GenSfen: games={} depth={} random_plies={} min_ply={} max_moves={} resign={} threads={} hash={}MB seed={}",
        cfg.games,
        cfg.depth,
        cfg.random_plies,
        cfg.min_ply,
        cfg.max_moves,
        cfg.resign,
        cfg.threads,
        cfg.hash_mb,
        cfg.seed,
    );
    let eval = {
        let mut f = std::fs::File::open(&cfg.eval)?;
        let (net, lineage) = himawari_engine::nnue_io::load(&mut f)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        eprintln!("EvalFile: {} ({lineage})", cfg.eval);
        Some((cfg.eval.clone(), Arc::new(net)))
    };
    let sink: Sink = Arc::new(Mutex::new(Vec::new()));
    let on_line = {
        let s = Arc::clone(&sink);
        Arc::new(move |line: &str| {
            if line.starts_with("info depth") {
                s.lock().expect("sink").push(line.to_string());
            }
        })
    };
    let pool = ThreadPool::new(cfg.hash_mb, cfg.threads, eval, on_line);

    let stop = match &cfg.stop_file {
        Some(p) => StopFile::at(std::path::PathBuf::from(p)),
        None => StopFile::beside(std::path::Path::new(&cfg.out)),
    };
    stop.clear_stale();
    eprintln!("止めるには: touch {}", stop.path().display());

    // 追記で開く。同じ条件で足したいときに、前の生成を消さずに済む
    let mut w = std::io::BufWriter::new(
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&cfg.out)?,
    );
    let mut rng = Rng(cfg.seed | 1);
    let started = std::time::Instant::now();
    let mut written = 0u64;
    let mut games = 0usize;
    let mut stopped = false;

    while games < cfg.games {
        // 1局の切れ目で見る。対局の途中では止めない（ADR-0123）
        if stop.requested() {
            eprintln!("停止ファイルを見つけた: {}", stop.path().display());
            stopped = true;
            break;
        }
        let (pending, winner, reason) = play_one(cfg, &pool, &sink, &mut rng);
        games += 1;
        for p in &pending {
            let mut rec = p.rec;
            rec.game_result = match winner {
                None => 0,
                Some(c) if c == p.stm => 1,
                Some(_) => -1,
            };
            w.write_all(&rec.to_bytes())?;
            written += 1;
        }
        if games.is_multiple_of(cfg.save_every) {
            w.flush()?;
            let secs = started.elapsed().as_secs_f64().max(0.001);
            eprintln!(
                "[{games:>6}局 {:>6.0}s] {written}局面 {:.0}局面/秒 直近={reason}",
                secs,
                written as f64 / secs,
            );
        }
    }
    w.flush()?;
    pool.quit();

    let secs = started.elapsed().as_secs_f64().max(0.001);
    eprintln!(
        "{} へ{written}局面を書きました（{games}局、{:.0}秒、{:.0}局面/秒）",
        cfg.out,
        secs,
        written as f64 / secs,
    );
    if stopped {
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
        random_plies: 8,
        min_ply: 8,
        max_moves: 320,
        resign: 3000,
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
            "--games" => cfg.games = val.parse().unwrap_or(cfg.games),
            "--depth" => cfg.depth = val.parse().unwrap_or(cfg.depth),
            "--random-plies" => cfg.random_plies = val.parse().unwrap_or(cfg.random_plies),
            "--min-ply" => cfg.min_ply = val.parse().unwrap_or(cfg.min_ply),
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
        assert_eq!(himawari_core::packed_sfen::PSV_BYTES, 40);
    }
}
