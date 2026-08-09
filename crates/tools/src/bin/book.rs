//! 定跡生成ツール（ADR-0063）。
//!
//! 平手初期局面から展開し、各局面をMultiPV=widthで探索して上位width手を
//! 記録する。出力はやねうら王db形式互換。
//!
//! 展開の順は幅優先ではない。「初期局面からここまでに何cp損したか」の
//! 小さい順に掘る（`Task` を参照）。実戦で現れやすい変化から深くなる。
//!
//! 使い方:
//!   book gen     定跡を掘り広げる（新しい局面を足す）
//!   book seed    棋譜に現れた局面を種にして定跡へ足す
//!   book refresh 定跡が持つ局面はそのままに、指し手と評価値を引き直す
//!   book stats   plyごとの網羅率を数える
//!
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
//! refreshは評価関数の世代が変わったときに使う（ADR-0146）。掘り直すのは
//! 高くつくので、局面の集合を変えずに中身だけ新しいネットで引き直す。
//!
//!   book refresh --out data/book/main.db --eval <新しいhmwr> [--depth 28]
//!
//! 定跡を広げるのは gen を条件を変えて再実行する。出力ファイルがあれば
//! 読んで再開するので、--max-positions を増やせば続きから掘り、
//! --full-ply を上げれば浅い層の幅が広がる。
//!
//! seedは実戦に現れた局面を定跡へ足す（ADR-0152）。floodgateの棋譜を定期的に
//! 回収して回すので、同じ入力からは同じ定跡が出る手順にしてある。
//!
//!   book seed --games data/raw/floodgate --out data/book/main.db
//!             --eval <hmwr> [--max-ply 24] [--width 4] [--depth 28]
//!             [--hash 256] [--margin 100] [--max-positions N]
//!             [--save-every 25] [--stop-file <path>]
//!
//! 決定論のために3つを固定する。CSAはファイルのパス昇順に読み、種は
//! (ply昇順, sfen辞書順) に並べ、探索は1スレッドで行う。**seedは
//! --threads を受け付けない。** マルチスレッド探索は同じ局面でも選ぶ手が
//! 揺れるためである。
//!
//! 定跡が既に持つ局面は探索せずに飛ばす。追加が0なら書き出しもしないので、
//! 2度目の実行はファイルを1byteも変えない。--max-positions は今回追加する
//! 局面数の上限で、gen（定跡全体の上限）とは意味が違う。
//!
//! statsは「相手の手が何であっても定跡を引けるか」を見る。plyを進めるたびに、
//! その層の全合法手のうち何手ぶんを定跡が持っているかを出す。--ply で
//! 数える深さを決める。
//!
//!   book stats --out data/book/main.db --ply 6
//!
//! 探索はThreadPool経由でLazy SMP（ADR-0031）を使う。置換表は局面を
//! またいで再利用する。親から子へ展開するので、親の探索で読んだ子局面の
//! 情報がそのまま効く。TTのエントリは深さ付きなので、浅い探索の結果が
//! 深い探索に流用されることはない。

