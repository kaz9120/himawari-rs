//! 配列盤（mailbox）とcoreの盤面操作を、同じ局面・同じ歩き方で測る（ADR-0241の段階4）。
//!
//! 歩き方は `boardbench`（ADR-0239）の `walk` 段と同じにする。各局面から合法手
//! （`generate_legal`、all=true）を全幅で深さNまで進め戻し、葉ではさらに駒取りだけを1手進める。
//! 比べる相手はcoreの `Position` で、探索の `SearchBoard`（評価関数の差分を含む）は使わない。
//!
//! 段は7つあり（`Stage`）、段ごとにcore・配列盤を交互に走らせる。
//! walk・gen（段階4）に、段階5の1手詰め・SEE・王手判定・合法性判定の段を足した。
//! coreの1手詰めは探索と同じ `himawari_engine::mate::mate_1ply`（利きの地図入り）である。
//!
//! 出力は1周1行のJSONである。`move_ns` はwalkの所要を進めた手の数で割った値、
//! `gen_ns` は生成1回あたり（gen−walkを呼び出し回数で割る）、`do_undo_ns` は
//! walkから生成の分を引いて進めた手の数で割った値である。1手詰め・SEE・王手判定は
//! 段−walk、合法性判定は legal−pgen を呼び出し回数で割る。
//! `R_mid`・`R_late` は処理ごとの比をADR-0230の割合で重み付けした平均である（ADR-0241の段階5）。
//! `check_core` と `check_mb` は、歩いた手の列と各段の返す値を順に混ぜたハッシュで、
//! 生成の順と返す値がcoreと揃っていれば一致する。一致しなければ `"same":false` になる。
//!
//! `--stage <段> --side <core|mb>` を渡すと、空回しのwalkの後にその段を片側だけ走らせる（第10回）。
//! 命令数は段ごとに分けて取れないので、`/usr/bin/time -l` の命令数を `--stage walk` の実行との差で読む。
//! 1回あたりの命令数は (段−walk)÷呼び出し回数で、合法性判定だけは (legal−pgen)÷呼び出し回数である。
//!
//! `--extra <all|drop|capture|quiet|long>` は、配列盤の側で選んだ種類の手ごとに、進めて戻す1往復を足す（第15回）。
//! 足さない実行との命令数の差を `"extra"`（足した往復の数）で割ると、歩き方と生成を含まない進め戻し1往復の命令数になる。
//! longは飛び駒（香・角・飛・馬・竜）の盤上の手である。coreの側は `all` だけを受け付ける。
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

use himawari_core::{
    GenType as CoreGen, MoveList as CoreList, Position as CorePos, generate, generate_legal,
};
use himawari_engine::mate::mate_1ply;
use himawari_mailbox::board::SQ81;
use himawari_mailbox::{Move, MoveList, Position};
use himawari_tools::positions::{builtin_positions, parse_position, read_positions};

const BUILTIN: &str = "builtin";
/// `--extra` の手の種類（0は足さない）。1手ごとに、進めて戻す1往復を足す手を選ぶ。
static EXTRA: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

struct Cli {
    positions: Vec<String>,
    depth: u32,
    repeat: u32,
    runs: u32,
    /// 段を1つだけ走らせる（`--stage` と `--side`）。命令数を段ごとに出すため（ADR-0241の第10回）
    only: Option<(Stage, Side)>,
}

/// `--stage` で走らせる側。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Core,
    Mb,
}

fn parse_stage(s: &str) -> Result<Stage, String> {
    Ok(match s {
        "walk" => Stage::Walk,
        "gen" => Stage::Gen,
        "mate" => Stage::Mate,
        "see" => Stage::See,
        "check" => Stage::Check,
        "pgen" => Stage::Pgen,
        "legal" => Stage::Legal,
        _ => return Err(format!("知らない段: {s}")),
    })
}

