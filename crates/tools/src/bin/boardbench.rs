//! 盤面操作だけを1回あたりのnsで測る（ADR-0239の手順0）。
//!
//! `bench` は探索ごと測るので、評価関数の推論と置換表が混ざり、背景の
//! 負荷で±1.5%動く。利きの表の維持費や、1手詰め・SEE・合法性判定の
//! 速さの差を見るには、盤面だけを歩く計測が要る。
//!
//! 各局面から合法手を全幅で深さNまで進め戻し、葉ではさらに駒取りだけを
//! 1手進める。同じ歩き方を5段で回し、段ごとに足す処理を変える。
//!
//! 1. walk  … 進め戻しだけ（指し手生成を含む）
//! 2. mate  … walkに加え、王手でない各ノードで `mate_1ply`
//! 3. see   … walkに加え、各ノードの駒取りで `see_ge`（閾値0と−90）
//! 4. gen   … walkに加え、王手でない各ノードで擬似合法手を生成する
//! 5. legal … genに加え、その擬似合法手ごとに `is_legal`
//!
//! 1回あたりのnsは段の差を呼び出し回数で割って出す。`is_legal` だけは
//! 擬似合法手の生成を差し引くためにgenとの差を取る。
//!
//! 手を進め戻すのは探索と同じ `SearchBoard` である。局面に付いて回る
//! 状態（利きの表など）を足すと、walkの値にその維持費が載る。
//!
//! 出力は1周1行のJSONである。`check` は `mate_1ply` の返す手、`see_ge` と
//! `is_legal` の返り値、各段の呼び出し回数を混ぜたハッシュで、比べる2本で
//! 一致しなければ返す値が変わっている。突き合わせは `hmwr bench board` が行う。

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use anyhow::Result;
use clap::Parser;

use himawari_core::{GenType, Move, MoveList, Piece};
use himawari_engine::SearchBoard;
use himawari_tools::exit;
use himawari_tools::positions::{builtin_positions, parse_position, read_positions};

/// 組み込みの4局面を指す `--positions` の値。
const BUILTIN: &str = "builtin";
/// 既定の局面。序中盤の組み込みと、終盤寄りの組（ADR-0230）。
const DEFAULT_POSITIONS: [&str; 2] = [BUILTIN, "openings/profile_late_positions.txt"];

#[derive(Parser)]
#[command(about = "盤面操作だけを1回あたりのnsで測る（ADR-0239）")]
struct Cli {
    /// 局面リストのファイル。重ねて指定できる。`builtin` は組み込みの4局面。
    /// 省略時は組み込みと openings/profile_late_positions.txt
    #[arg(long, value_name = "パス")]
    positions: Vec<String>,

    /// 全幅で進め戻す深さ。葉ではさらに駒取りだけを1手進める
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u32).range(1..))]
    depth: u32,

    /// 1段の中で局面の組を何回歩くか。既定の局面と深さで1周が1.5秒ほどになる
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..))]
    repeat: u32,

    /// 何周測るか。周ごとに1行のJSONを出す
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
    runs: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Walk,
    Mate,
    See,
    Gen,
    Legal,
}

const STAGES: [Stage; 5] = [
    Stage::Walk,
    Stage::Mate,
    Stage::See,
    Stage::Gen,
    Stage::Legal,
];

/// 1段を歩いた間の回数と、返り値を混ぜたハッシュ。
#[derive(Default)]
struct Tally {
    moves: u64,
    calls: u64,
    hash: u64,
}

impl Tally {
    #[inline(always)]
    fn mix(&mut self, v: u64) {
        // FNV-1aの64bit版。順序も含めて混ぜる
        self.hash = (self.hash ^ v).wrapping_mul(0x0000_0100_0000_01b3);
    }
}

#[inline(always)]
fn is_capture(b: &SearchBoard, m: Move) -> bool {
    !m.is_drop() && b.piece_on(m.to()) != Piece::EMPTY
}

#[inline(always)]
fn move_bits(m: Move) -> u64 {
    u64::from(m.to_move16().0) | (u64::from(m.piece_after().0) << 16)
}

/// 段ごとの処理をノードで1回行う。`list` はそのノードの合法手である。
#[inline(always)]
fn visit(b: &SearchBoard, stage: Stage, list: &MoveList, t: &mut Tally) {
    match stage {
        Stage::Walk => {}
        Stage::Mate => {
            if !b.in_check() {
                let r = b.mate_1ply();
                t.calls += 1;
                t.mix(r.map_or(0, move_bits));
            }
        }
        Stage::See => {
            for &m in list.as_slice() {
                if is_capture(b, m) {
                    let a = b.see_ge(m, 0);
                    let c = b.see_ge(m, -90);
                    t.calls += 2;
                    t.mix(u64::from(a) | (u64::from(c) << 1));
                }
            }
        }
        Stage::Gen | Stage::Legal => {
            if !b.in_check() {
                let mut pseudo = MoveList::default();
                b.generate(GenType::NonEvasions, false, &mut pseudo);
                if stage == Stage::Gen {
                    t.mix(pseudo.len() as u64);
                } else {
                    t.mix(pseudo.len() as u64);
                    let mut legal = 0u64;
                    for &m in pseudo.as_slice() {
                        legal = (legal << 1 | u64::from(b.is_legal(m))).rotate_left(7);
                    }
                    t.calls += pseudo.len() as u64;
                    t.mix(legal);
                }
            }
        }
    }
}

