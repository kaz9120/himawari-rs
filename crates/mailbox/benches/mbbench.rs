//! 配列盤（mailbox）とcoreの盤面操作を、同じ局面・同じ歩き方で測る（ADR-0241の段階4）。
//!
//! 歩き方は `boardbench`（ADR-0239）の `walk` 段と同じにする。各局面から合法手
//! （`generate_legal`、all=true）を全幅で深さNまで進め戻し、葉ではさらに駒取りだけを1手進める。
//! 比べる相手はcoreの `Position` で、探索の `SearchBoard`（評価関数の差分を含む）は使わない。
//!
//! 段は4つあり、1周の中でcore・配列盤を交互に走らせる。
//!
//! 1. walk（core） … 進め戻しと、各ノードの合法手生成
//! 2. walk（配列盤）
//! 3. gen（core） … walkに加え、各ノードでもう1回合法手を生成する
//! 4. gen（配列盤）
//!
//! 出力は1周1行のJSONである。`move_ns` はwalkの所要を進めた手の数で割った値、
//! `gen_ns` は生成1回あたり（gen−walkを呼び出し回数で割る）、`do_undo_ns` は
//! walkから生成の分を引いて進めた手の数で割った値である。
//! `check_core` と `check_mb` は、歩いた手の列（升・成り・打った駒）を順に混ぜたハッシュで、
//! 生成の順がcoreと揃っていれば一致する。一致しなければ `"same":false` になる。
//!
//! 走らせ方（開発機が空いているときだけ。`hmwr queue pause` を先に打つ）:
//!
//! ```text
//! RUSTFLAGS="-C target-cpu=native" cargo bench -p himawari-mailbox --bench mbbench -- --runs 7
//! RUSTFLAGS="-C target-cpu=native" cargo bench -p himawari-mailbox --bench mbbench --features copy-undo -- --runs 7
//! ```

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use himawari_core::{MoveList as CoreList, Position as CorePos, generate_legal};
use himawari_mailbox::{Move, MoveList, Position};
use himawari_tools::positions::{builtin_positions, parse_position, read_positions};

const BUILTIN: &str = "builtin";

struct Cli {
    positions: Vec<String>,
    depth: u32,
    repeat: u32,
    runs: u32,
}

fn parse_args() -> Result<Cli, String> {
    let mut cli = Cli {
        positions: Vec::new(),
        depth: 2,
        repeat: 3,
        runs: 1,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} に値がない"));
        match a.as_str() {
            "--positions" => cli.positions.push(value("--positions")?),
            "--depth" => cli.depth = value("--depth")?.parse().map_err(|e| format!("{e}"))?,
            "--repeat" => cli.repeat = value("--repeat")?.parse().map_err(|e| format!("{e}"))?,
            "--runs" => cli.runs = value("--runs")?.parse().map_err(|e| format!("{e}"))?,
            // cargo bench が渡すフラグ
            "--bench" => {}
            _ => return Err(format!("知らない引数: {a}")),
        }
    }
    if cli.positions.is_empty() {
        let late = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../openings/profile_late_positions.txt");
        cli.positions = vec![BUILTIN.to_string(), late.to_string_lossy().into_owned()];
    }
    if cli.depth == 0 || cli.repeat == 0 || cli.runs == 0 {
        return Err("--depth・--repeat・--runs は1以上".into());
    }
    Ok(cli)
}

#[derive(Default)]
struct Tally {
    moves: u64,
    nodes: u64,
    calls: u64,
    hash: u64,
}

impl Tally {
    #[inline(always)]
    fn mix(&mut self, v: u64) {
        self.hash = (self.hash ^ v).wrapping_mul(0x0000_0100_0000_01b3);
    }
}

/// 両者で共通の手の符号。升は81升の通し番号。
#[inline(always)]
fn key(from: u64, to: u64, promote: bool, drop: u64) -> u64 {
    to | (from << 8) | (u64::from(promote) << 16) | (drop << 17)
}

#[inline(always)]
fn mb_key(m: Move) -> u64 {
    let sq = |i: u8| u64::from(himawari_mailbox::board::SQ81[i as usize]);
    if m.is_drop() {
        key(0, sq(m.to()), false, u64::from(m.drop_type()))
    } else {
        key(sq(m.from()), sq(m.to()), m.is_promote(), 0)
    }
}

#[inline(always)]
fn core_key(m: himawari_core::Move) -> u64 {
    if m.is_drop() {
        key(
            0,
            m.to().index() as u64,
            false,
            u64::from(m.drop_piece_type().0),
        )
    } else {
        key(
            m.from_sq().index() as u64,
            m.to().index() as u64,
            m.is_promote(),
            0,
        )
    }
}

fn walk_core(pos: &mut CorePos, depth: u32, extra_gen: bool, t: &mut Tally) {
    let mut list = CoreList::default();
    generate_legal(pos, true, &mut list);
    t.nodes += 1;
    if extra_gen {
        let mut again = CoreList::default();
        generate_legal(pos, true, &mut again);
        t.calls += 1;
        t.mix(again.len() as u64);
    }
    for &m in list.as_slice() {
        let capture = !m.is_drop() && !pos.piece_on(m.to()).is_empty();
        if depth == 0 && !capture {
            continue;
        }
        t.mix(core_key(m));
        pos.do_move(m);
        t.moves += 1;
        if depth > 0 {
            walk_core(pos, depth - 1, extra_gen, t);
        }
        pos.undo_move(m);
    }
}