fn parse_args() -> Result<Cli, String> {
    let mut cli = Cli {
        positions: Vec::new(),
        depth: 2,
        repeat: 3,
        runs: 1,
        only: None,
    };
    let mut stage = None;
    let mut side = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} に値がない"));
        match a.as_str() {
            "--positions" => cli.positions.push(value("--positions")?),
            "--depth" => cli.depth = value("--depth")?.parse().map_err(|e| format!("{e}"))?,
            "--repeat" => cli.repeat = value("--repeat")?.parse().map_err(|e| format!("{e}"))?,
            "--runs" => cli.runs = value("--runs")?.parse().map_err(|e| format!("{e}"))?,
            "--stage" => stage = Some(parse_stage(&value("--stage")?)?),
            "--extra" => {
                let k = match value("--extra")?.as_str() {
                    "all" => 1,
                    "drop" => 2,
                    "capture" => 3,
                    "quiet" => 4,
                    "long" => 5,
                    x => return Err(format!("--extra は all・drop・capture・quiet・long: {x}")),
                };
                EXTRA.store(k, std::sync::atomic::Ordering::Relaxed);
            }
            "--side" => {
                side = Some(match value("--side")?.as_str() {
                    "core" => Side::Core,
                    "mb" => Side::Mb,
                    x => return Err(format!("--side は core か mb: {x}")),
                })
            }
            // cargo bench が渡すフラグ
            "--bench" => {}
            _ => return Err(format!("知らない引数: {a}")),
        }
    }
    cli.only = match (stage, side) {
        (Some(st), Some(sd)) => Some((st, sd)),
        (None, None) => None,
        _ => return Err("--stage と --side は一緒に渡す".into()),
    };
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
    /// `--extra` で足した進め戻しの往復の数
    extra: u64,
    hash: u64,
    /// 配列盤の合法性判定の段で、次に読むノードの番号
    cursor: usize,
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
    let sq = |i: u8| u64::from(SQ81[i as usize]);
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

/// coreの指し手を配列盤の指し手へ直す（合法性判定の段の入力を作るときだけ使う）。
fn to_mb(m: himawari_core::Move) -> Move {
    use himawari_mailbox::board::IDX81;
    let pa = m.piece_after().0;
    if m.is_drop() {
        let c = if pa & 16 != 0 {
            himawari_mailbox::Color::White
        } else {
            himawari_mailbox::Color::Black
        };
        Move::drop(m.drop_piece_type().0, IDX81[m.to().index()], c)
    } else {
        Move::normal(
            IDX81[m.from_sq().index()],
            IDX81[m.to().index()],
            m.is_promote(),
            pa,
        )
    }
}

/// 段。walkに足す処理を変える。1回あたりのnsは、段の差を呼び出し回数で割って出す。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// 進め戻しと、各ノードの合法手生成
    Walk,
    /// walkに加え、各ノードでもう1回合法手を生成する
    Gen,
    /// walkに加え、王手でない各ノードで `mate_1ply`
    Mate,
    /// walkに加え、各ノードの駒取りで `see_ge`（閾値0と−90）
    See,
    /// walkに加え、各ノードの合法手ごとに `gives_check`
    Check,
    /// walkに加え、王手でない各ノードでcoreの擬似合法手（NonEvasions、all=false）を用意する。
    /// coreはその場で生成し、配列盤は前もって変換した列を読む
    Pgen,
    /// pgenに加え、その擬似合法手ごとに `is_legal`
    Legal,
}

const STAGES: [Stage; 7] = [
    Stage::Walk,
    Stage::Gen,
    Stage::Mate,
    Stage::See,
    Stage::Check,
    Stage::Pgen,
    Stage::Legal,
];

/// 配列盤の合法性判定の段で使う、coreの擬似合法手を変換した列。ノードを歩く順に並ぶ。
#[derive(Default)]
struct Pseudo {
    moves: Vec<Move>,
    /// ノードnの手は moves[offs[n]..offs[n+1]]
    offs: Vec<u32>,
}

#[inline(always)]
fn bits_of<I: Iterator<Item = bool>>(it: I) -> u64 {
    it.fold(0u64, |acc, b| (acc << 1 | u64::from(b)).rotate_left(7))
}

