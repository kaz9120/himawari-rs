//! 動作環境の最適なスレッド数を、NPSと自己対局で決める（ADR-0203）。
//!
//! 使い方:
//!   himawari threadtune --eval <hmwr> [--hours 4] [--current 6] [--tc 10+0.1]
//!                       [--hash 256] [--max-threads N] [--warmup-secs 120]
//!                       [--pairs N] [--nps-only]
//!
//! 2段で絞る。まず最大スレッド数で温度を落ち着かせてから候補ごとの
//! 持続NPSを測り、伸びない候補を落とす。次に現在の設定を起点に隣の候補と
//! 自己対局し、信頼区間が0を跨がなければ勝った側へ進む。跨げば少ない
//! ほうを採る（熱と電力が少ない側に倒す）。
//!
//! 対局の相手は自分自身で、`himawari` を子プロセスとしてUSIで起動する。
//! 探索の経路には触れない。開始局面は埋め込んである。
//! 終了コード: 0=成功、2=引数エラー、3=実行時エラー（ADR-0122）。

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use himawari_core::Color;

use crate::game::{GameConfig, GameRecord, TimeControl, play_game};
use crate::usi_engine::UsiEngine;

/// 埋め込みの開始局面（start_sfens_ply24の先頭1,000局面）。
const OPENINGS: &str = include_str!("threadtune_openings.txt");

/// NPSを測る局面（bench と同じ4局面）。
const NPS_POSITIONS: [&str; 4] = [
    "startpos",
    "sfen l6nl/5+P1gk/2np1S3/p1p4Pp/3P2Sp1/1PPb2P1P/P5GS1/R8/LN4bKL w RGgsn5p 1",
    "sfen lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1 moves 7g7f 3c3d 2g2f 4c4d",
    "sfen 8l/1l+R2P3/p2pBG1pp/kps1p4/Nn1P2G2/P1P1P2PP/1PS6/1KSG3+r1/LN2+p3L w Sbgn3p 124",
];

/// NPSがこの比率以上伸びない候補は落とす。
const NPS_GAIN_MIN: f64 = 1.08;

struct Args {
    eval: String,
    hours: f64,
    current: Option<usize>,
    tc: String,
    hash: usize,
    max_threads: Option<usize>,
    warmup_secs: u64,
    /// 1組のペア数。指定すると --hours からの算出を使わない
    pairs: Option<usize>,
    nps_only: bool,
}

fn usage() -> String {
    "使い方: himawari threadtune --eval <hmwr> [--hours 4] [--current N] [--tc 10+0.1] \
     [--hash 256] [--max-threads N] [--warmup-secs 120] [--pairs N] [--nps-only]"
        .to_string()
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut a = Args {
        eval: String::new(),
        hours: 4.0,
        current: None,
        tc: "10+0.1".to_string(),
        hash: 256,
        max_threads: None,
        warmup_secs: 120,
        pairs: None,
        nps_only: false,
    };
    let mut i = 0;
    let value = |i: usize| -> Result<&String, String> {
        args.get(i + 1)
            .ok_or_else(|| format!("{} に値がありません\n{}", args[i], usage()))
    };
    let num = |i: usize| -> Result<usize, String> {
        value(i)?
            .parse()
            .map_err(|_| format!("{} は整数: {}", args[i], value(i).unwrap_or(&String::new())))
    };
    while i < args.len() {
        match args[i].as_str() {
            "--eval" => a.eval = value(i)?.clone(),
            "--hours" => {
                a.hours = value(i)?
                    .parse()
                    .map_err(|_| format!("--hours は実数: {}", args[i + 1]))?
            }
            "--current" => a.current = Some(num(i)?),
            "--tc" => a.tc = value(i)?.clone(),
            "--hash" => a.hash = num(i)?,
            "--max-threads" => a.max_threads = Some(num(i)?),
            "--warmup-secs" => a.warmup_secs = num(i)? as u64,
            "--pairs" => a.pairs = Some(num(i)?),
            "--nps-only" => {
                a.nps_only = true;
                i += 1;
                continue;
            }
            "-h" | "--help" => return Err(usage()),
            other => return Err(format!("不明な引数: {other}\n{}", usage())),
        }
        i += 2;
    }
    if a.eval.is_empty() {
        return Err(format!("--eval が要る\n{}", usage()));
    }
    Ok(a)
}

