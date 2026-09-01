//! 盲点ベンチマーク（ADR-0191）。
//!
//! floodgateの実戦で評価が崩壊した局面を集め、深い探索の正解ラベル付きの
//! 測定集合を作る。学習と検収が自己対局で閉じている穴を、実戦の相手が
//! 掘った局面で補う。
//!
//! 使い方:
//!   blindspot extract --dir data/raw/floodgate/2026 --out candidates.tsv
//!
//! 抽出は決定論で、同じ入力からは同じ出力が出る。実戦時の評価値
//! （CSAの `'**` コメント）だけを使い、エンジンは起動しない。

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use himawari_core::{Position, SFEN_STARTPOS};
use himawari_tools::csa::{self, CsaGame};
use himawari_tools::usi_engine::UsiEngine;
use himawari_tools::{OrBail, ensure_executable, eval_file, path_str, single_thread_options};

#[derive(Parser)]
#[command(about = "盲点ベンチマークの抽出・ラベル・測定（ADR-0191）")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 崩壊局面の候補をCSA群から抽出してTSVへ書く
    Extract {
        /// 棋譜の置き場
        #[arg(long, default_value = "data/raw/floodgate/2026")]
        dir: PathBuf,
        /// 出力TSV
        #[arg(long, default_value = "data/raw/blindspots/candidates.tsv")]
        out: PathBuf,
        /// 自分とみなす対局者名の部分一致
        #[arg(long, default_value = "Himawari")]
        player: String,
        /// 崩壊とみなす評価の落差[cp]
        #[arg(long, default_value_t = 300)]
        drop: i32,
        /// 崩壊前の評価の床[cp]。これ未満から始まる悪化は除く
        #[arg(long, default_value_t = 0)]
        floor: i32,
    },
    /// 候補を深い探索で再解析し、正解ラベルをTSVへ追記する
    Label {
        /// extractが書いた候補TSV
        #[arg(long, default_value = "data/raw/blindspots/candidates.tsv")]
        candidates: PathBuf,
        /// ラベルの出力TSV。既にある行のSFENは飛ばす（再開できる）
        #[arg(long, default_value = "data/raw/blindspots/labels.tsv")]
        out: PathBuf,
        /// 再解析に使うエンジン
        #[arg(long, default_value = "target/release/himawari")]
        engine: PathBuf,
        /// 評価関数。省くと環境変数EVAL_FILEを読む
        #[arg(long)]
        eval_file: Option<PathBuf>,
        /// 再解析のノード数（1スレッド固定で決定論）
        #[arg(long, default_value_t = 10_000_000)]
        nodes: u64,
        /// 1局面の制限時間[秒]
        #[arg(long, default_value_t = 300)]
        timeout: u64,
        /// 先頭のこの件数だけ処理する
        #[arg(long)]
        limit: Option<usize>,
    },
    /// 現行ネットの浅い評価（qsearch葉）と正解ラベルの乖離を測る
    Measure {
        /// labelが書いたラベルTSV
        #[arg(long, default_value = "data/raw/blindspots/labels.tsv")]
        labels: PathBuf,
        /// 評価関数。省くと環境変数EVAL_FILEを読む
        #[arg(long)]
        eval_file: Option<PathBuf>,
        /// ベンチに入れる下限。実戦評価と正解の勝率乖離がこれ未満の行は
        /// 「評価は正しく、水平線か相手の妙手」なので除く
        #[arg(long, default_value_t = 0.15)]
        gap_floor: f64,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("エラー: {e:#}");
            ExitCode::from(3)
        }
    }
}

fn run(cli: &Cli) -> Result<()> {
    match &cli.cmd {
        Cmd::Extract {
            dir,
            out,
            player,
            drop,
            floor,
        } => extract(dir, out, player, *drop, *floor),
        Cmd::Label {
            candidates,
            out,
            engine,
            eval_file: eval,
            nodes,
            timeout,
            limit,
        } => label(
            candidates,
            out,
            engine,
            eval.clone(),
            *nodes,
            *timeout,
            *limit,
        ),
        Cmd::Measure {
            labels,
            eval_file: eval,
            gap_floor,
        } => measure(labels, eval.clone(), *gap_floor),
    }
}

/// 勝率変換。学習の損失と同じ sigmoid(score/600)（crates/pyのSIGMOID_SCALE）。
fn winprob(cp: i32) -> f64 {
    1.0 / (1.0 + (-f64::from(cp.clamp(-20000, 20000)) / 600.0).exp())
}