use std::collections::{BinaryHeap, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
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
    /// seedの入力。CSAのファイルかディレクトリを並べる
    games: Vec<String>,
    /// seedが種として拾う手数の上限
    max_ply: u16,
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

/// 評価関数を読み、探索用のスレッドプールとinfo行の受け皿を作る。
/// genとrefreshで同じ手順を使う。
fn make_pool(cfg: &Config) -> std::io::Result<(ThreadPool, Sink)> {
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
    Ok((
        ThreadPool::new(cfg.hash_mb, cfg.threads, eval, on_line),
        sink,
    ))
}

/// 定跡が持つ局面はそのままに、指し手と評価値だけを引き直す（ADR-0146）。
///
/// 評価関数の世代が変わると、古い定跡は前の世代の判断を持ち続ける。掘り直す
/// のは高くつくので、局面の集合を変えずに中身だけ更新する経路を分けている。
///
/// 途中で止めても定跡は壊れない。読んだ定跡をそのまま土台にして上書きするので、
/// 更新できなかった局面は前の値のまま残る。
fn refresh(cfg: &Config) -> std::io::Result<()> {
    eprintln!(
        "BookRefresh: width={} depth={} threads={} hash={}MB margin={}",
        cfg.width, cfg.depth, cfg.threads, cfg.hash_mb, cfg.margin
    );
    let (pool, sink) = make_pool(cfg)?;
    let mut book = Book::load(&cfg.out)?;
    let total = book.len();
    if total == 0 {
        pool.quit();
        eprintln!("{} に定跡がない", cfg.out);
        return Ok(());
    }
    eprintln!("{}局面を引き直す", total);

    let stop = match &cfg.stop_file {
        Some(p) => StopFile::at(std::path::PathBuf::from(p)),
        None => StopFile::beside(std::path::Path::new(&cfg.out)),
    };
    stop.clear_stale();
    eprintln!("止めるには: touch {}", stop.path().display());

    let keys = book.order.clone();
    let started = std::time::Instant::now();
    let mut done = 0usize;
    let mut stopped = false;
    for key in &keys {
        // 1局面に入る前に見る。探索の途中では止めない（ADR-0123）
        if stop.requested() {
            eprintln!("停止ファイルを見つけた: {}", stop.path().display());
            stopped = true;
            break;
        }
        // キーは手数を落としたsfenなので、読むときに補う
        let Ok(pos) = Position::from_sfen(&format!("{key} 1")) else {
            eprintln!("読めない局面を飛ばす: {key}");
            continue;
        };
        let found = prune_by_margin(search_lines(&pos, cfg, &pool, &sink), cfg.margin);
        if found.is_empty() {
            continue;
        }
        book.insert(key.clone(), found.clone());
        done += 1;
        let secs = started.elapsed().as_secs();
        eprintln!(
            "[{done:>5}/{total} {secs:>6}s] 手={} 評価={} 候補{}",
            found[0].0,
            found[0].1,
            found.len(),
        );
        if done.is_multiple_of(cfg.save_every) {
            book.save(&cfg.out, cfg.depth)?;
            eprintln!("  途中書き出し: {done}局面を更新（{secs}秒）");
        }
    }
    pool.quit();
    book.save(&cfg.out, cfg.depth)?;
    eprintln!(
        "{} の{done}局面を引き直しました（全{total}局面、{}秒）",
        cfg.out,
        started.elapsed().as_secs()
    );
    if stopped {
        // 消しておかないと、次回が何もせずに終わる
        stop.consume();
        eprintln!("停止した。同じコマンドで最初から引き直せる");
    }
    Ok(())
}

/// CSA棋譜から序盤の局面を取り出す（ADR-0152）。
///
/// 必要なのは局面だけなので、読むのは開始局面の指定と指し手に絞る。
/// 指し手は合法手との突き合わせで解決する。CSAは移動元・移動先・移動後の
/// 駒種を全部書くので、合法手の中に同じ形は1つしかない。棋譜の表記から
/// 直接Moveを組み立てるより、駒種のエンコーディングに触れずに済む。
mod csa {
    use himawari_core::{Color, Move, MoveList, PieceType, Position, Square, generate_legal};
    use himawari_core::{File, Rank};

    /// 平手の初期配置。`PI` を展開するときに使う。P1〜P9と同じ書式で持ち、
    /// 盤面の読み取りを1つの関数に集約する。
    const HIRATE: [&str; 9] = [
        "-KY-KE-GI-KI-OU-KI-GI-KE-KY",
        " * -HI *  *  *  *  * -KA * ",
        "-FU-FU-FU-FU-FU-FU-FU-FU-FU",
        " *  *  *  *  *  *  *  *  * ",
        " *  *  *  *  *  *  *  *  * ",
        " *  *  *  *  *  *  *  *  * ",
        "+FU+FU+FU+FU+FU+FU+FU+FU+FU",
        " * +KA *  *  *  *  * +HI * ",
        "+KY+KE+GI+KI+OU+KI+GI+KE+KY",
    ];

    /// 手駒をsfenへ書き出す順。やねうら王・USIの慣例に合わせる。
    const HAND_ORDER: [PieceType; 7] = [
        PieceType::ROOK,
        PieceType::BISHOP,
        PieceType::GOLD,
        PieceType::SILVER,
        PieceType::KNIGHT,
        PieceType::LANCE,
        PieceType::PAWN,
    ];

    /// CSAの駒名（2文字）を駒種へ。
    fn piece_type(name: &str) -> Option<PieceType> {
        Some(match name {
            "FU" => PieceType::PAWN,
            "KY" => PieceType::LANCE,
            "KE" => PieceType::KNIGHT,
            "GI" => PieceType::SILVER,
            "KI" => PieceType::GOLD,
            "KA" => PieceType::BISHOP,
            "HI" => PieceType::ROOK,
            "OU" => PieceType::KING,
            "TO" => PieceType::PRO_PAWN,
            "NY" => PieceType::PRO_LANCE,
            "NK" => PieceType::PRO_KNIGHT,
            "NG" => PieceType::PRO_SILVER,
            "UM" => PieceType::HORSE,
            "RY" => PieceType::DRAGON,
            _ => return None,
        })
    }

    /// CSAのマス表記（"76" = 7筋6段）を盤の位置へ。
    fn square(s: &str) -> Option<Square> {
        let b = s.as_bytes();
        if b.len() != 2 {
            return None;
        }
        let file = b[0].checked_sub(b'1')?;
        let rank = b[1].checked_sub(b'1')?;
        if file < 9 && rank < 9 {
            Some(Square::new(File(file), Rank(rank)))
        } else {
            None
        }
    }

    /// マス表記を盤配列の添字（段, 列）へ。列は9筋から1筋の順で、
    /// CSAのP行およびsfenの並びと同じにする。
    fn cell(s: &str) -> Option<(usize, usize)> {
        let b = s.as_bytes();
        if b.len() != 2 {
            return None;
        }
        let file = b[0].checked_sub(b'1')? as usize;
        let rank = b[1].checked_sub(b'1')? as usize;
        if file < 9 && rank < 9 {
            Some((rank, 8 - file))
        } else {
            None
        }
    }

    /// 開始局面を組み立てる途中の状態。
    struct Builder {
        /// [段][列]。列は9筋から1筋の順。
        board: [[Option<(Color, PieceType)>; 9]; 9],
        hands: [[u8; PieceType::NB]; 2],
        /// 盤面の指定（PIかP1〜P9）を受け取ったか
        placed: bool,
    }

    impl Builder {
        fn new() -> Self {
            Self {
                board: [[None; 9]; 9],
                hands: [[0; PieceType::NB]; 2],
                placed: false,
            }
        }

        /// P1〜P9の1行を読む。末尾の空白が落ちた棋譜があるので、
        /// 27字に満たない分は空マスとして補う。
        fn set_row(&mut self, rank: usize, payload: &str) -> Result<(), String> {
            if !payload.is_ascii() {
                return Err(format!("P{}行にascii以外がある", rank + 1));
            }
            if payload.len() > 27 {
                return Err(format!("P{}行が長い", rank + 1));
            }
            let padded = format!("{payload:<27}");
            for col in 0..9 {
                let chunk = &padded[col * 3..col * 3 + 3];
                if chunk.trim() == "*" || chunk.trim().is_empty() {
                    self.board[rank][col] = None;
                    continue;
                }
                let color = match chunk.as_bytes()[0] {
                    b'+' => Color::Black,
                    b'-' => Color::White,
                    _ => return Err(format!("駒の先後がない: {chunk}")),
                };
                let pt = piece_type(&chunk[1..3]).ok_or_else(|| format!("不明な駒: {chunk}"))?;
                self.board[rank][col] = Some((color, pt));
            }
            self.placed = true;
            Ok(())
        }

        /// `PI` を読む。平手を敷いてから、続く「マス+駒」の分を取り除く。
        fn apply_pi(&mut self, rest: &str) -> Result<(), String> {
            for (rank, row) in HIRATE.iter().enumerate() {
                self.set_row(rank, row)?;
            }
            for group in groups(rest, 4)? {
                let (rank, col) =
                    cell(&group[0..2]).ok_or_else(|| format!("不明なマス: {group}"))?;
                self.board[rank][col] = None;
            }
            Ok(())
        }

        /// `P+00FU` のような持ち駒・駒配置の行を読む。
        fn apply_hand(&mut self, color: Color, rest: &str) -> Result<(), String> {
            if rest.trim() == "AL" {
                return Err("残り全部（AL）の指定は読めない".to_string());
            }
            for group in groups(rest, 4)? {
                let pt = piece_type(&group[2..4]).ok_or_else(|| format!("不明な駒: {group}"))?;
                if &group[0..2] == "00" {
                    self.hands[color.index()][pt.index()] += 1;
                } else {
                    let (rank, col) =
                        cell(&group[0..2]).ok_or_else(|| format!("不明なマス: {group}"))?;
                    self.board[rank][col] = Some((color, pt));
                    self.placed = true;
                }
            }
            Ok(())
        }

        /// 組み立てた盤面をsfenにする。手数は1で固定する。
        fn to_sfen(&self, side: Color) -> Result<String, String> {
            if !self.placed {
                return Err("開始局面の指定がない".to_string());
            }
            let mut board = String::new();
            for (rank, row) in self.board.iter().enumerate() {
                if rank > 0 {
                    board.push('/');
                }
                let mut empty = 0;
                for cell in row {
                    let Some((color, pt)) = cell else {
                        empty += 1;
                        continue;
                    };
                    if empty > 0 {
                        board.push_str(&empty.to_string());
                        empty = 0;
                    }
                    let s = pt
                        .to_sfen()
                        .ok_or_else(|| "駒をsfenにできない".to_string())?;
                    if *color == Color::Black {
                        board.push_str(s);
                    } else {
                        board.push_str(&s.to_ascii_lowercase());
                    }
                }
                if empty > 0 {
                    board.push_str(&empty.to_string());
                }
            }
            let mut hands = String::new();
            for (index, color) in [Color::Black, Color::White].iter().enumerate() {
                for pt in HAND_ORDER {
                    let n = self.hands[index][pt.index()];
                    if n == 0 {
                        continue;
                    }
                    if n > 1 {
                        hands.push_str(&n.to_string());
                    }
                    let s = pt
                        .to_sfen()
                        .ok_or_else(|| "手駒をsfenにできない".to_string())?;
                    if *color == Color::Black {
                        hands.push_str(s);
                    } else {
                        hands.push_str(&s.to_ascii_lowercase());
                    }
                }
            }
            if hands.is_empty() {
                hands.push('-');
            }
            let stm = if side == Color::Black { "b" } else { "w" };
            Ok(format!("{board} {stm} {hands} 1"))
        }
    }

    /// 固定長のフィールドへ切る。半端が出たらエラーにする。
    fn groups(s: &str, width: usize) -> Result<Vec<&str>, String> {
        let s = s.trim();
        if !s.is_ascii() || !s.len().is_multiple_of(width) {
            return Err(format!("{width}文字ずつに分けられない: {s}"));
        }
        Ok((0..s.len() / width)
            .map(|i| &s[i * width..i * width + width])
            .collect())
    }

    /// CSAの指し手（"+7776FU"）を合法手の中から選ぶ。
    pub fn resolve(pos: &Position, token: &str) -> Result<Move, String> {
        let b = token.as_bytes();
        if b.len() != 7 || !token.is_ascii() {
            return Err(format!("指し手の形でない: {token}"));
        }
        let color = match b[0] {
            b'+' => Color::Black,
            b'-' => Color::White,
            _ => return Err(format!("先後がない: {token}")),
        };
        if color != pos.side_to_move() {
            return Err(format!("手番と合わない: {token}"));
        }
        let pt = piece_type(&token[5..7]).ok_or_else(|| format!("不明な駒: {token}"))?;
        let to = square(&token[3..5]).ok_or_else(|| format!("不明な移動先: {token}"))?;
        let from = &token[1..3];
        let from_sq = if from == "00" {
            None
        } else {
            Some(square(from).ok_or_else(|| format!("不明な移動元: {token}"))?)
        };
        // 不成を含む全合法手から選ぶ。相手が不成を指した棋譜を落とさない
        let mut list = MoveList::default();
        generate_legal(pos, true, &mut list);
        for &m in &list {
            let hit = match from_sq {
                None => m.is_drop() && m.to() == to && m.drop_piece_type() == pt,
                Some(f) => {
                    !m.is_drop()
                        && m.from_sq() == f
                        && m.to() == to
                        && m.piece_after().piece_type() == pt
                }
            };
            if hit {
                return Ok(m);
            }
        }
        Err(format!("合法手にない: {token}"))
    }

    /// 1つの棋譜を読み、開始局面からmax_ply手目までの局面を返す。
    /// 先頭が開始局面（ply 0）で、i番目がply iの局面になる。
    pub fn positions(text: &str, max_ply: u16) -> Result<Vec<Position>, String> {
        let mut builder = Builder::new();
        let mut out: Vec<Position> = Vec::new();
        let mut current: Option<Position> = None;
        for stmt in statements(text) {
            match &mut current {
                // 開始局面が決まるまでは盤面の指定を集める
                None => {
                    if stmt == "+" || stmt == "-" {
                        let side = if stmt == "+" {
                            Color::Black
                        } else {
                            Color::White
                        };
                        let sfen = builder.to_sfen(side)?;
                        let pos = Position::from_sfen(&sfen)
                            .map_err(|e| format!("開始局面を作れない（{sfen}）: {e:?}"))?;
                        out.push(pos.clone());
                        current = Some(pos);
                    } else if let Some(rest) = stmt.strip_prefix("PI") {
                        builder.apply_pi(rest)?;
                    } else if let Some(rest) = stmt.strip_prefix("P+") {
                        builder.apply_hand(Color::Black, rest)?;
                    } else if let Some(rest) = stmt.strip_prefix("P-") {
                        builder.apply_hand(Color::White, rest)?;
                    } else if let Some(rest) = stmt.strip_prefix('P') {
                        // P1〜P9。それ以外のP行は読み飛ばす
                        let Some(rank) = rest
                            .as_bytes()
                            .first()
                            .and_then(|b| b.checked_sub(b'1'))
                            .map(usize::from)
                            .filter(|r| *r < 9)
                        else {
                            continue;
                        };
                        builder.set_row(rank, &rest[1..])?;
                    }
                    // ヘッダ（V・N・$）と時間・コメントは読み飛ばす
                }
                Some(pos) => {
                    if stmt.starts_with('%') {
                        break;
                    }
                    if !stmt.starts_with('+') && !stmt.starts_with('-') {
                        continue;
                    }
                    // 7文字でない +/- は投了などの特殊な宣言なので、そこで終わる
                    if stmt.len() != 7 {
                        break;
                    }
                    pos.do_move(resolve(pos, stmt)?);
                    out.push(pos.clone());
                    if out.len() as u16 > max_ply {
                        break;
                    }
                }
            }
        }
        if current.is_none() {
            return Err("開始局面の指定がない".to_string());
        }
        Ok(out)
    }

    /// 棋譜を文へ切る。P行は空白が意味を持つので触らない。指し手・時間・
    /// 特殊手はカンマで連ねられることがあるので、そこだけ分ける。
    fn statements(text: &str) -> Vec<&str> {
        let mut out = Vec::new();
        for raw in text.lines() {
            let line = raw.trim_end_matches('\r');
            let head = line.trim_start();
            let Some(first) = head.as_bytes().first() else {
                continue;
            };
            if *first == b'\'' {
                continue;
            }
            if matches!(first, b'+' | b'-' | b'%' | b'T') {
                for s in head.split(',') {
                    let s = s.trim();
                    if !s.is_empty() {
                        out.push(s);
                    }
                }
            } else {
                out.push(line);
            }
        }
        out
    }
}

/// 種になる局面。定跡のキーと、それがどの棋譜の何手目に現れたかを持つ。
#[derive(Clone, PartialEq, Eq, Debug)]
struct Seed {
    ply: u16,
    key: String,
    origin: String,
}

/// 種の抽出結果。読めなかった棋譜は理由付きで残す。
struct Seeds {
    seeds: Vec<Seed>,
    games: usize,
    skipped: Vec<(String, String)>,
}

/// 入力に並んだCSAを列挙する。ディレクトリは再帰的にたどり、拡張子が
/// csaのファイルだけを拾う。名指しされたファイルは拡張子を問わない。
/// 順序はパスの昇順で固定する（ADR-0152の決定論）。
fn collect_csa_files(inputs: &[String]) -> std::io::Result<Vec<PathBuf>> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                walk(&path, out)?;
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("csa"))
            {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    for input in inputs {
        let path = Path::new(input);
        if path.is_dir() {
            walk(path, &mut out)?;
        } else {
            out.push(path.to_path_buf());
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// CSA群から種になる局面を集める。
///
/// 同じ局面は棋譜をまたいで何度も現れるので、定跡のキーで1つにまとめる。
/// 残すのは最も浅いplyのもので、並びは (ply昇順, キーの辞書順) にする。
/// 入力ファイルの並びが変わっても同じ種列が出る。
fn collect_seeds(files: &[PathBuf], max_ply: u16) -> Seeds {
    let mut best: HashMap<String, (u16, String)> = HashMap::new();
    let mut skipped = Vec::new();
    let mut games = 0usize;
    for path in files {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into(),
        );
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                skipped.push((name, e.to_string()));
                continue;
            }
        };
        let positions = match csa::positions(&text, max_ply) {
            Ok(p) => p,
            Err(e) => {
                skipped.push((name, e));
                continue;
            }
        };
        games += 1;
        for (ply, pos) in positions.iter().enumerate() {
            let key = book_key(pos);
            let ply = ply as u16;
            match best.entry(key) {
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert((ply, name.clone()));
                }
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    if ply < e.get().0 {
                        e.insert((ply, name.clone()));
                    }
                }
            }
        }
    }
    let mut seeds: Vec<Seed> = best
        .into_iter()
        .map(|(key, (ply, origin))| Seed { ply, key, origin })
        .collect();
    // キーは重複しないので、この2つで全順序になる
    seeds.sort_by(|a, b| a.ply.cmp(&b.ply).then_with(|| a.key.cmp(&b.key)));
    Seeds {
        seeds,
        games,
        skipped,
    }
}