#[inline(always)]
fn visit_core(pos: &CorePos, stage: Stage, list: &CoreList, t: &mut Tally) {
    match stage {
        Stage::Walk => {}
        Stage::Gen => {
            let mut again = CoreList::default();
            generate_legal(pos, true, &mut again);
            t.calls += 1;
            t.mix(again.len() as u64);
        }
        Stage::Mate => {
            if !pos.in_check() {
                let r = mate_1ply(pos);
                t.calls += 1;
                t.mix(r.map_or(0, core_key));
            }
        }
        Stage::See => {
            for &m in list.as_slice() {
                if !m.is_drop() && !pos.piece_on(m.to()).is_empty() {
                    let a = pos.see_ge(m, 0);
                    let c = pos.see_ge(m, -90);
                    t.calls += 2;
                    t.mix(u64::from(a) | (u64::from(c) << 1));
                }
            }
        }
        Stage::Check => {
            t.calls += list.len() as u64;
            t.mix(bits_of(list.as_slice().iter().map(|&m| pos.gives_check(m))));
        }
        Stage::Pgen | Stage::Legal => {
            if !pos.in_check() {
                let mut pseudo = CoreList::default();
                generate(pos, CoreGen::NonEvasions, false, &mut pseudo);
                t.mix(pseudo.len() as u64);
                if stage == Stage::Legal {
                    t.calls += pseudo.len() as u64;
                    t.mix(bits_of(pseudo.as_slice().iter().map(|&m| pos.is_legal(m))));
                } else {
                    t.mix(
                        pseudo
                            .as_slice()
                            .iter()
                            .fold(0, |a, m| a ^ m.to().index() as u64),
                    );
                }
            }
        }
    }
}

#[inline(always)]
fn visit_mb(pos: &mut Position, stage: Stage, list: &MoveList, pseudo: &Pseudo, t: &mut Tally) {
    match stage {
        Stage::Walk => {}
        Stage::Gen => {
            let mut again = MoveList::default();
            pos.generate_legal(true, &mut again);
            t.calls += 1;
            t.mix(again.len() as u64);
        }
        Stage::Mate => {
            if !pos.in_check() {
                let r = pos.mate_1ply();
                t.calls += 1;
                t.mix(r.map_or(0, mb_key));
            }
        }
        Stage::See => {
            for &m in list.as_slice() {
                if !m.is_drop() && pos.piece_on(m.to()) != himawari_mailbox::board::EMPTY {
                    let a = pos.see_ge(m, 0);
                    let c = pos.see_ge(m, -90);
                    t.calls += 2;
                    t.mix(u64::from(a) | (u64::from(c) << 1));
                }
            }
        }
        Stage::Check => {
            t.calls += list.len() as u64;
            t.mix(bits_of(list.as_slice().iter().map(|&m| pos.gives_check(m))));
        }
        Stage::Pgen | Stage::Legal => {
            let n = t.cursor;
            t.cursor += 1;
            if !pos.in_check() {
                let ms = &pseudo.moves[pseudo.offs[n] as usize..pseudo.offs[n + 1] as usize];
                t.mix(ms.len() as u64);
                if stage == Stage::Legal {
                    t.calls += ms.len() as u64;
                    t.mix(bits_of(ms.iter().map(|&m| pos.is_legal(m))));
                } else {
                    t.mix(
                        ms.iter()
                            .fold(0, |a, m| a ^ u64::from(SQ81[m.to() as usize])),
                    );
                }
            }
        }
    }
}

fn walk_core(pos: &mut CorePos, depth: u32, stage: Stage, t: &mut Tally) {
    let mut list = CoreList::default();
    generate_legal(pos, true, &mut list);
    t.nodes += 1;
    visit_core(pos, stage, &list, t);
    for &m in list.as_slice() {
        let capture = !m.is_drop() && !pos.piece_on(m.to()).is_empty();
        if depth == 0 && !capture {
            continue;
        }
        t.mix(core_key(m));
        if EXTRA.load(std::sync::atomic::Ordering::Relaxed) == 1 {
            pos.do_move(m);
            pos.undo_move(m);
            t.extra += 1;
        }
        pos.do_move(m);
        t.moves += 1;
        if depth > 0 {
            walk_core(pos, depth - 1, stage, t);
        }
        pos.undo_move(m);
    }
}

fn walk_mb(pos: &mut Position, depth: u32, stage: Stage, pseudo: &Pseudo, t: &mut Tally) {
    let mut list = MoveList::default();
    pos.generate_legal(true, &mut list);
    t.nodes += 1;
    visit_mb(pos, stage, &list, pseudo, t);
    for &m in list.as_slice() {
        let capture = !m.is_drop() && pos.piece_on(m.to()) != himawari_mailbox::board::EMPTY;
        if depth == 0 && !capture {
            continue;
        }
        t.mix(mb_key(m));
        let extra = EXTRA.load(std::sync::atomic::Ordering::Relaxed);
        if extra != 0 {
            let long = !m.is_drop()
                && himawari_mailbox::board::MOVES[pos.piece_on(m.from()) as usize].long != 0;
            let hit = match extra {
                1 => true,
                2 => m.is_drop(),
                3 => capture,
                4 => !m.is_drop() && !capture,
                _ => long,
            };
            if hit {
                pos.do_move(m);
                pos.undo_move(m);
                t.extra += 1;
            }
        }
        pos.do_move(m);
        t.moves += 1;
        if depth > 0 {
            walk_mb(pos, depth - 1, stage, pseudo, t);
        }
        pos.undo_move(m);
    }
}

