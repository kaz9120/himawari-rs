//! 総当たりリーグ戦で複数エンジンの相対Eloを測る（ADR-0128）。
//!
//! `selfplay` は2本の比較に特化していて、H1採択の可否だけを返す。
//! 候補が3つ以上あるとき、どれがどれだけ強いかは分からない。
//! ここでは固定局数で総当たりし、勝敗表からレーティングを推定する。
//!
//! 使い方:
//!   league <名前>=<バイナリ>[:<評価ファイル>] ... [--pairs N]
//!          [--tc 10+0.1 | --nodes N] [--openings <file>] [--concurrency N]
//!          [--hash MB] [--adjudicate CP,PLIES] [--max-moves N]
//!          [--anchor <名前>] [--out <path>]
//!
//! 1カード（参加者の組）につき `--pairs` 回の先後入れ替えペアを消化する。
//! 対局数は 参加者数×(参加者数-1)/2 × pairs × 2 になる。
//!
//! `--nodes` を使うと1手あたりのノード数で戦う。速い構成の得が消えるので、
//! 評価関数の質だけを比べられる（ADR-0127）。

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use clap::Parser;

use himawari_core::Color;
use himawari_tools::game::{GameConfig, GameRecord, TimeControl, play_game};
use himawari_tools::stop_file::StopFile;
use himawari_tools::usi_engine::UsiEngine;
use himawari_tools::{ensure_executable, exit, path_str};

#[derive(Parser)]
#[command(
    about = "総当たりリーグ戦で相対Eloを測る（ADR-0128）",
    long_about = "総当たりリーグ戦で相対Eloを測る（ADR-0128）。

参加者は `名前=バイナリ[:評価ファイル]` で並べる。評価ファイルを省くと
環境変数 EVAL_FILE を使う。ネットワーク構成ごとにバイナリと評価ファイルが
対になる比較（ADR-0127）を想定している。

出力はADRへ転記できるmarkdown表。Eloは総当たりの勝敗表から最尤で解く。"
)]
struct Cli {
    /// 参加者。`名前=バイナリ[:評価ファイル]` を2つ以上並べる
    #[arg(required = true, value_name = "名前=バイナリ[:評価ファイル]")]
    participants: Vec<String>,

    /// 1カードあたりの先後入れ替えペア数（対局数はこの2倍）
    #[arg(long, default_value_t = 25, value_parser = clap::value_parser!(u32).range(1..))]
    pairs: u32,

    /// 持ち時間 `<秒>+<増分秒>`
    #[arg(long, default_value = "10+0.1")]
    tc: String,

    /// 持ち時間の代わりに1手あたりのノード数で戦う。速い構成の得が
    /// 消えるので、評価関数の質だけを比べられる（ADR-0127）
    #[arg(long, value_name = "ノード数")]
    nodes: Option<u64>,

    /// 開始局面集（1行1SFEN、#はコメント）。省略時は平手初期局面
    #[arg(long, value_name = "パス")]
    openings: Option<PathBuf>,

    /// 同時に進める対局数
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u16).range(1..))]
    concurrency: u16,

    /// 置換表のサイズ（MB）
    #[arg(long, default_value_t = 128)]
    hash: u32,

    /// 手数上限。到達で引き分け
    #[arg(long, default_value_t = 320)]
    max_moves: usize,

    /// スコアによる早期終局 `<cp>,<連続ply>`
    #[arg(long, value_name = "CP,PLIES")]
    adjudicate: Option<String>,

    /// Eloの基準にする参加者。この参加者を0にそろえる。省略時は平均が0
    #[arg(long, value_name = "名前")]
    anchor: Option<String>,

    /// 棋譜の書き出し先（1局1行のJSON）
    #[arg(long, default_value = "league.jsonl", value_name = "パス")]
    out: PathBuf,
}

/// 参加者1人。バイナリと評価ファイルの対で1つの構成を表す。
struct Player {
    name: String,
    bin: PathBuf,
    eval: PathBuf,
}

/// 対戦カード1つぶんの結果。`wins[i]` は先に書いたほうから見た勝ち数。
#[derive(Clone, Copy, Default)]
struct Score {
    win: u32,
    draw: u32,
    loss: u32,
}

