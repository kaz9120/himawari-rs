//! 定跡生成ツール（ADR-0063）。
//!
//! 平手初期局面から展開し、各局面をMultiPV=widthで探索して上位width手を
//! 記録する。出力はやねうら王db形式互換。
//!
//! 展開の順は幅優先ではない。「初期局面からここまでに何cp損したか」の
//! 小さい順に掘る（`Task` を参照）。実戦で現れやすい変化から深くなる。
//!
//! 使い方:
//!   book gen --out <path> [--eval <hmwr>] [--ply 24] [--width 4]
//!            [--full-ply 0] [--depth 24] [--hash 256] [--threads N]
//!            [--max-positions 2000] [--margin 100] [--save-every 25]
//!            [--stop-file <path>]
//!
//! --ply は展開する手数、--width は各局面で探索する候補手数。
//! 先手番・後手番の両方を含める。相手の手を経由しないと自分の手番の
//! 局面に到達できないため。
//!
//! 幅をwidthだけで決めると局面数がwidth^plyで爆発する。--margin で
//! 最善手との評価差を切り、差が開いた候補は記録も展開もしない。実戦で
//! 選ばれない手に探索時間を使わないためである。--max-positions に
//! 達したら打ち切る。
//!
//! --full-ply より浅い層では、この絞り込みを外して全合法手を展開する
//! （ADR-0146）。相手の手は選べないので、widthとmarginで絞ると定跡を
//! 引けるかどうかが相手の指し手に依存する。平手初期局面の合法手は30通り
//! あり、--full-ply 2 なら61局面で「相手の初手が何であっても自分の2手目と
//! 3手目を定跡から出せる」状態になる。この区間はcostを0で積むため、
//! 埋め終わるまで深い層へ進まない。
//!
//! 長時間走るので --save-every 局面ごとに書き出す。出力ファイルが既に
//! あれば読み込んで再開する。中断しても、そこまでの定跡はそのまま使える。
//!
//! 途中で止めるときは停止ファイルを置く（ADR-0123）。次の局面へ進む前に
//! 書き出して終わるので、探索中の1局面ぶんも失わない。
//!
//!   touch data/book/main.db.stop
//!
//! 探索はThreadPool経由でLazy SMP（ADR-0031）を使う。置換表は局面を
//! またいで再利用する。親から子へ展開するので、親の探索で読んだ子局面の
//! 情報がそのまま効く。TTのエントリは深さ付きなので、浅い探索の結果が
//! 深い探索に流用されることはない。

use std::collections::{BinaryHeap, HashMap, HashSet};
use std::io::Write;
use std::sync::{Arc, Mutex};

use himawari_core::{Color, MoveList, Position, SFEN_STARTPOS, generate_legal};
use himawari_engine::{EngineOptions, Limits, ThreadPool};
use himawari_tools::stop_file::StopFile;

struct Config {
    out: String,
    eval: String,
    ply: u16,
    width: usize,
    full_ply: u16,
    depth: u32,
    hash_mb: usize,
    threads: usize,
    max_positions: usize,
    margin: i32,
    save_every: usize,
    /// 停止ファイルのパス。省略すると `<out>.stop`（ADR-0123）
    stop_file: Option<String>,
}

/// 1局面ぶんの候補手。(指し手, 評価値, 予想応手) を評価値の降順で持つ。
type Lines = Vec<(String, i32, String)>;

/// 展開待ちの局面。`cost` は初期局面からここへ至るまでに、各手が
/// その局面の最善手から何cp劣っていたかの累計である。
///
/// 幅優先だと浅い層の全変化を埋めてから次の層へ進むため、上限で打ち切ると
/// 定跡が浅いまま終わる。costの小さい順に展開すると、全部最善手で進む本筋を
/// 先に深く掘り、次善手を1つ挟んだ変化がその次に来る。実戦で現れる順に
/// 近くなる。
struct Task {
    cost: i32,
    seq: usize,
    ply: u16,
    pos: Position,
    /// この枝でエンジンが持つ側。同じ局面でも、自分が先手か後手かで
    /// 「相手の手」が入れ替わるため、展開の幅が変わる（ADR-0146）。
    my_side: Color,
}