/// coreで同じ歩き方をし、王手でないノードの擬似合法手を配列盤の手へ直して積む。
fn collect_pseudo(pos: &mut CorePos, depth: u32, out: &mut Pseudo) {
    let mut list = CoreList::default();
    generate_legal(pos, true, &mut list);
    if !pos.in_check() {
        let mut pseudo = CoreList::default();
        generate(pos, CoreGen::NonEvasions, false, &mut pseudo);
        out.moves
            .extend(pseudo.as_slice().iter().map(|&m| to_mb(m)));
    }
    out.offs.push(out.moves.len() as u32);
    for &m in list.as_slice() {
        let capture = !m.is_drop() && !pos.piece_on(m.to()).is_empty();
        if depth == 0 && !capture {
            continue;
        }
        pos.do_move(m);
        if depth > 0 {
            collect_pseudo(pos, depth - 1, out);
        }
        pos.undo_move(m);
    }
}

fn time<F: FnMut(&mut Tally)>(repeat: u32, mut f: F) -> (u64, Tally) {
    let mut t = Tally::default();
    let start = Instant::now();
    for _ in 0..repeat {
        t.cursor = 0;
        f(&mut t);
    }
    (start.elapsed().as_nanos() as u64, std::hint::black_box(t))
}

/// ADR-0230の序中盤1スレッドの割合（1手詰め・進め戻し・SEE・王手判定・合法性判定・指し手生成）。
const W_MID: [f64; 6] = [7.9, 3.6, 4.4, 3.7, 2.0, 12.4];
/// 同じく終盤寄りの割合。
const W_LATE: [f64; 6] = [14.8, 3.9, 4.7, 2.6, 2.5, 14.4];