impl Score {
    /// 勝ち1・引き分け0.5で数えた得点。
    fn points(self) -> f64 {
        f64::from(self.win) + 0.5 * f64::from(self.draw)
    }

    fn games(self) -> u32 {
        self.win + self.draw + self.loss
    }
}

fn parse_participants(specs: &[String]) -> Result<Vec<Player>> {
    let mut out = Vec::with_capacity(specs.len());
    for spec in specs {
        let Some((name, rest)) = spec.split_once('=') else {
            bail!("参加者は `名前=バイナリ[:評価ファイル]` で書く: {spec}");
        };
        let (bin, eval) = match rest.split_once(':') {
            Some((b, e)) => (PathBuf::from(b), himawari_tools::eval_file(Some(e.into()))?),
            None => (PathBuf::from(rest), himawari_tools::eval_file(None)?),
        };
        ensure_executable(&bin)?;
        if out.iter().any(|p: &Player| p.name == name) {
            bail!("参加者の名前が重複している: {name}");
        }
        out.push(Player {
            name: name.to_string(),
            bin,
            eval,
        });
    }
    if out.len() < 2 {
        bail!("参加者が2人以上要る");
    }
    Ok(out)
}

fn parse_tc(s: &str) -> Result<TimeControl> {
    let Some((base, inc)) = s.split_once('+') else {
        bail!("持ち時間は `<秒>+<増分秒>` で書く: {s}");
    };
    let to_ms = |v: &str| -> Result<u64> {
        let secs: f64 = v
            .parse()
            .map_err(|_| anyhow::anyhow!("持ち時間が数値でない: {v}"))?;
        Ok((secs * 1000.0).round() as u64)
    };
    Ok(TimeControl::Fischer {
        base_ms: to_ms(base)?,
        inc_ms: to_ms(inc)?,
    })
}

fn parse_adjudicate(s: &Option<String>) -> Result<Option<(i32, u32)>> {
    let Some(s) = s else { return Ok(None) };
    let Some((cp, plies)) = s.split_once(',') else {
        bail!("--adjudicate は `<cp>,<連続ply>` で書く: {s}");
    };
    Ok(Some((
        cp.parse()
            .map_err(|_| anyhow::anyhow!("cpが整数でない: {cp}"))?,
        plies
            .parse()
            .map_err(|_| anyhow::anyhow!("plyが整数でない: {plies}"))?,
    )))
}

fn load_openings(path: &Option<PathBuf>) -> Result<Vec<String>> {
    let Some(path) = path else {
        return Ok(vec![himawari_core::SFEN_STARTPOS.to_string()]);
    };
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("開始局面集を読めない {}: {e}", path.display()))?;
    // 行頭の「sfen 」は配布ファイルで一般的なので剥がして受け入れる
    let v: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.strip_prefix("sfen ").unwrap_or(l).to_string())
        .collect();
    if v.is_empty() {
        bail!("開始局面集が空: {}", path.display());
    }
    Ok(v)
}

/// 勝敗表からEloを最尤で解く（ADR-0128）。
///
/// 参加者iの勝率をロジスティック関数 `1/(1+10^((r_j-r_i)/400))` で表し、
/// 各参加者の期待得点が実得点に合うところまで反復する。総当たりなので、
/// 誰と当たったかを織り込める。単純な勝率の並べ替えではこれができない。
fn solve_elo(n: usize, table: &[Vec<Score>]) -> Vec<f64> {
    /// 反復を打ち切る更新量（Elo）。
    const EPSILON: f64 = 1e-4;
    const MAX_ITERATIONS: usize = 10_000;
    /// 1回の更新でレーティングを動かす幅の上限。振動を防ぐ
    const MAX_STEP: f64 = 50.0;

    let mut rating = vec![0.0f64; n];
    for _ in 0..MAX_ITERATIONS {
        let mut moved: f64 = 0.0;
        for i in 0..n {
            let mut actual = 0.0;
            let mut expected = 0.0;
            let mut derivative = 0.0;
            for (j, s) in table[i].iter().enumerate() {
                if i == j || s.games() == 0 {
                    continue;
                }
                let games = f64::from(s.games());
                actual += s.points();
                let p = 1.0 / (1.0 + 10f64.powf((rating[j] - rating[i]) / 400.0));
                expected += games * p;
                // dP/dr = ln(10)/400 * p * (1-p)
                derivative += games * p * (1.0 - p) * std::f64::consts::LN_10 / 400.0;
            }
            if derivative <= 0.0 {
                continue;
            }
            let step = ((actual - expected) / derivative).clamp(-MAX_STEP, MAX_STEP);
            rating[i] += step;
            moved = moved.max(step.abs());
        }
        if moved < EPSILON {
            break;
        }
    }
    rating
}