fn parse_tc(s: &str) -> Result<TimeControl, String> {
    let (b, inc) = s
        .split_once('+')
        .ok_or_else(|| format!("--tc は base+inc の形: {s}"))?;
    let b: f64 = b.parse().map_err(|_| format!("--tc の基本時間: {s}"))?;
    let inc: f64 = inc.parse().map_err(|_| format!("--tc の加算: {s}"))?;
    Ok(TimeControl::Fischer {
        base_ms: (b * 1000.0).round() as u64,
        inc_ms: (inc * 1000.0).round() as u64,
    })
}

/// 候補のスレッド数。1、2、4、6、8、…と偶数で上限まで。現在値は必ず含める。
fn candidates(max: usize, current: Option<usize>) -> Vec<usize> {
    let mut v = vec![1, 2];
    let mut t = 4;
    while t <= max {
        v.push(t);
        t += 2;
    }
    if let Some(c) = current {
        v.push(c);
    }
    v.retain(|&t| t >= 1 && t <= max);
    v.sort_unstable();
    v.dedup();
    v
}

fn launch(engine: &str, eval: &str, hash: usize, threads: usize) -> Result<UsiEngine, String> {
    let opts = vec![
        ("EvalFile".to_string(), eval.to_string()),
        ("USI_Hash".to_string(), hash.to_string()),
        ("Threads".to_string(), threads.to_string()),
    ];
    UsiEngine::launch(engine, &opts)
}

/// 1候補の持続NPS。4局面を各 `secs` 秒読ませ、合計ノード/合計時間。
fn measure_nps(eng: &mut UsiEngine, secs: u64) -> Result<f64, String> {
    let (mut nodes, mut ms) = (0u64, 0u64);
    for pos in NPS_POSITIONS {
        eng.new_game()?;
        let r = eng.think(
            &format!("position {pos}"),
            &format!("go movetime {}", secs * 1000),
            Duration::from_secs(secs + 30),
        )?;
        nodes += r.last_info.nodes.unwrap_or(0);
        ms += r.elapsed_ms.max(1);
    }
    Ok(nodes as f64 * 1000.0 / ms as f64)
}

fn elo_of(score: f64) -> f64 {
    let s = score.clamp(1e-9, 1.0 - 1e-9);
    -400.0 * (1.0 / s - 1.0).log10()
}

/// ペア得点の度数からEloと95%信頼区間を出す（selfplayと同じ近似）。
fn elo_estimate(pent: &[u64; 5]) -> (f64, f64, f64) {
    const VALUES: [f64; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];
    let c: Vec<f64> = pent.iter().map(|&x| (x as f64).max(1e-3)).collect();
    let n: f64 = c.iter().sum();
    let mean = c.iter().zip(VALUES).map(|(c, v)| c * v).sum::<f64>() / n;
    let var = c.iter().zip(VALUES).map(|(c, v)| c * v * v).sum::<f64>() / n - mean * mean;
    let se = (var.max(0.0) / n).sqrt();
    (
        elo_of(mean),
        elo_of(mean - 1.96 * se),
        elo_of(mean + 1.96 * se),
    )
}

fn score_of(rec: &GameRecord, side: Color) -> f64 {
    match rec.winner {
        None => 0.5,
        Some(c) if c == side => 1.0,
        Some(_) => 0.0,
    }
}

/// 対局の共通設定。
struct Arena<'a> {
    engine: &'a str,
    eval: &'a str,
    hash: usize,
    tc: TimeControl,
    pairs: usize,
    openings: &'a [&'a str],
}