fn weighted(r: &[f64; 6], w: &[f64; 6]) -> f64 {
    let num: f64 = r.iter().zip(w).map(|(a, b)| a * b).sum();
    num / w.iter().sum::<f64>()
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
    let mut pseudo = Pseudo::default();
    pseudo.offs.push(0);
    for p in cores.iter_mut() {
        collect_pseudo(p, d, &mut pseudo);
    }
    let core_stage = |cores: &mut Vec<CorePos>, st: Stage| {
        time(r, |t| cores.iter_mut().for_each(|p| walk_core(p, d, st, t)))
    };
    let mb_stage = |mbs: &mut Vec<Position>, st: Stage| {
        time(r, |t| {
            mbs.iter_mut().for_each(|p| walk_mb(p, d, st, &pseudo, t))
        })
    };
    if let Some((st, side)) = cli.only {
        // 段を1つだけ、片側だけ走らせる。命令数（`/usr/bin/time -l`）を、walk段だけの実行との差で
        // 段ごとに出すための形で、空回しも同じ側のwalkで1回行う
        for run in 1..=cli.runs {
            let (ns, t) = match side {
                Side::Core => {
                    core_stage(&mut cores, Stage::Walk);
                    core_stage(&mut cores, st)
                }
                Side::Mb => {
                    mb_stage(&mut mbs, Stage::Walk);
                    mb_stage(&mut mbs, st)
                }
            };
            let name = ["walk", "gen", "mate", "see", "check", "pgen", "legal"]
                [STAGES.iter().position(|&x| x == st).unwrap_or(0)];
            let side = if side == Side::Core { "core" } else { "mb" };
            println!(
                "{{\"run\":{run},\"stage\":\"{name}\",\"side\":\"{side}\",\"ns\":{ns},\
                 \"moves\":{},\"nodes\":{},\"calls\":{},\"extra\":{},\"hash\":\"{:016x}\"}}",
                t.moves, t.nodes, t.calls, t.extra, t.hash
            );
        }
        return Ok(());
    }
    // 1段ぶん空回しして、表と分岐予測を温める
    core_stage(&mut cores, Stage::Walk);
    mb_stage(&mut mbs, Stage::Walk);

    let undo = if cfg!(feature = "copy-undo") {
        "copy"
    } else {
        "diff"
    };
    for run in 1..=cli.runs {
        let mut cn = [0u64; STAGES.len()];
        let mut mn = [0u64; STAGES.len()];
        let mut ct: Vec<Tally> = Vec::new();
        let mut mt: Vec<Tally> = Vec::new();
        for (i, &st) in STAGES.iter().enumerate() {
            let (n, t) = core_stage(&mut cores, st);
            cn[i] = n;
            ct.push(t);
            let (n, t) = mb_stage(&mut mbs, st);
            mn[i] = n;
            mt.push(t);
        }
        let per = |ns: f64, n: u64| if n == 0 { 0.0 } else { ns / n as f64 };
        let diff = |ns: &[u64; 7], a: usize, b: usize| ns[a] as f64 - ns[b] as f64;
        // 段の添字: 0 walk, 1 gen, 2 mate, 3 see, 4 check, 5 pgen, 6 legal
        let stats = |ns: &[u64; 7], t: &[Tally]| -> [f64; 7] {
            let gen_ns = per(diff(ns, 1, 0), t[1].calls);
            let move_ns = per(ns[0] as f64, t[0].moves);
            let du = per(ns[0] as f64 - gen_ns * t[0].nodes as f64, t[0].moves);
            [
                move_ns,
                gen_ns,
                du,
                per(diff(ns, 2, 0), t[2].calls),
                per(diff(ns, 3, 0), t[3].calls),
                per(diff(ns, 4, 0), t[4].calls),
                per(diff(ns, 6, 5), t[6].calls),
            ]
        };
        let c = stats(&cn, &ct);
        let m = stats(&mn, &mt);
        let ratio = |i: usize| m[i] / c[i];
        // 重みの順（1手詰め・進め戻し・SEE・王手判定・合法性判定・指し手生成）
        let rv = [ratio(3), ratio(2), ratio(4), ratio(5), ratio(6), ratio(1)];
        let same_stage: Vec<bool> = (0..STAGES.len())
            .map(|i| {
                ct[i].hash == mt[i].hash && ct[i].calls == mt[i].calls && ct[i].moves == mt[i].moves
            })
            .collect();
        let hex = |t: &[Tally]| {
            let mut h = Tally::default();
            for x in t {
                h.mix(x.hash);
                h.mix(x.calls);
                h.mix(x.moves);
            }
            format!("{:016x}", h.hash)
        };
        let calls = |t: &[Tally]| {
            format!(
                "[{}]",
                t.iter()
                    .map(|x| x.calls.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        println!(
            "{{\"run\":{run},\"undo\":\"{undo}\",\"positions\":{},\"depth\":{d},\"repeat\":{r},\
             \"moves\":{},\"nodes\":{},\
             \"core_move_ns\":{:.2},\"mb_move_ns\":{:.2},\"ratio_move\":{:.3},\
             \"core_gen_ns\":{:.2},\"mb_gen_ns\":{:.2},\"ratio_gen\":{:.3},\
             \"core_do_undo_ns\":{:.2},\"mb_do_undo_ns\":{:.2},\"ratio_do_undo\":{:.3},\
             \"core_mate_ns\":{:.2},\"mb_mate_ns\":{:.2},\"ratio_mate\":{:.3},\
             \"core_see_ns\":{:.2},\"mb_see_ns\":{:.2},\"ratio_see\":{:.3},\
             \"core_check_ns\":{:.2},\"mb_check_ns\":{:.2},\"ratio_check\":{:.3},\
             \"core_legal_ns\":{:.2},\"mb_legal_ns\":{:.2},\"ratio_legal\":{:.3},\
             \"R_mid\":{:.3},\"R_late\":{:.3},\"calls\":{},\
             \"check_core\":\"{}\",\"check_mb\":\"{}\",\"same\":{}}}",
            cores.len(),
            ct[0].moves,
            ct[0].nodes,
            c[0],
            m[0],
            ratio(0),
            c[1],
            m[1],
            ratio(1),
            c[2],
            m[2],
            ratio(2),
            c[3],
            m[3],
            ratio(3),
            c[4],
            m[4],
            ratio(4),
            c[5],
            m[5],
            ratio(5),
            c[6],
            m[6],
            ratio(6),
            weighted(&rv, &W_MID),
            weighted(&rv, &W_LATE),
            calls(&ct),
            hex(&ct),
            hex(&mt),
            same_stage.iter().all(|&b| b),
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