/// 標準誤差の目安（Elo）。総当たりの相関は無視し、対戦数だけから出す。
fn elo_stderr(i: usize, table: &[Vec<Score>], rating: &[f64]) -> f64 {
    let mut information = 0.0;
    for (j, s) in table[i].iter().enumerate() {
        if i == j || s.games() == 0 {
            continue;
        }
        let games = f64::from(s.games());
        let p = 1.0 / (1.0 + 10f64.powf((rating[j] - rating[i]) / 400.0));
        let d = std::f64::consts::LN_10 / 400.0;
        information += games * p * (1.0 - p) * d * d;
    }
    if information <= 0.0 {
        return f64::INFINITY;
    }
    information.sqrt().recip()
}

/// 1カードぶんの対局を回す。戻り値は先手側から見た通算成績。
fn play_card(
    cli: &Cli,
    a: &Player,
    b: &Player,
    openings: &[String],
    game_cfg: &GameConfig,
    out: &Mutex<Box<dyn Write + Send>>,
    stop: &AtomicBool,
) -> Result<Score, String> {
    let opts = |eval: &std::path::Path| {
        vec![
            ("USI_Hash".to_string(), cli.hash.to_string()),
            ("Threads".to_string(), "1".to_string()),
            ("EvalFile".to_string(), eval.display().to_string()),
        ]
    };
    let mut ea = UsiEngine::launch(path_str(&a.bin).map_err(|e| e.to_string())?, &opts(&a.eval))?;
    let mut eb = UsiEngine::launch(path_str(&b.bin).map_err(|e| e.to_string())?, &opts(&b.eval))?;

    let mut score = Score::default();
    for pair in 0..cli.pairs {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let opening = &openings[(pair as usize) % openings.len()];
        // 同じ開始局面を先後入れ替えて2局。先手番の有利を打ち消す
        let g1 = play_game(&mut ea, &mut eb, opening, game_cfg, [false, false])?;
        let g2 = play_game(&mut eb, &mut ea, opening, game_cfg, [false, false])?;
        for (game, a_side) in [(&g1, Color::Black), (&g2, Color::White)] {
            match game.winner {
                None => score.draw += 1,
                Some(w) if w == a_side => score.win += 1,
                Some(_) => score.loss += 1,
            }
        }
        let mut w = out.lock().expect("out lock");
        for (game, a_side) in [(&g1, Color::Black), (&g2, Color::White)] {
            let _ = writeln!(
                w,
                "{}",
                jsonl_line(&a.name, &b.name, pair, opening, a_side, game)
            );
        }
    }
    ea.quit();
    eb.quit();
    Ok(score)
}

fn jsonl_line(a: &str, b: &str, pair: u32, opening: &str, a_side: Color, g: &GameRecord) -> String {
    let winner = match g.winner {
        None => "draw".to_string(),
        Some(Color::Black) => "black".to_string(),
        Some(Color::White) => "white".to_string(),
    };
    format!(
        r#"{{"a":"{a}","b":"{b}","pair":{pair},"a_side":"{}","opening":"{opening}","winner":"{winner}","reason":"{}","moves":{}}}"#,
        if a_side == Color::Black {
            "black"
        } else {
            "white"
        },
        g.reason,
        g.moves.len()
    )
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("エラー: {e:#}");
            ExitCode::from(exit::RUNTIME)
        }
    }
}