impl Arena<'_> {
    fn game_config(&self) -> GameConfig {
        GameConfig {
            tc: match self.tc {
                TimeControl::Fischer { base_ms, inc_ms } => {
                    TimeControl::Fischer { base_ms, inc_ms }
                }
                TimeControl::Nodes(n) => TimeControl::Nodes(n),
            },
            odds: [1.0, 1.0],
            max_moves: 320,
            adjudicate: Some((2000, 8)),
        }
    }

    /// 候補 `cand` を `base` に当てる。戻り値は cand から見た (Elo, 下限, 上限)。
    fn play_match(&self, base: usize, cand: usize) -> Result<(f64, f64, f64), String> {
        let mut a = launch(self.engine, self.eval, self.hash, cand)?;
        let mut b = launch(self.engine, self.eval, self.hash, base)?;
        let cfg = self.game_config();
        let mut pent = [0u64; 5];
        let start = Instant::now();
        for i in 0..self.pairs {
            let opening = self.openings[i % self.openings.len()];
            let g1 = play_game(&mut a, &mut b, opening, &cfg, [false, false])?;
            let g2 = play_game(&mut b, &mut a, opening, &cfg, [false, false])?;
            let s = score_of(&g1, Color::Black) + score_of(&g2, Color::White);
            pent[(s * 2.0).round() as usize] += 1;
            if (i + 1) % 10 == 0 || i + 1 == self.pairs {
                let (e, lo, hi) = elo_estimate(&pent);
                println!(
                    "  {cand}スレッド 対 {base}スレッド: {}ペア Elo {e:+.1} [{lo:+.1}, {hi:+.1}] {:.0}分",
                    i + 1,
                    start.elapsed().as_secs_f64() / 60.0
                );
            }
        }
        a.quit();
        b.quit();
        Ok(elo_estimate(&pent))
    }
}

/// `himawari threadtune ...` の本体。戻り値は終了コード。
pub fn main(args: &[String]) -> u8 {
    let a = match parse_args(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    match run(&a) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("エラー: {e}");
            3
        }
    }
}