fn walk(b: &mut SearchBoard, depth: u32, stage: Stage, t: &mut Tally) {
    let mut list = MoveList::default();
    b.generate_legal(true, &mut list);
    visit(b, stage, &list, t);
    for &m in list.as_slice() {
        if depth == 0 && !is_capture(b, m) {
            continue;
        }
        b.do_move(m);
        t.moves += 1;
        if depth > 0 {
            walk(b, depth - 1, stage, t);
        }
        b.undo_move(m);
    }
}

/// 1段を測る。(所要ns, 回数) を返す。
fn run_stage(boards: &mut [SearchBoard], cli: &Cli, stage: Stage) -> (u64, Tally) {
    let mut t = Tally::default();
    let start = Instant::now();
    for _ in 0..cli.repeat {
        for b in boards.iter_mut() {
            walk(b, cli.depth, stage, &mut t);
        }
    }
    let ns = start.elapsed().as_nanos() as u64;
    (ns, std::hint::black_box(t))
}

fn per_call(ns: u64, base_ns: u64, calls: u64) -> f64 {
    if calls == 0 {
        return 0.0;
    }
    (ns as f64 - base_ns as f64) / calls as f64
}

fn load(cli: &Cli) -> Result<Vec<SearchBoard>> {
    let lists: Vec<String> = if cli.positions.is_empty() {
        DEFAULT_POSITIONS.iter().map(|s| s.to_string()).collect()
    } else {
        cli.positions.clone()
    };
    let mut boards = Vec::new();
    for list in &lists {
        let lines = if list == BUILTIN {
            builtin_positions()
        } else {
            read_positions(&PathBuf::from(list))?
        };
        for line in &lines {
            boards.push(SearchBoard::new(parse_position(line)?));
        }
    }
    Ok(boards)
}

fn run(cli: &Cli) -> Result<()> {
    let mut boards = load(cli)?;
    // 1段ぶん空回しして、表と分岐予測を温める
    run_stage(&mut boards, cli, Stage::Walk);

    for run in 1..=cli.runs {
        let mut ns = [0u64; STAGES.len()];
        let mut tallies: Vec<Tally> = Vec::with_capacity(STAGES.len());
        for (i, &stage) in STAGES.iter().enumerate() {
            let (n, t) = run_stage(&mut boards, cli, stage);
            ns[i] = n;
            tallies.push(t);
        }
        let mut check = Tally::default();
        for t in &tallies {
            check.mix(t.moves);
            check.mix(t.calls);
            check.mix(t.hash);
        }
        let moves = tallies[0].moves;
        let move_ns = if moves == 0 {
            0.0
        } else {
            ns[0] as f64 / moves as f64
        };
        println!(
            "{{\"run\":{run},\"positions\":{},\"depth\":{},\"repeat\":{},\
             \"moves\":{moves},\"move_ns\":{move_ns:.2},\
             \"mate_calls\":{},\"mate_ns\":{:.2},\
             \"see_calls\":{},\"see_ns\":{:.2},\
             \"legal_calls\":{},\"legal_ns\":{:.2},\
             \"stage_ms\":[{}],\"check\":\"{:016x}\"}}",
            boards.len(),
            cli.depth,
            cli.repeat,
            tallies[1].calls,
            per_call(ns[1], ns[0], tallies[1].calls),
            tallies[2].calls,
            per_call(ns[2], ns[0], tallies[2].calls),
            tallies[4].calls,
            per_call(ns[4], ns[3], tallies[4].calls),
            ns.iter()
                .map(|n| format!("{:.1}", *n as f64 / 1e6))
                .collect::<Vec<_>>()
                .join(","),
            check.hash,
        );
    }
    Ok(())
}

/// 計測を走らせるスレッドのスタック。深さ2の歩き方には十分に大きい。
const STACK_BYTES: usize = 64 << 20;

fn main() -> ExitCode {
    let cli = Cli::parse();
    // 計測は別スレッドで走らせる。メインスレッドのスタックは、起動時の
    // 引数と環境変数の長さで開始位置がずれる。同じバイナリでも、実験名で
    // 渡すか相対パスで渡すかで mate_1ply が 92ns と 154ns に分かれた。
    // スレッドのスタックはページ境界から始まるので、呼び方に左右されない。
    // 探索のワーカーも別スレッドで動くので、条件が揃う（ADR-0239）
    let worker = std::thread::Builder::new()
        .stack_size(STACK_BYTES)
        .spawn(move || run(&cli));
    let result = match worker {
        Ok(handle) => handle
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("計測のスレッドが落ちた"))),
        Err(e) => Err(anyhow::anyhow!("計測のスレッドを起こせない: {e}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("エラー: {e:#}");
            ExitCode::from(exit::RUNTIME)
        }
    }
}