impl Ord for Task {
    /// BinaryHeapは最大値から取り出すので、costが小さいほど大きいとする。
    /// 同点は先に積んだものを優先し、順序を決定的にする。
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other
            .cost
            .cmp(&self.cost)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

impl PartialOrd for Task {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Task {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost && self.seq == other.seq
    }
}

impl Eq for Task {}

/// 生成中の定跡。キーは手数を除いたsfenで、`order` が挿入順を保つ。
struct Book {
    lines: HashMap<String, Lines>,
    order: Vec<String>,
}

impl Book {
    fn new() -> Self {
        Self {
            lines: HashMap::new(),
            order: Vec::new(),
        }
    }

    fn insert(&mut self, key: String, lines: Lines) {
        if self.lines.insert(key.clone(), lines).is_none() {
            self.order.push(key);
        }
    }

    fn len(&self) -> usize {
        self.order.len()
    }

    /// db2016形式で書き出す。一時ファイルへ書いてから置き換えるので、
    /// 途中で落ちても出力が壊れない。
    fn save(&self, path: &str, depth: u32) -> std::io::Result<()> {
        let tmp = format!("{path}.tmp");
        {
            let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
            writeln!(f, "#YANEURAOU-DB2016 1.00")?;
            for key in &self.order {
                writeln!(f, "sfen {key} 1")?;
                for (mv, score, ponder) in &self.lines[key] {
                    writeln!(f, "{mv} {ponder} {score} {depth} 1")?;
                }
            }
            f.flush()?;
        }
        std::fs::rename(&tmp, path)
    }

    /// 書き出し済みの定跡を読む。再開のために使う。
    /// ファイルがなければ空を返す。
    fn load(path: &str) -> std::io::Result<Self> {
        let mut book = Self::new();
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(book),
            Err(e) => return Err(e),
        };
        let mut key: Option<String> = None;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("sfen ") {
                // sfen行の末尾は手数。キーは手数を除いた部分に揃える
                let k = match rest.rfind(' ') {
                    Some(i) => rest[..i].to_string(),
                    None => rest.to_string(),
                };
                book.insert(k.clone(), Vec::new());
                key = Some(k);
                continue;
            }
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let Some(k) = key.as_ref() else { continue };
            let t: Vec<&str> = line.split_whitespace().collect();
            if t.len() < 3 {
                continue;
            }
            let Ok(score) = t[2].parse::<i32>() else {
                continue;
            };
            book.lines.get_mut(k).expect("直前のsfen行で作った").push((
                t[0].to_string(),
                score,
                t[1].to_string(),
            ));
        }
        Ok(book)
    }
}

/// 手数を除いたsfen（盤面・手番・手駒）をキーにする。
fn book_key(pos: &Position) -> String {
    let sfen = pos.to_sfen();
    match sfen.rfind(' ') {
        Some(i) => sfen[..i].to_string(),
        None => sfen,
    }
}