/// 現行ネットのqsearch葉の評価と正解ラベルの勝率乖離を集計する。
///
/// 数字は世代ごとにADR-0191の測定節へ追記する。ゲートではなく、
/// 自己分布の外の性能を世代の推移として見る検出器である。
fn measure(labels: &Path, eval: Option<PathBuf>, gap_floor: f64) -> Result<()> {
    use himawari_core::Position;
    use himawari_engine::eval::Evaluator;
    use himawari_engine::movepick::Histories;
    use himawari_engine::search::{Shared, Worker};
    use himawari_engine::timeman::{Limits, TimeManager, TimeOptions};
    use std::sync::Arc;

    let eval_path = eval_file(eval)?;
    let mut f = std::fs::File::open(&eval_path)?;
    let (net, _lineage) =
        himawari_engine::nnue_io::load(&mut f).map_err(|e| anyhow::anyhow!("{e}"))?;
    let net = Arc::new(net);
    let shared = Arc::new(Shared::new(16));
    let limits = Limits::default();
    let start_pos = Position::from_sfen(SFEN_STARTPOS).expect("平手初期局面");
    let tm = TimeManager::new(
        &limits,
        start_pos.side_to_move(),
        start_pos.game_ply(),
        &TimeOptions::default(),
    );
    let mut worker = Worker::new(
        start_pos,
        Arc::clone(&shared),
        limits,
        tm,
        0,
        1,
        Evaluator::nnue(Arc::clone(&net)),
        Histories::default(),
    );

    let text = std::fs::read_to_string(labels)
        .with_context(|| format!("ラベルTSVを開けません: {}", labels.display()))?;
    let mut gaps: Vec<Gap> = Vec::new();
    let mut skipped = 0usize;
    for line in text.lines().skip(1) {
        let cols: Vec<&str> = line.split('\t').collect();
        let [sfen, ply, eval_game, deep_cp, ..] = cols[..] else {
            continue;
        };
        let (Ok(ply), Ok(eval_game), Ok(deep_cp)) = (
            ply.parse::<usize>(),
            eval_game.parse::<i32>(),
            deep_cp.parse::<i32>(),
        ) else {
            continue;
        };
        // 確定基準: 実戦時の評価が正解から乖離していた局面だけを測る
        if (winprob(eval_game) - winprob(deep_cp)).abs() < gap_floor {
            skipped += 1;
            continue;
        }
        let Ok(pos) = Position::from_sfen(sfen) else {
            skipped += 1;
            continue;
        };
        // クラス分けは元の局面で決める。walk_to_quietが進めた後の局面は
        // 手番も駒割も変わる
        let us = pos.side_to_move();
        let king_rank = pos.king(us).rank().relative(us).0;
        let opp_king_rank = pos.king(us.flip()).rank().relative(us.flip()).0;
        let sign = if us == himawari_core::Color::Black {
            1
        } else {
            -1
        };
        let material = pos.state().material * sign;
        worker.set_position(pos);
        let plies = worker.walk_to_quiet(16);
        let raw = worker.evaluator.evaluate(&worker.pos);
        let shallow = if plies % 2 == 1 { -raw } else { raw };
        let signed = winprob(shallow) - winprob(deep_cp);
        let gap = signed.abs();
        gaps.push(Gap {
            gap,
            signed,
            ply,
            eval_game,
            king_rank,
            opp_king_rank,
            material,
        });
    }
    if gaps.is_empty() {
        anyhow::bail!("測定対象がない（確定基準で全滅）");
    }

    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    let all: Vec<f64> = gaps.iter().map(|g| g.gap).collect();
    let mut sorted = all.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("評価関数: {}", eval_path.display());
    println!(
        "対象{}件（確定基準の床{gap_floor}、除外{skipped}件）",
        gaps.len()
    );
    println!(
        "浅い評価と正解の勝率乖離: 平均{:.3} 中央値{:.3} p90={:.3}",
        mean(&all),
        sorted[sorted.len() / 2],
        sorted[sorted.len() * 9 / 10]
    );
    // 符号つきで見ると、外し方の向きが分かる。正なら浅い評価が高すぎる
    let signed: Vec<f64> = gaps.iter().map(|g| g.signed).collect();
    let over = signed.iter().filter(|v| **v > 0.0).count();
    println!(
        "符号つきの乖離: 平均{:+.3}（過大{}件 / 過小{}件）",
        mean(&signed),
        over,
        signed.len() - over
    );
    for (name, lo, hi) in [
        ("〜60手", 0, 60),
        ("61〜100手", 61, 100),
        ("101手〜", 101, 9999),
    ] {
        let v: Vec<f64> = gaps
            .iter()
            .filter(|g| (lo..=hi).contains(&g.ply))
            .map(|g| g.gap)
            .collect();
        if !v.is_empty() {
            println!("  {name:>9}: 平均{:.3}（{}件）", mean(&v), v.len());
        }
    }
    for (name, lo, hi) in [
        ("互角〜+500", 0, 500),
        ("+501〜2000", 501, 2000),
        ("+2001〜", 2001, 99999),
    ] {
        let v: Vec<f64> = gaps
            .iter()
            .filter(|g| (lo..=hi).contains(&g.eval_game))
            .map(|g| g.gap)
            .collect();
        if !v.is_empty() {
            println!("  実戦評価{name:>10}: 平均{:.3}（{}件）", mean(&v), v.len());
        }
    }
    // 玉の段は入玉度そのもの。教師データの入玉が薄いなら、ここに乖離が
    // 集まるはずである（ADR-0190の被覆測定で入玉圏の質量は1.7%だった）
    for (name, lo, hi) in [
        ("敵陣（〜3段）", 0, 2),
        ("中段（4〜6段）", 3, 5),
        ("自陣（7段〜）", 6, 8),
    ] {
        let v: Vec<f64> = gaps
            .iter()
            .filter(|g| (lo..=hi).contains(&g.king_rank))
            .map(|g| g.gap)
            .collect();
        if !v.is_empty() {
            println!("  手番玉{name:>14}: 平均{:.3}（{}件）", mean(&v), v.len());
        }
    }
    let both_in: Vec<f64> = gaps
        .iter()
        .filter(|g| g.king_rank <= 2 && g.opp_king_rank <= 2)
        .map(|g| g.gap)
        .collect();
    if !both_in.is_empty() {
        println!(
            "  相互入玉          : 平均{:.3}（{}件）",
            mean(&both_in),
            both_in.len()
        );
    }
    for (name, keep) in [("駒得", true), ("駒損", false)] {
        let sel: Vec<&Gap> = gaps.iter().filter(|g| (g.material >= 0) == keep).collect();
        if !sel.is_empty() {
            let v: Vec<f64> = sel.iter().map(|g| g.gap).collect();
            let sv: Vec<f64> = sel.iter().map(|g| g.signed).collect();
            println!(
                "  手番側の{name}      : 平均{:.3} 符号つき{:+.3}（{}件）",
                mean(&v),
                mean(&sv),
                v.len()
            );
        }
    }
    Ok(())
}