fn run(a: &Args) -> Result<(), String> {
    let engine = std::env::current_exe()
        .map_err(|e| format!("自分の場所が分からない: {e}"))?
        .to_string_lossy()
        .into_owned();
    if !std::path::Path::new(&a.eval).is_file() {
        return Err(format!("評価関数がない: {}", a.eval));
    }
    let logical = std::thread::available_parallelism().map_or(1, |n| n.get());
    let max = a.max_threads.unwrap_or(logical).max(1);
    let tc = parse_tc(&a.tc)?;
    let openings: Vec<&str> = OPENINGS.lines().filter(|l| !l.trim().is_empty()).collect();

    println!("エンジン: {engine}");
    println!("論理コア: {logical}　候補の上限: {max}");
    println!("注意: 放流と同じ電源モード（最適なパフォーマンス）とAC接続で測る。");
    println!("      混成コアの機械では、電源の設定のほうがスレッド数より効くことがある。");
    println!();

    // 1. 持続NPS。最大スレッド数で暖機してから、往復で測って温度の漂いを均す
    let cands = candidates(max, a.current);
    println!(
        "=== 1. 持続クロックでのNPS（{}秒の暖機のあと、候補を往復で測る） ===",
        a.warmup_secs
    );
    {
        let mut eng = launch(&engine, &a.eval, a.hash, max)?;
        let start = Instant::now();
        while start.elapsed().as_secs() < a.warmup_secs {
            measure_nps(&mut eng, 5)?;
        }
        eng.quit();
    }
    let order: Vec<usize> = cands
        .iter()
        .copied()
        .chain(cands.iter().rev().copied())
        .collect();
    let mut sums: BTreeMap<usize, (f64, u32)> = BTreeMap::new();
    for &t in &order {
        let mut eng = launch(&engine, &a.eval, a.hash, t)?;
        let v = measure_nps(&mut eng, 3)?;
        eng.quit();
        let e = sums.entry(t).or_insert((0.0, 0));
        e.0 += v;
        e.1 += 1;
    }
    let nps: Vec<(usize, f64)> = sums
        .iter()
        .map(|(&t, &(sum, n))| (t, sum / f64::from(n)))
        .collect();
    let base_nps = nps[0].1.max(1.0);
    println!("{:>6} {:>12} {:>8}", "スレッド", "NPS", "1との比");
    for (t, v) in &nps {
        println!("{t:>8} {v:>12.0} {:>8.2}", v / base_nps);
    }

    // 伸びない候補を落とす。現在値は残す
    let mut kept: Vec<usize> = Vec::new();
    let mut last = 0.0f64;
    for &(t, v) in &nps {
        if kept.is_empty() || v >= last * NPS_GAIN_MIN || Some(t) == a.current {
            kept.push(t);
            last = last.max(v);
        }
    }
    println!(
        "候補（NPSが{:.0}%以上伸びるもの）: {kept:?}",
        (NPS_GAIN_MIN - 1.0) * 100.0
    );
    println!();
    if a.nps_only || kept.len() < 2 {
        println!("推奨: Threads={}", kept.last().copied().unwrap_or(1));
        return Ok(());
    }

    // 2. 自己対局で登る。起点から上へ、勝てなければ下へ、最大2組
    let secs_per_game = match tc {
        TimeControl::Fischer { base_ms, inc_ms } => {
            2.0 * (base_ms + inc_ms * 60) as f64 / 1000.0 + 2.0
        }
        TimeControl::Nodes(_) => 20.0,
    };
    let total_games = (a.hours * 3600.0 / secs_per_game).floor() as usize;
    let matches_max = 2usize;
    let pairs = a.pairs.unwrap_or((total_games / matches_max / 2).max(20));
    println!(
        "=== 2. 自己対局（{}、同時1局、1組{pairs}ペア、予算{:.1}時間） ===",
        a.tc, a.hours
    );
    let ar = Arena {
        engine: &engine,
        eval: &a.eval,
        hash: a.hash,
        tc,
        pairs,
        openings: &openings,
    };
    let start_idx = match a.current {
        Some(c) => kept.iter().position(|&t| t == c).unwrap_or(kept.len() - 1),
        None => kept.len() - 1,
    };
    let mut best = kept[start_idx];
    let mut results: Vec<(usize, usize, f64, f64, f64)> = Vec::new();
    let mut i = start_idx;
    while i + 1 < kept.len() && results.len() < matches_max {
        let (e, lo, hi) = ar.play_match(kept[i], kept[i + 1])?;
        results.push((kept[i + 1], kept[i], e, lo, hi));
        if lo > 0.0 {
            best = kept[i + 1];
            i += 1;
        } else {
            break;
        }
    }
    if best == kept[start_idx] && start_idx > 0 && results.len() < matches_max {
        let (e, lo, hi) = ar.play_match(kept[start_idx], kept[start_idx - 1])?;
        results.push((kept[start_idx - 1], kept[start_idx], e, lo, hi));
        // 少ないほうは、負けていなければ採る
        if hi >= 0.0 {
            best = kept[start_idx - 1];
        }
    }
    println!();
    println!("=== 結果 ===");
    for (c, b, e, lo, hi) in &results {
        println!("{c}スレッド 対 {b}スレッド: Elo {e:+.1} [{lo:+.1}, {hi:+.1}]");
    }
    println!("信頼区間が0を跨ぐ組は区別できず、少ないほうを採った。");
    println!("推奨: Threads={best}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_include_current_and_cap() {
        assert_eq!(candidates(12, Some(6)), vec![1, 2, 4, 6, 8, 10, 12]);
        assert_eq!(candidates(3, Some(3)), vec![1, 2, 3]);
        assert_eq!(candidates(1, None), vec![1]);
    }

    #[test]
    fn args_require_eval() {
        assert!(parse_args(&[]).is_err());
        let a = parse_args(&[
            "--eval".into(),
            "x.hmwr".into(),
            "--current".into(),
            "6".into(),
        ])
        .expect("引数");
        assert_eq!(a.current, Some(6));
        assert_eq!(a.hours, 4.0);
    }
}