/// info行から (depth, multipv, 評価値, pvの指し手列) を取り出す。
/// mateスコアは詰み手数から評価値に直す（定跡には数値で入れる）。
fn parse_info(line: &str) -> Option<(u32, usize, i32, Vec<String>)> {
    let t: Vec<&str> = line.split_whitespace().collect();
    let at = |k: &str| t.iter().position(|&x| x == k);
    let depth: u32 = t.get(at("depth")? + 1)?.parse().ok()?;
    let multipv = at("multipv")
        .and_then(|i| t.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(1usize);
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
    let pv = t[at("pv")? + 1..].iter().map(|s| s.to_string()).collect();
    Some((depth, multipv, score, pv))
}

/// 1局面をMultiPV=widthで探索し、上位width手を (指し手, 評価値, 予想応手) で返す。
fn search_lines(pos: &Position, cfg: &Config, pool: &ThreadPool, sink: &Sink) -> Lines {
    sink.lock().expect("sink").clear();
    let limits = Limits {
        depth: cfg.depth,
        ..Limits::default()
    };
    let opts = EngineOptions {
        multi_pv: cfg.width,
        threads: cfg.threads,
        hash_mb: cfg.hash_mb,
        ..EngineOptions::default()
    };
    pool.go(pos.clone(), limits, opts);
    pool.wait_idle();

    // 各ラインについて最終深さの結果を採る
    // (depth, score, pv)
    type BestEntry = (u32, i32, Vec<String>);
    let mut best: HashMap<usize, BestEntry> = HashMap::new();
    for line in sink.lock().expect("sink").iter() {
        let Some((depth, multipv, score, pv)) = parse_info(line) else {
            continue;
        };
        if pv.is_empty() {
            continue;
        }
        let e = best.entry(multipv).or_insert((0, 0, Vec::new()));
        if depth >= e.0 {
            *e = (depth, score, pv);
        }
    }
    let mut lines: Vec<(usize, BestEntry)> = best.into_iter().collect();
    lines.sort_by_key(|(k, _)| *k);
    lines
        .into_iter()
        .filter_map(|(_, (_, score, pv))| {
            let mv = pv.first()?.clone();
            let ponder = pv.get(1).cloned().unwrap_or_else(|| "none".to_string());
            Some((mv, score, ponder))
        })
        .collect()
}

type Sink = Arc<Mutex<Vec<String>>>;

fn generate(cfg: &Config) -> std::io::Result<()> {
    // 生成条件をログの先頭に残す（ADR-0082）。定跡は非決定的に生成され、
    // 評価関数にも依存するため、どの設定で作ったかを後から追えないと
    // 作り直しの判断ができない
    eprintln!(
        "BookGen: ply={} width={} full_ply={} depth={} threads={} hash={}MB max={} margin={}",
        cfg.ply,
        cfg.width,
        cfg.full_ply,
        cfg.depth,
        cfg.threads,
        cfg.hash_mb,
        cfg.max_positions,
        cfg.margin
    );
    let eval = if cfg.eval.is_empty() {
        None
    } else {
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

    let mut book = Book::load(&cfg.out)?;
    let resumed = book.len();
    if resumed > 0 {
        eprintln!("再開: {} から {resumed}局面を読み込みました", cfg.out);
    }

    let stop = match &cfg.stop_file {
        Some(p) => StopFile::at(std::path::PathBuf::from(p)),
        None => StopFile::beside(std::path::Path::new(&cfg.out)),
    };
    stop.clear_stale();
    eprintln!("止めるには: touch {}", stop.path().display());

    let root = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
    let mut queue: BinaryHeap<Task> = BinaryHeap::new();
    let mut seq = 0usize;
    // 先手用と後手用の2本を同じ木に重ねて掘る。エンジンはどちらにもなるので、
    // 片側だけ掘ると反対の手番で穴が残る（ADR-0146）
    for my_side in [Color::Black, Color::White] {
        queue.push(Task {
            cost: 0,
            seq,
            ply: 0,
            pos: root.clone(),
            my_side,
        });
        seq += 1;
    }
    // 局面が同じでも持つ側が違えば展開の幅が変わるので、側までをキーにする。
    // 探索の結果はbookに入っているので、2度目は使い回して探索し直さない
    let mut visited: HashSet<(String, u8)> = HashSet::new();
    let started = std::time::Instant::now();
    let mut searched = 0usize;
    let mut stopped = false;

    while let Some(task) = queue.pop() {
        if task.ply >= cfg.ply || book.len() >= cfg.max_positions {
            continue;
        }
        // 1局面に入る前に見る。探索の途中では止めない（ADR-0123）
        if stop.requested() {
            eprintln!("停止ファイルを見つけた: {}", stop.path().display());
            stopped = true;
            break;
        }
        let key = book_key(&task.pos);
        if !visited.insert((key.clone(), task.my_side as u8)) {
            continue;
        }
        // 再開時、探索済みの局面は結果を使って子だけを積む
        let lines = match book.lines.get(&key) {
            Some(known) => known.clone(),
            None => {
                let found = prune_by_margin(search_lines(&task.pos, cfg, &pool, &sink), cfg.margin);
                if found.is_empty() {
                    continue;
                }
                searched += 1;
                let secs = started.elapsed().as_secs();
                eprintln!(
                    "[{:>5}局面 {:>6}s] ply={:>2} 差{:>4} 待ち{:>5} 手={} 評価={} 候補{}",
                    book.len() + 1,
                    secs,
                    task.ply,
                    task.cost,
                    queue.len(),
                    found[0].0,
                    found[0].1,
                    found.len(),
                );
                book.insert(key.clone(), found.clone());
                if searched.is_multiple_of(cfg.save_every) {
                    book.save(&cfg.out, cfg.depth)?;
                    eprintln!("  途中書き出し: {}局面（{secs}秒）", book.len());
                }
                found
            }
        };
        // 相手の手番の浅い層は、widthとmarginで絞らず全合法手を展開する
        // （ADR-0146）。相手の指し手は選べないので、絞ると定跡を引けるか
        // どうかが相手に依存する。cost 0 で積み、この層を埋め終わるまで
        // 深い層へ進まない
        if task.pos.side_to_move() != task.my_side && task.ply < cfg.full_ply {
            let mut legal = MoveList::default();
            generate_legal(&task.pos, false, &mut legal);
            for &m in &legal {
                let mut next = task.pos.clone();
                next.do_move(m);
                seq += 1;
                queue.push(Task {
                    cost: 0,
                    seq,
                    ply: task.ply + 1,
                    pos: next,
                    my_side: task.my_side,
                });
            }
            continue;
        }
        let best = lines[0].1;
        for (mv, score, _) in &lines {
            let Some(m) = task.pos.move_from_usi(mv) else {
                continue;
            };
            let mut next = task.pos.clone();
            next.do_move(m);
            seq += 1;
            queue.push(Task {
                cost: task.cost + (best - score),
                seq,
                ply: task.ply + 1,
                pos: next,
                my_side: task.my_side,
            });
        }
    }
    pool.quit();

    book.save(&cfg.out, cfg.depth)?;
    eprintln!(
        "{} に {}局面を書き出しました（今回{searched}局面を探索、{}秒）",
        cfg.out,
        book.len(),
        started.elapsed().as_secs()
    );
    if stopped {
        // 消しておかないと、再開したつもりの次回が何もせずに終わる
        stop.consume();
        eprintln!("停止した。同じコマンドで再開できる");
    }
    Ok(())
}

/// 最善手からmargin（cp）以上離れた候補を落とす。最善手は必ず残す。
/// 実戦で選ばれない手を展開しても、その先の枝が丸ごと無駄になる。
///
/// MultiPVは評価順に並ぶが、ラインごとに最終深さが違うと順序が乱れる。
/// 切る前に評価値で並べ直す。
fn prune_by_margin(mut lines: Lines, margin: i32) -> Lines {
    lines.sort_by_key(|(_, score, _)| -score);
    let Some(best) = lines.first().map(|l| l.1) else {
        return lines;
    };
    lines.retain(|(_, score, _)| best - score <= margin);
    lines
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("gen") {
        eprintln!("使い方は crates/tools/src/bin/book.rs 冒頭のコメントを参照");
        std::process::exit(3);
    }
    let mut cfg = Config {
        out: "data/book/mini.db".to_string(),
        eval: String::new(),
        ply: 24,
        width: 4,
        full_ply: 0,
        depth: 24,
        hash_mb: 256,
        threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
        max_positions: 2000,
        margin: 100,
        save_every: 25,
        stop_file: None,
    };
    let mut i = 1;
    while i < args.len() {
        let val = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--out" => cfg.out = val,
            "--eval" => cfg.eval = val,
            "--ply" => cfg.ply = val.parse().unwrap_or(cfg.ply),
            "--width" => cfg.width = val.parse::<usize>().unwrap_or(cfg.width).max(1),
            "--full-ply" => cfg.full_ply = val.parse().unwrap_or(cfg.full_ply),
            "--depth" => cfg.depth = val.parse().unwrap_or(cfg.depth),
            "--hash" => cfg.hash_mb = val.parse().unwrap_or(cfg.hash_mb),
            "--threads" => cfg.threads = val.parse::<usize>().unwrap_or(cfg.threads).max(1),
            "--max-positions" => {
                cfg.max_positions = val.parse::<usize>().unwrap_or(cfg.max_positions).max(1);
            }
            "--margin" => cfg.margin = val.parse().unwrap_or(cfg.margin),
            "--stop-file" => cfg.stop_file = Some(val),
            "--save-every" => {
                cfg.save_every = val.parse::<usize>().unwrap_or(cfg.save_every).max(1);
            }
            other => {
                eprintln!("不明な引数: {other}");
                std::process::exit(3);
            }
        }
        i += 2;
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

    fn line(mv: &str, score: i32) -> (String, i32, String) {
        (mv.to_string(), score, "none".to_string())
    }

    #[test]
    fn margin_keeps_best_and_drops_far_candidates() {
        let lines = vec![
            line("7g7f", 30),
            line("2g2f", -80),
            line("6i7h", 20),
            line("9g9f", -71),
        ];
        let kept = prune_by_margin(lines, 100);
        // 最善は30。-71は差101で落ち、-80は差110で落ちる
        let moves: Vec<&str> = kept.iter().map(|(m, _, _)| m.as_str()).collect();
        assert_eq!(moves, ["7g7f", "6i7h"]);
    }

    #[test]
    fn margin_sorts_before_cutting() {
        // MultiPVの順序が評価値の降順でなくても最善を取り違えない
        let lines = vec![line("a", -50), line("b", 40), line("c", 0)];
        let kept = prune_by_margin(lines, 45);
        let moves: Vec<&str> = kept.iter().map(|(m, _, _)| m.as_str()).collect();
        assert_eq!(moves, ["b", "c"]);
    }

    #[test]
    fn margin_keeps_sole_candidate_however_bad() {
        let kept = prune_by_margin(vec![line("7g7f", -900)], 10);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn cheaper_task_pops_first() {
        let pos = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
        let mut heap = BinaryHeap::new();
        for (cost, seq) in [(120, 1), (0, 2), (40, 3), (0, 4)] {
            heap.push(Task {
                cost,
                seq,
                ply: 1,
                pos: pos.clone(),
                my_side: Color::Black,
            });
        }
        // costの小さい順、同点は先に積んだ順
        let got: Vec<(i32, usize)> =
            std::iter::from_fn(|| heap.pop().map(|t| (t.cost, t.seq))).collect::<Vec<_>>();
        assert_eq!(got, [(0, 2), (0, 4), (40, 3), (120, 1)]);
    }

    #[test]
    fn save_then_load_round_trips() {
        let path =
            std::env::temp_dir().join(format!("himawari-book-test-{}.db", std::process::id()));
        let path = path.to_str().expect("utf8");

        let mut book = Book::new();
        book.insert(
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b -".to_string(),
            vec![
                line("7g7f", 30),
                ("2g2f".to_string(), 12, "8c8d".to_string()),
            ],
        );
        book.insert(
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/7P1/PPPPPPP1P/1B5R1/LNSGKGSNL w -".to_string(),
            vec![line("3c3d", -8)],
        );
        book.save(path, 32).expect("save");

        let loaded = Book::load(path).expect("load");
        assert_eq!(loaded.order, book.order);
        assert_eq!(loaded.lines, book.lines);

        std::fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn load_returns_empty_when_file_is_absent() {
        let book = Book::load("/nonexistent/himawari/book.db").expect("欠損は空として扱う");
        assert_eq!(book.len(), 0);
    }
}