/// 棋譜に現れた局面を種にして定跡へ足す（ADR-0152）。
///
/// genとの違いは掘る順序である。genは初期局面から評価値の損失が小さい順に
/// 広げるが、seedは実戦に現れた局面をそのまま種にする。実戦の分布に定跡を
/// 追従させるための経路で、手の選定は自前の探索で行う（棋譜の指し手は
/// 使わない）。
///
/// 定期実行するので決定論を優先する。探索は1スレッド固定で、既に定跡が
/// 持つ局面は探索せずに飛ばす。追加が0なら書き出しもしないため、2度目の
/// 実行は定跡ファイルを変えない。
fn seed(cfg: &Config) -> std::io::Result<()> {
    println!(
        "BookSeed: max_ply={} width={} depth={} hash={}MB max_add={} margin={}",
        cfg.max_ply, cfg.width, cfg.depth, cfg.hash_mb, cfg.max_positions, cfg.margin
    );
    let files = collect_csa_files(&cfg.games)?;
    if files.is_empty() {
        println!("棋譜が見つからない（--games を確認）");
        return Ok(());
    }
    let found = collect_seeds(&files, cfg.max_ply);
    println!(
        "棋譜{}件のうち{}件を読み、種{}局面を得た（読めなかった棋譜{}件）",
        files.len(),
        found.games,
        found.seeds.len(),
        found.skipped.len()
    );
    for (name, why) in found.skipped.iter().take(20) {
        println!("  読めない棋譜: {name}: {why}");
    }

    let mut book = Book::load(&cfg.out)?;
    println!("{} は{}局面を持っている", cfg.out, book.len());
    let stop = match &cfg.stop_file {
        Some(p) => StopFile::at(PathBuf::from(p)),
        None => StopFile::beside(Path::new(&cfg.out)),
    };
    stop.clear_stale();
    println!("止めるには: touch {}", stop.path().display());

    let (pool, sink) = make_pool(cfg)?;
    let started = std::time::Instant::now();
    let mut added = 0usize;
    let mut known = 0usize;
    let mut failed = 0usize;
    let mut stopped = false;
    for s in &found.seeds {
        if added >= cfg.max_positions {
            println!("--max-positions {} に達した", cfg.max_positions);
            break;
        }
        // 定跡が持つ局面は探索しない。これが冪等性の要になる
        if book.lines.contains_key(&s.key) {
            known += 1;
            continue;
        }
        // 1局面に入る前に見る。探索の途中では止めない（ADR-0123）
        if stop.requested() {
            println!("停止ファイルを見つけた: {}", stop.path().display());
            stopped = true;
            break;
        }
        let Ok(pos) = Position::from_sfen(&format!("{} 1", s.key)) else {
            println!("読めない局面を飛ばす: {}", s.key);
            failed += 1;
            continue;
        };
        let lines = prune_by_margin(search_lines(&pos, cfg, &pool, &sink), cfg.margin);
        if lines.is_empty() {
            failed += 1;
            continue;
        }
        book.insert(s.key.clone(), lines.clone());
        added += 1;
        let secs = started.elapsed().as_secs();
        println!(
            "[{added:>5}/{} {secs:>6}s] ply={:>2} 手={} 評価={} 候補{} 由来={}",
            cfg.max_positions,
            s.ply,
            lines[0].0,
            lines[0].1,
            lines.len(),
            s.origin,
        );
        if added.is_multiple_of(cfg.save_every) {
            book.save(&cfg.out, cfg.depth)?;
            println!("  途中書き出し: {}局面（{secs}秒）", book.len());
        }
    }
    pool.quit();

    // 追加が0なら触らない。定期実行で同じ入力を渡しても定跡が変わらない
    if added > 0 {
        book.save(&cfg.out, cfg.depth)?;
    }
    println!(
        "{} へ{added}局面を追加した（既にあった{known}局面、探索できなかった{failed}局面、全{}局面、{}秒）",
        cfg.out,
        book.len(),
        started.elapsed().as_secs()
    );
    if stopped {
        // 消しておかないと、続きのつもりの次回が何もせずに終わる
        stop.consume();
        println!("停止した。同じコマンドで続きから足せる");
    }
    Ok(())
}

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
    let (pool, sink) = make_pool(cfg)?;

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