/// 1局面の測定結果と、クラス分けの材料。
struct Gap {
    /// 浅い評価と正解ラベルの勝率乖離
    gap: f64,
    /// 符号つきの乖離。正なら浅い評価が正解より高い（過大評価）
    signed: f64,
    ply: usize,
    /// 実戦時の評価値（手番側から見たcp）
    eval_game: i32,
    /// 手番側の玉の相対段。0が敵陣の最奥で、2以下なら入玉圏にいる
    king_rank: u8,
    /// 相手玉の相対段。両方が2以下なら相互入玉になる
    opp_king_rank: u8,
    /// 手番側から見た駒割
    material: i32,
}

/// 候補を1スレッド・固定ノードで再解析し、正解ラベルを追記する。
///
/// 局面はSFENでなくUSI手順で渡し、千日手の判定を実戦と揃える。
/// 局面ごとに `usinewgame` でTTを消す（kifuの再解析と同じ決定論の要件）。
fn label(
    candidates: &Path,
    out: &Path,
    engine: &Path,
    eval: Option<PathBuf>,
    nodes: u64,
    timeout: u64,
    limit: Option<usize>,
) -> Result<()> {
    let eval = eval_file(eval)?;
    ensure_executable(engine)?;
    let text = std::fs::read_to_string(candidates)
        .with_context(|| format!("候補TSVを開けません: {}", candidates.display()))?;

    // 既にラベル済みのSFENは飛ばす（追記・再開）
    let mut done = BTreeSet::new();
    if out.is_file() {
        for line in std::fs::read_to_string(out)?.lines().skip(1) {
            if let Some(sfen) = line.split('\t').next() {
                done.insert(sfen.to_string());
            }
        }
    }
    let fresh = !out.is_file();
    let mut w = std::io::BufWriter::new(
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(out)?,
    );
    if fresh {
        writeln!(
            w,
            "sfen\tply\teval_game\tdeep_cp\tdeep_score\tbestmove\tfile"
        )?;
    }

    let mut options = single_thread_options(&eval);
    options.push(("USI_OwnBook".to_string(), "false".to_string()));
    let mut eng = UsiEngine::launch(path_str(engine)?, &options).or_bail()?;

    let (mut labeled, mut skipped) = (0usize, 0usize);
    for line in text.lines().skip(1) {
        let cols: Vec<&str> = line.split('\t').collect();
        let [sfen, file, ply, eval_before, _eval_after, moves] = cols[..] else {
            continue;
        };
        if done.contains(sfen) {
            skipped += 1;
            continue;
        }
        if limit.is_some_and(|l| labeled >= l) {
            break;
        }
        let position_cmd = format!("position startpos moves {moves}");
        eng.new_game().or_bail()?;
        let result = eng
            .think(
                &position_cmd,
                &format!("go nodes {nodes}"),
                Duration::from_secs(timeout),
            )
            .or_bail()?;
        let deep_cp = result.score_cp.unwrap_or(0);
        let deep_score = result.last_info.score.unwrap_or_else(|| "n/a".to_string());
        writeln!(
            w,
            "{sfen}\t{ply}\t{eval_before}\t{deep_cp}\t{deep_score}\t{}\t{file}",
            result.bestmove
        )?;
        w.flush()?;
        labeled += 1;
        if labeled % 20 == 0 {
            println!("{labeled}件ラベル済み（直近: 実戦{eval_before} → 深い再解析{deep_cp}）");
        }
    }
    eng.quit();
    println!(
        "ラベル{labeled}件を追記、既存{skipped}件を飛ばしました → {}",
        out.display()
    );
    Ok(())
}