fn run(cli: &Cli) -> Result<()> {
    let players = parse_participants(&cli.participants)?;
    let openings = load_openings(&cli.openings)?;
    let game_cfg = GameConfig {
        // ノード数を指定したら持ち時間を使わない。速度差を消して測る
        tc: match cli.nodes {
            Some(n) => TimeControl::Nodes(n),
            None => parse_tc(&cli.tc)?,
        },
        max_moves: cli.max_moves,
        adjudicate: parse_adjudicate(&cli.adjudicate)?,
    };
    let n = players.len();
    // 総当たりのカード。i<j の組をすべて作る
    let cards: Vec<(usize, usize)> = (0..n)
        .flat_map(|i| (i + 1..n).map(move |j| (i, j)))
        .collect();

    let condition = match cli.nodes {
        Some(n) => format!("{n}ノード/手"),
        None => cli.tc.clone(),
    };
    println!(
        "=== リーグ戦: {n}人、{}カード、1カード{}ペア（{}局）、{condition} ===",
        cards.len(),
        cli.pairs,
        cards.len() * cli.pairs as usize * 2,
    );
    for p in &players {
        println!(
            "  {:<16} {} / {}",
            p.name,
            p.bin.display(),
            p.eval.display()
        );
    }
    println!();

    let file = std::fs::File::create(&cli.out)
        .map_err(|e| anyhow::anyhow!("棋譜を作れない {}: {e}", cli.out.display()))?;
    let out: Arc<Mutex<Box<dyn Write + Send>>> =
        Arc::new(Mutex::new(Box::new(std::io::BufWriter::new(file))));
    let table = Arc::new(Mutex::new(vec![vec![Score::default(); n]; n]));
    let next = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(Mutex::new(Vec::<String>::new()));
    // 長い対局を外から止める（ADR-0123）
    let stop_file = StopFile::beside(&cli.out);

    std::thread::scope(|scope| {
        for _ in 0..usize::from(cli.concurrency).min(cards.len()) {
            scope.spawn(|| {
                loop {
                    if stop.load(Ordering::Relaxed) || stop_file.requested() {
                        stop.store(true, Ordering::Relaxed);
                        break;
                    }
                    let idx = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&(i, j)) = cards.get(idx) else {
                        break;
                    };
                    match play_card(
                        cli,
                        &players[i],
                        &players[j],
                        &openings,
                        &game_cfg,
                        &out,
                        &stop,
                    ) {
                        Ok(s) => {
                            let mut t = table.lock().expect("table lock");
                            t[i][j] = s;
                            t[j][i] = Score {
                                win: s.loss,
                                draw: s.draw,
                                loss: s.win,
                            };
                            println!(
                                "  {:<16} vs {:<16} +{} ={} -{}",
                                players[i].name, players[j].name, s.win, s.draw, s.loss
                            );
                        }
                        Err(e) => {
                            failed
                                .lock()
                                .expect("failed lock")
                                .push(format!("{} vs {}: {e}", players[i].name, players[j].name));
                            stop.store(true, Ordering::Relaxed);
                        }
                    }
                }
            });
        }
    });

    let failed = failed.lock().expect("failed lock");
    if !failed.is_empty() {
        for f in failed.iter() {
            eprintln!("対局に失敗: {f}");
        }
        bail!("{}カードが失敗した", failed.len());
    }
    if stop_file.requested() {
        println!("\n停止ファイルを見つけたので打ち切った（ADR-0123）");
    }

    report(cli, &players, &table.lock().expect("table lock"))
}