fn walk_mb(pos: &mut Position, depth: u32, extra_gen: bool, t: &mut Tally) {
    let mut list = MoveList::default();
    pos.generate_legal(true, &mut list);
    t.nodes += 1;
    if extra_gen {
        let mut again = MoveList::default();
        pos.generate_legal(true, &mut again);
        t.calls += 1;
        t.mix(again.len() as u64);
    }
    for &m in list.as_slice() {
        let capture = !m.is_drop() && pos.piece_on(m.to()) != himawari_mailbox::board::EMPTY;
        if depth == 0 && !capture {
            continue;
        }
        t.mix(mb_key(m));
        pos.do_move(m);
        t.moves += 1;
        if depth > 0 {
            walk_mb(pos, depth - 1, extra_gen, t);
        }
        pos.undo_move(m);
    }
}

fn time<F: FnMut(&mut Tally)>(repeat: u32, mut f: F) -> (u64, Tally) {
    let mut t = Tally::default();
    let start = Instant::now();
    for _ in 0..repeat {
        f(&mut t);
    }
    (start.elapsed().as_nanos() as u64, std::hint::black_box(t))
}

fn run(cli: &Cli) -> Result<(), String> {
    let mut cores = Vec::new();
    let mut mbs = Vec::new();
    for list in &cli.positions {
        let lines = if list == BUILTIN {
            builtin_positions()
        } else {
            read_positions(&PathBuf::from(list)).map_err(|e| format!("{e:#}"))?
        };
        for line in &lines {
            let core = parse_position(line).map_err(|e| format!("{e:#}"))?;
            let mb = Position::from_sfen(&core.to_sfen()).map_err(|e| e.0.to_string())?;
            cores.push(core);
            mbs.push(mb);
        }
    }
    let d = cli.depth;
    let r = cli.repeat;
    let core_stage = |cores: &mut Vec<CorePos>, extra: bool| {
        time(r, |t| {
            cores.iter_mut().for_each(|p| walk_core(p, d, extra, t))
        })
    };
    let mb_stage = |mbs: &mut Vec<Position>, extra: bool| {
        time(r, |t| mbs.iter_mut().for_each(|p| walk_mb(p, d, extra, t)))
    };
    // 1段ぶん空回しして、表と分岐予測を温める
    core_stage(&mut cores, false);
    mb_stage(&mut mbs, false);

    let undo = if cfg!(feature = "copy-undo") {
        "copy"
    } else {
        "diff"
    };
    for run in 1..=cli.runs {
        let (cw, ct) = core_stage(&mut cores, false);
        let (mw, mt) = mb_stage(&mut mbs, false);
        let (cg, cgt) = core_stage(&mut cores, true);
        let (mg, mgt) = mb_stage(&mut mbs, true);
        let per = |ns: u64, n: u64| if n == 0 { 0.0 } else { ns as f64 / n as f64 };
        let gen_ns = |g: u64, w: u64, calls: u64| per(g.saturating_sub(w), calls);
        let c_gen = gen_ns(cg, cw, cgt.calls);
        let m_gen = gen_ns(mg, mw, mgt.calls);
        let c_du = per(
            cw.saturating_sub((c_gen * ct.nodes as f64) as u64),
            ct.moves,
        );
        let m_du = per(
            mw.saturating_sub((m_gen * mt.nodes as f64) as u64),
            mt.moves,
        );
        let c_move = per(cw, ct.moves);
        let m_move = per(mw, mt.moves);
        println!(
            "{{\"run\":{run},\"undo\":\"{undo}\",\"positions\":{},\"depth\":{d},\"repeat\":{r},\
             \"moves\":{},\"nodes\":{},\
             \"core_move_ns\":{c_move:.2},\"mb_move_ns\":{m_move:.2},\"ratio_move\":{:.3},\
             \"core_gen_ns\":{c_gen:.2},\"mb_gen_ns\":{m_gen:.2},\"ratio_gen\":{:.3},\
             \"core_do_undo_ns\":{c_du:.2},\"mb_do_undo_ns\":{m_du:.2},\
             \"check_core\":\"{:016x}\",\"check_mb\":\"{:016x}\",\"same\":{}}}",
            cores.len(),
            ct.moves,
            ct.nodes,
            m_move / c_move,
            m_gen / c_gen,
            ct.hash,
            mt.hash,
            ct.hash == mt.hash && ct.moves == mt.moves && cgt.hash == mgt.hash,
        );
    }
    Ok(())
}

/// 計測を走らせるスレッドのスタック（boardbenchと同じ理由で別スレッドにする）。
const STACK_BYTES: usize = 64 << 20;

fn main() -> ExitCode {
    let cli = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("エラー: {e}");
            return ExitCode::from(2);
        }
    };
    let worker = std::thread::Builder::new()
        .stack_size(STACK_BYTES)
        .spawn(move || run(&cli));
    let result = match worker {
        Ok(h) => h
            .join()
            .unwrap_or_else(|_| Err("計測のスレッドが落ちた".into())),
        Err(e) => Err(format!("計測のスレッドを起こせない: {e}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("エラー: {e}");
            ExitCode::from(3)
        }
    }
}