/// 1件の崩壊候補。崩壊前の自分の手番の局面を指す。
struct Candidate {
    sfen: String,
    file: String,
    /// 崩壊前の自分の手番（1始まり）。
    ply: usize,
    eval_before: i32,
    eval_after: i32,
    /// 初期局面からこの局面までのUSI手順。千日手の履歴を保つ。
    moves: String,
}

fn extract(dir: &Path, out: &Path, player: &str, drop: i32, floor: i32) -> Result<()> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("開けません: {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "csa"))
        .collect();
    files.sort();

    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    let (mut games, mut skipped) = (0usize, 0usize);
    for path in &files {
        let text = std::fs::read_to_string(path)?;
        let game = match csa::parse(&text) {
            Ok(g) => g,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        games += 1;
        if let Err(_e) = scan_game(&game, path, player, drop, floor, &mut seen, &mut candidates) {
            skipped += 1;
        }
    }

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut w = std::io::BufWriter::new(std::fs::File::create(out)?);
    writeln!(w, "sfen\tfile\tply\teval_before\teval_after\tmoves")?;
    for c in &candidates {
        writeln!(
            w,
            "{}\t{}\t{}\t{}\t{}\t{}",
            c.sfen, c.file, c.ply, c.eval_before, c.eval_after, c.moves
        )?;
    }
    w.flush()?;
    println!(
        "対局{games}件（読めない棋譜{skipped}件）から候補{}件を{}へ書き出しました",
        candidates.len(),
        out.display()
    );
    Ok(())
}

/// 1局を走査し、自分の評価が次の自分の手番までにdrop以上落ちた対の
/// 崩壊前局面を候補へ足す。
fn scan_game(
    game: &CsaGame,
    path: &std::path::Path,
    player: &str,
    drop: i32,
    floor: i32,
    seen: &mut BTreeSet<String>,
    out: &mut Vec<Candidate>,
) -> Result<()> {
    let Some(me) = game.side_of(player) else {
        return Ok(());
    };
    // 自分の手番のうち評価値が付いている(手index, eval)の列
    let evals: Vec<(usize, i32)> = game
        .moves
        .iter()
        .enumerate()
        .filter(|(_, m)| m.color == me)
        .filter_map(|(i, m)| m.eval_cp.map(|e| (i, e)))
        .collect();

    // 崩壊対を先に決めてから、必要な局面だけ再生する
    let wanted: Vec<(usize, i32, i32)> = evals
        .windows(2)
        .filter(|w| w[0].1 >= floor && w[0].1 - w[1].1 >= drop)
        .map(|w| (w[0].0, w[0].1, w[1].1))
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }

    let mut pos = Position::from_sfen(SFEN_STARTPOS).expect("平手初期局面");
    let mut usi_moves: Vec<String> = Vec::with_capacity(game.moves.len());
    let mut iter = wanted.iter().peekable();
    for (i, m) in game.moves.iter().enumerate() {
        if let Some(&&(idx, before, after)) = iter.peek()
            && idx == i
        {
            let sfen = pos.to_sfen();
            if seen.insert(sfen.clone()) {
                out.push(Candidate {
                    sfen,
                    file: path
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    ply: i + 1,
                    eval_before: before,
                    eval_after: after,
                    moves: usi_moves.join(" "),
                });
            }
            iter.next();
            if iter.peek().is_none() {
                break;
            }
        }
        let mv = csa::resolve_move(&pos, m)
            .ok_or_else(|| anyhow::anyhow!("{}手目を解決できない: {}", i + 1, m.text))?;
        pos.do_move(mv);
        usi_moves.push(mv.to_usi());
    }
    Ok(())
}