fn report(cli: &Cli, players: &[Player], table: &[Vec<Score>]) -> Result<()> {
    let n = players.len();
    let mut rating = solve_elo(n, table);

    // 基準を決める。指定がなければ全体の平均を0にする
    let offset = match &cli.anchor {
        Some(name) => {
            let idx = players
                .iter()
                .position(|p| &p.name == name)
                .ok_or_else(|| anyhow::anyhow!("--anchor の参加者がいない: {name}"))?;
            rating[idx]
        }
        None => rating.iter().sum::<f64>() / n as f64,
    };
    for r in &mut rating {
        *r -= offset;
    }

    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| rating[b].partial_cmp(&rating[a]).expect("NaN"));

    println!();
    println!("| 参加者 | Elo | ±2SE | 得点 | 対局 |");
    println!("|---|---|---|---|---|");
    for &i in &order {
        let (points, games) = (0..n).filter(|&j| j != i).fold((0.0, 0u32), |(p, g), j| {
            (p + table[i][j].points(), g + table[i][j].games())
        });
        println!(
            "| {} | {:+.1} | {:.1} | {:.1}/{} | {} |",
            players[i].name,
            rating[i],
            2.0 * elo_stderr(i, table, &rating),
            points,
            games,
            games
        );
    }

    println!();
    println!("勝敗表（行から見た +勝 =分 -負）");
    print!("| |");
    for &j in &order {
        print!(" {} |", players[j].name);
    }
    println!();
    print!("|---|");
    for _ in &order {
        print!("---|");
    }
    println!();
    for &i in &order {
        print!("| {} |", players[i].name);
        for &j in &order {
            if i == j {
                print!(" — |");
            } else {
                let s = table[i][j];
                print!(" +{} ={} -{} |", s.win, s.draw, s.loss);
            }
        }
        println!();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(win: u32, draw: u32, loss: u32) -> Score {
        Score { win, draw, loss }
    }

    /// 与えたEloどおりの勝率を作ると、そのEloが復元できる。
    #[test]
    fn solve_elo_recovers_known_ratings() {
        let truth = [0.0, 100.0, -100.0];
        let n = truth.len();
        let games = 100_000.0;
        let mut table = vec![vec![Score::default(); n]; n];
        for (i, row) in table.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                if i == j {
                    continue;
                }
                let p = 1.0 / (1.0 + 10f64.powf((truth[j] - truth[i]) / 400.0));
                let win = (games * p).round() as u32;
                *cell = score(win, 0, games as u32 - win);
            }
        }
        let rating = solve_elo(n, &table);
        let base = rating[0];
        for (i, t) in truth.iter().enumerate() {
            assert!(
                (rating[i] - base - t).abs() < 2.0,
                "参加者{i}: 推定{:.1} 真値{t}",
                rating[i] - base
            );
        }
    }

    /// 引き分けだけなら全員同じレーティングになる。
    #[test]
    fn all_draws_give_equal_ratings() {
        let n = 4;
        let mut table = vec![vec![Score::default(); n]; n];
        for (i, row) in table.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                if i != j {
                    *cell = score(0, 50, 0);
                }
            }
        }
        let rating = solve_elo(n, &table);
        for r in &rating {
            assert!((r - rating[0]).abs() < 1e-3, "{rating:?}");
        }
    }

    /// 参加者の書き方が違えばエラーにする。
    #[test]
    fn participant_spec_is_validated() {
        assert!(parse_participants(&["バイナリだけ".to_string()]).is_err());
        assert!(
            parse_participants(&["a=/bin/ls:/bin/ls".to_string()]).is_err(),
            "1人では足りない"
        );
        let dup = [
            "a=/bin/ls:/bin/ls".to_string(),
            "a=/bin/cat:/bin/ls".to_string(),
        ];
        assert!(parse_participants(&dup).is_err(), "名前が重複している");
        let ok = [
            "a=/bin/ls:/bin/ls".to_string(),
            "b=/bin/cat:/bin/ls".to_string(),
        ];
        assert_eq!(parse_participants(&ok).unwrap().len(), 2);
    }

    /// 開始局面集は行頭の「sfen 」を剥がして読む。剥がさないと
    /// `Position::from_sfen` が段の数を数え違える。
    #[test]
    fn openings_drop_the_sfen_prefix() {
        let dir = std::env::temp_dir().join("himawari-league-openings");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("openings.txt");
        std::fs::write(
            &path,
            "# コメント\nsfen lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1\n\nlnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL w - 2\n",
        )
        .unwrap();
        let v = load_openings(&Some(path)).unwrap();
        assert_eq!(v.len(), 2, "コメントと空行を落とす");
        for sfen in &v {
            assert!(!sfen.starts_with("sfen "), "前置きが残っている: {sfen}");
            himawari_core::Position::from_sfen(sfen).expect("SFENとして読めること");
        }
    }

    #[test]
    fn tc_and_adjudicate_are_parsed() {
        let TimeControl::Fischer { base_ms, inc_ms } = parse_tc("10+0.1").unwrap() else {
            panic!("Fischerになるはず");
        };
        assert_eq!((base_ms, inc_ms), (10_000, 100));
        assert!(parse_tc("10").is_err());
        assert_eq!(
            parse_adjudicate(&Some("2000,8".to_string())).unwrap(),
            Some((2000, 8))
        );
        assert!(parse_adjudicate(&Some("2000".to_string())).is_err());
    }
}