/// 定跡がどこまで網羅できているかを数える（ADR-0146）。
///
/// 見るのは「相手の手が何であっても定跡を引けるか」である。plyを1つ進める
/// たびに、その層の全合法手のうち何手ぶんの局面を定跡が持っているかを出す。
/// 網羅できていない手が現れた時点で、そこから先は定跡が外れうる。
fn stats(cfg: &Config) -> std::io::Result<()> {
    let book = Book::load(&cfg.out)?;
    if book.len() == 0 {
        eprintln!("{} に定跡がない", cfg.out);
        return Ok(());
    }
    println!("{}: {}局面", cfg.out, book.len());
    println!();
    println!("| ply | 到達局面 | 合法手 | 定跡にある | 網羅率 |");
    println!("|---|---|---|---|---|");

    // その層で「定跡が持っている局面」だけを次の層へ伸ばす。持っていない
    // 局面から先を数えても、実戦ではそこへ到達する前に定跡が切れている
    let root = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
    let mut frontier = vec![root];
    for ply in 1..=cfg.ply {
        let mut next: Vec<Position> = Vec::new();
        let mut legal_total = 0usize;
        let mut covered = 0usize;
        let mut seen: HashSet<String> = HashSet::new();
        for pos in &frontier {
            let mut list = MoveList::default();
            generate_legal(pos, false, &mut list);
            for &m in &list {
                let mut child = pos.clone();
                child.do_move(m);
                let key = book_key(&child);
                if !seen.insert(key.clone()) {
                    continue;
                }
                legal_total += 1;
                if book.lines.contains_key(&key) {
                    covered += 1;
                    next.push(child);
                }
            }
        }
        if legal_total == 0 {
            break;
        }
        println!(
            "| {ply} | {} | {legal_total} | {covered} | {:.1}% |",
            frontier.len(),
            covered as f64 * 100.0 / legal_total as f64,
        );
        if covered == 0 {
            break;
        }
        frontier = next;
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let sub = args.first().map(String::as_str).unwrap_or("");
    if sub != "gen" && sub != "seed" && sub != "refresh" && sub != "stats" {
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
        games: Vec::new(),
        max_ply: 24,
    };
    let mut threads_given = false;
    let mut i = 1;
    while i < args.len() {
        // --games だけは値を複数取る。ディレクトリとファイルを混ぜて渡せる
        if args[i] == "--games" {
            let mut j = i + 1;
            while j < args.len() && !args[j].starts_with("--") {
                cfg.games.push(args[j].clone());
                j += 1;
            }
            i = j;
            continue;
        }
        let val = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--out" => cfg.out = val,
            "--eval" => cfg.eval = val,
            "--ply" => cfg.ply = val.parse().unwrap_or(cfg.ply),
            "--width" => cfg.width = val.parse::<usize>().unwrap_or(cfg.width).max(1),
            "--full-ply" => cfg.full_ply = val.parse().unwrap_or(cfg.full_ply),
            "--depth" => cfg.depth = val.parse().unwrap_or(cfg.depth),
            "--hash" => cfg.hash_mb = val.parse().unwrap_or(cfg.hash_mb),
            "--max-ply" => cfg.max_ply = val.parse().unwrap_or(cfg.max_ply),
            "--threads" => {
                cfg.threads = val.parse::<usize>().unwrap_or(cfg.threads).max(1);
                threads_given = true;
            }
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
    if sub == "seed" {
        // マルチスレッド探索は同じ局面でも選ぶ手が揺れる。決定論を優先して
        // 1スレッドに固定し、指定されたら黙って落とさずに知らせる（ADR-0152）
        if threads_given {
            eprintln!("seed は --threads を受け付けない（決定論のため1スレッド固定）");
            std::process::exit(3);
        }
        cfg.threads = 1;
        if cfg.games.is_empty() {
            eprintln!("seed には --games が要る");
            std::process::exit(3);
        }
    }
    if let Some(dir) = std::path::Path::new(&cfg.out).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let result = match sub {
        "seed" => seed(&cfg),
        "refresh" => refresh(&cfg),
        "stats" => stats(&cfg),
        _ => generate(&cfg),
    };
    if let Err(e) = result {
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

    /// 平手の盤面指定。floodgateのCSAと同じ書式にしてある。
    const BOARD: &str = "\
V2.2
N+Himawari+test
N-Opponent
$EVENT:test
P1-KY-KE-GI-KI-OU-KI-GI-KE-KY
P2 * -HI *  *  *  *  * -KA *
P3-FU-FU-FU-FU-FU-FU-FU-FU-FU
P4 *  *  *  *  *  *  *  *  *
P5 *  *  *  *  *  *  *  *  *
P6 *  *  *  *  *  *  *  *  *
P7+FU+FU+FU+FU+FU+FU+FU+FU+FU
P8 * +KA *  *  *  *  * +HI *
P9+KY+KE+GI+KI+OU+KI+GI+KE+KY
+
";

    /// 角交換から銀で取り返す4手。
    const GAME_A: &str = "+7776FU\nT5\n-3334FU\nT7\n+8822UM\nT3\n-3122GI\nT4\n%TORYO\n";
    /// 相掛かりの出だし。
    const GAME_B: &str = "+2726FU\nT2\n-8384FU\nT2\n+2625FU\nT1\n-8485FU\nT1\n%CHUDAN\n";
    /// 途中まではGAME_Aと同じで、3手目が違う。合流の重複を作る。
    const GAME_C: &str = "+7776FU\nT5\n-3334FU\nT7\n+2726FU\nT3\n%TORYO\n";

    fn game(moves: &str) -> String {
        format!("{BOARD}{moves}")
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "himawari-book-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("作業ディレクトリ");
        dir
    }

    #[test]
    fn csa_board_reads_as_startpos() {
        let got = csa::positions(&game(""), 24).expect("読める");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].to_sfen(), SFEN_STARTPOS);
    }

    #[test]
    fn csa_pi_is_the_same_as_the_written_board() {
        let got = csa::positions("V2.2\nPI\n+\n", 24).expect("読める");
        assert_eq!(got[0].to_sfen(), SFEN_STARTPOS);
    }

    #[test]
    fn csa_moves_resolve_to_usi() {
        let got = csa::positions(&game(GAME_A), 24).expect("読める");
        // 開始局面＋4手ぶん
        assert_eq!(got.len(), 5);
        // 8八角が2二へ成って角を取り、3一銀が取り返す
        assert_eq!(
            got[3].to_sfen(),
            "lnsgkgsnl/1r5+B1/pppppp1pp/6p2/9/2P6/PP1PPPPPP/7R1/LNSGKGSNL w B 4"
        );
        // 取り返した側にも角が入る
        assert_eq!(got[4].to_sfen().split(' ').nth(2), Some("Bb"));
    }

    #[test]
    fn csa_resolve_reads_promotion_and_drop() {
        let pos = Position::from_sfen(SFEN_STARTPOS).expect("startpos");
        assert_eq!(
            csa::resolve(&pos, "+7776FU").expect("合法").to_usi(),
            "7g7f"
        );
        // 手番が合わない指し手は読めない
        assert!(csa::resolve(&pos, "-7776FU").is_err());
        // 動かせない駒も読めない
        assert!(csa::resolve(&pos, "+7775FU").is_err());

        let promoted = Position::from_sfen(
            "lnsgkgsnl/1r5b1/pppppp1pp/6p2/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL b - 1",
        )
        .expect("sfen");
        assert_eq!(
            csa::resolve(&promoted, "+8822UM").expect("合法").to_usi(),
            "8h2b+"
        );

        // 2八の飛車を手駒に持ち替えた局面
        let with_hand =
            Position::from_sfen("lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B7/LNSGKGSNL b R 1")
                .expect("sfen");
        assert_eq!(
            csa::resolve(&with_hand, "+0055HI").expect("合法").to_usi(),
            "R*5e"
        );
    }

    #[test]
    fn csa_rejects_unresolvable_game() {
        // 5五に動かせる歩はない
        let err = csa::positions(&game("+7755FU\n"), 24)
            .map(|p| p.len())
            .expect_err("読めない");
        assert!(err.contains("合法手にない"), "{err}");
    }

    #[test]
    fn csa_stops_at_max_ply() {
        let got = csa::positions(&game(GAME_A), 2).expect("読める");
        assert_eq!(got.len(), 3);
    }

    #[test]
    fn seed_extraction_is_independent_of_input_order() {
        let dir = temp_dir("seeds");
        for (name, moves) in [("b.csa", GAME_B), ("a.csa", GAME_A), ("c.csa", GAME_C)] {
            std::fs::write(dir.join(name), game(moves)).expect("書き出し");
        }
        let as_dir = collect_csa_files(&[dir.to_string_lossy().into()]).expect("列挙");
        let shuffled: Vec<String> = ["c.csa", "a.csa", "b.csa"]
            .iter()
            .map(|n| dir.join(n).to_string_lossy().into())
            .collect();
        let as_files = collect_csa_files(&shuffled).expect("列挙");
        assert_eq!(as_dir, as_files, "並べ替えても同じ順序になる");

        let first = collect_seeds(&as_dir, 24).seeds;
        let second = collect_seeds(&as_files, 24).seeds;
        assert_eq!(first, second);

        // 3局で 5 + 5 + 4 = 14 局面。AとCは2手目まで同じなので3つ重なり、
        // Bは初期局面だけが重なる。残るのは 14 - 3 - 1 = 10 局面
        assert_eq!(first.len(), 10);
        assert_eq!(first[0].ply, 0);
        // plyの昇順、同じplyはキーの辞書順
        for w in first.windows(2) {
            assert!((w[0].ply, &w[0].key) < (w[1].ply, &w[1].key));
        }
        // 重なる局面の由来は、パス順で先に来る棋譜になる
        assert_eq!(first[0].origin, "a.csa");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn seed_skips_unreadable_games_and_counts_them() {
        let dir = temp_dir("skip");
        std::fs::write(dir.join("ok.csa"), game(GAME_A)).expect("書き出し");
        std::fs::write(dir.join("ng.csa"), game("+7755FU\n")).expect("書き出し");
        // 拡張子がcsaでないファイルは拾わない
        std::fs::write(dir.join("note.txt"), "not a game").expect("書き出し");

        let files = collect_csa_files(&[dir.to_string_lossy().into()]).expect("列挙");
        assert_eq!(files.len(), 2);
        let found = collect_seeds(&files, 24);
        assert_eq!(found.games, 1);
        assert_eq!(found.skipped.len(), 1);
        assert_eq!(found.skipped[0].0, "ng.csa");
        assert_eq!(found.seeds.len(), 5);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 実際に探索する。深さ1・1局面に絞り、CIで重くならないようにする。
    /// 評価関数は渡さないので駒得だけで指すが、確かめたいのは定跡の
    /// 追記と冪等性なので影響しない。
    #[test]
    fn seed_appends_then_leaves_the_book_untouched() {
        let dir = temp_dir("run");
        std::fs::write(dir.join("a.csa"), game(GAME_A)).expect("書き出し");
        let out = dir.join("book.db").to_string_lossy().into_owned();
        let mut cfg = Config {
            out: out.clone(),
            eval: String::new(),
            ply: 24,
            width: 2,
            full_ply: 0,
            depth: 1,
            hash_mb: 1,
            threads: 1,
            max_positions: 2,
            margin: 100,
            save_every: 25,
            stop_file: None,
            games: vec![dir.to_string_lossy().into()],
            max_ply: 24,
        };
        let seeds = collect_seeds(&collect_csa_files(&cfg.games).expect("列挙"), 24).seeds;
        assert_eq!(seeds.len(), 5);

        seed(&cfg).expect("1回目");
        let book = Book::load(&out).expect("読める");
        assert_eq!(book.len(), 2, "--max-positions で追加を2局面に切る");
        // 種はply昇順なので、先に入るのは開始局面と1手目の局面になる
        assert_eq!(book.order, vec![seeds[0].key.clone(), seeds[1].key.clone()]);

        // 上限を上げると、持っていない残りだけを足す
        cfg.max_positions = 10;
        seed(&cfg).expect("2回目");
        let full = std::fs::read(&out).expect("定跡ができている");
        let book = Book::load(&out).expect("読める");
        assert_eq!(book.len(), seeds.len());

        seed(&cfg).expect("3回目");
        let again = std::fs::read(&out).expect("定跡が残っている");
        assert_eq!(full, again, "全部持っていれば定跡は変わらない");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
