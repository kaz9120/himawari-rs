//! 棋譜の再解析レポート（ADR-0152）。
//!
//! CSA棋譜を1局ずつ再生し、疑わしい局面を列挙したmarkdownを出す。
//! 検査するのは3つ。大悪手（自分の手の前後で評価が落ちる）、詰み見逃し
//! （終盤で詰みがあるのに指していない）、時間の使い方である。
//!
//! **改善案そのものは書かない。** 疑わしい局面のSFENと根拠を並べるまでを
//! 受け持ち、そこから何を直すかはレポートを読んだ人が決める（ADR-0152）。
//!
//! 再解析は1スレッド・固定ノード数で行い、局面ごとにTTを消す。同じ入力
//! （棋譜・エンジン・評価関数・ノード数）からは同じレポートが出る。
//! レポートに日時や所要時間を書かないのはこのためである。
//!
//! 使い方:
//!   kifu <エンジン> <CSAファイルまたはディレクトリ>... [--nodes 1000000]
//!        [--player Himawari] [--blunder-cp 300] [--mate-window 10]
//!        [--out <path>] [--eval-file <hmwr>]

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::Parser;

use himawari_core::{Color, MoveList, Position, SFEN_STARTPOS, generate_legal};
use himawari_tools::csa::{self, CsaGame};
use himawari_tools::usi_engine::UsiEngine;
use himawari_tools::{
    OrBail, basename, ensure_executable, eval_file, exit, path_str, single_thread_options,
    usage_error,
};

/// 詰みスコアの下限。`usi_engine::parse_info` は `mate n` を `30000 - n` へ
/// 写す。nはMAX_PLY（128）を超えないので、29000で切れば詰みだけが残る。
const MATE_MIN: i32 = 29_000;

/// 詰み上がりの局面に入れる値。手番側は0手で詰まされているので、同じ写像
/// （`mate -0` → -30000）に合わせる。ここを -29000 のような丸めた値にすると、
/// 詰ました手が「mate 1（+29999）から+29000へ落ちた」と読めてしまい、
/// 大悪手として挙がる。
const MATED_SCORE: i32 = -30_000;

#[derive(Parser)]
#[command(
    about = "棋譜を再解析して疑わしい局面を列挙する（ADR-0152）",
    long_about = "棋譜を再解析して疑わしい局面を列挙する（ADR-0152）。

CSAファイルまたはディレクトリを受け取り、1局ずつ再生して
大悪手・詰み見逃し・時間の使い方をmarkdownで報告する。

探索は1スレッド・固定ノード数で、局面ごとに置換表を消す。
同じ入力からは同じレポートが出る。評価関数は EVAL_FILE か
--eval-file で渡す。"
)]
struct Cli {
    /// エンジンのバイナリ
    #[arg(value_name = "engine")]
    engine: PathBuf,

    /// CSAファイル、またはCSAを含むディレクトリ
    #[arg(value_name = "path", required = true)]
    paths: Vec<PathBuf>,

    /// 1局面あたりの探索ノード数
    #[arg(long, default_value_t = 1_000_000, value_parser = clap::value_parser!(u64).range(1..))]
    nodes: u64,

    /// 解析対象の対局者。対局者名にこの文字列を含む側を自分とみなす
    #[arg(long, default_value = "Himawari")]
    player: String,

    /// 大悪手とみなす評価の落差[cp]
    #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(i32).range(1..))]
    blunder_cp: i32,

    /// 逆転とみなす振れ幅[cp]。符号がプラスからマイナスへ変わり、
    /// かつ振れ幅がこの値以上の点を挙げる
    #[arg(long, default_value_t = 200, value_parser = clap::value_parser!(i32).range(1..))]
    flip_cp: i32,

    /// 詰みを調べる終盤の手数（自分の手番で数える）
    #[arg(long, default_value_t = 10)]
    mate_window: usize,

    /// 持ち時間の基準[秒]。残り時間の推定に使う（floodgateは300）
    #[arg(long, default_value_t = 300)]
    base_time: u64,

    /// 1手ごとの加算[秒]。省略時はCSAの 'Increment: を読む
    #[arg(long)]
    increment: Option<u64>,

    /// 序盤とみなす手数。ここまでの長考を挙げる
    #[arg(long, default_value_t = 24)]
    opening_ply: usize,

    /// 序盤の長考とみなす秒数
    #[arg(long, default_value_t = 30)]
    long_think: u64,

    /// 1秒以下の指し手が何手続いたら挙げるか
    #[arg(long, default_value_t = 10)]
    short_run: usize,

    /// 1局面を打ち切るまでの秒数
    #[arg(long, default_value_t = 600)]
    timeout: u64,

    /// レポートの書き出し先。既定は data/logs/floodgate-report-<今日>.md（UTC）
    #[arg(long, value_name = "パス")]
    out: Option<PathBuf>,

    /// 評価関数。省略時は環境変数 EVAL_FILE
    #[arg(long, value_name = "パス")]
    eval_file: Option<PathBuf>,
}

/// 1局面の再解析結果。
struct Analysed {
    /// 手番視点の評価値。詰みは±30000近傍へ写る。
    score_cp: i32,
    /// USI表記のscore（"cp 123" / "mate 5"）。
    score: String,
    bestmove: String,
    sfen: String,
    /// 合法手がない（詰み上がり）。探索していない。
    mated: bool,
}

impl Analysed {
    /// 詰ましている側の手番か。
    fn is_mate_win(&self) -> bool {
        !self.mated && self.score_cp >= MATE_MIN
    }
}

/// 1手ぶんの時間の使い方。
struct TimeStat {
    total_s: u64,
    max_s: u64,
    max_ply: usize,
    /// 残り時間の推定の最小値[秒]と、そのときの手数。
    min_left_s: i64,
    min_left_ply: usize,
    /// 序盤の長考 (手数, 秒)。
    long_thinks: Vec<(usize, u64)>,
    /// 1秒以下が続いた最長区間 (開始手数, 手数)。
    longest_short_run: (usize, usize),
    /// 加算[秒]。推定の前提としてレポートへ出す。
    increment_s: u64,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("エラー: {e:#}");
            ExitCode::from(exit::RUNTIME)
        }
    }
}

fn run(cli: &Cli) -> Result<u8> {
    let eval = eval_file(cli.eval_file.clone())?;
    ensure_executable(&cli.engine)?;
    let files = collect_csa(&cli.paths)?;
    if files.is_empty() {
        usage_error("CSAファイルが1つも見つからない");
    }
    let out = match &cli.out {
        Some(p) => p.clone(),
        None => PathBuf::from(format!("data/logs/floodgate-report-{}.md", today_ymd()?)),
    };

    let mut report = String::new();
    write_header(&mut report, cli, &eval, &files);

    let mut totals = (0usize, 0usize, 0usize); // (解析できた局, 大悪手, 詰み見逃し)
    let mut sections = String::new();
    for (i, path) in files.iter().enumerate() {
        eprintln!("[{}/{}] {}", i + 1, files.len(), basename(path));
        match analyse_game(cli, &eval, path) {
            Ok(section) => {
                totals.0 += 1;
                totals.1 += section.blunders;
                totals.2 += section.mate_misses;
                sections.push_str(&section.text);
            }
            Err(e) => {
                let _ = writeln!(sections, "## {}\n", basename(path));
                let _ = writeln!(sections, "解析できない: {e:#}\n");
            }
        }
    }

    let _ = writeln!(report, "## 全体\n");
    let _ = writeln!(report, "| 項目 | 値 |");
    let _ = writeln!(report, "|---|---|");
    let _ = writeln!(report, "| 棋譜 | {} |", files.len());
    let _ = writeln!(report, "| 解析できた局 | {} |", totals.0);
    let _ = writeln!(report, "| 大悪手 | {} |", totals.1);
    let _ = writeln!(report, "| 詰み見逃し | {} |", totals.2);
    report.push('\n');
    report.push_str(&sections);

    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("ディレクトリを作れない: {}", dir.display()))?;
    }
    std::fs::write(&out, &report)
        .with_context(|| format!("レポートを書けない: {}", out.display()))?;
    print!("{report}");
    eprintln!("レポート: {}", out.display());
    Ok(0)
}

fn write_header(report: &mut String, cli: &Cli, eval: &Path, files: &[PathBuf]) {
    let _ = writeln!(report, "# 棋譜の再解析（ADR-0152）\n");
    let _ = writeln!(report, "| 条件 | 値 |");
    let _ = writeln!(report, "|---|---|");
    let _ = writeln!(report, "| エンジン | {} |", cli.engine.display());
    let _ = writeln!(report, "| 評価関数 | {} |", eval.display());
    let _ = writeln!(report, "| 探索 | 1スレッド・{}ノード固定 |", cli.nodes);
    let _ = writeln!(
        report,
        "| 解析対象 | 対局者名に `{}` を含む側 |",
        cli.player
    );
    let _ = writeln!(report, "| 大悪手の閾値 | {}cp |", cli.blunder_cp);
    let _ = writeln!(
        report,
        "| 詰みを見る範囲 | 終局前の自分の手番{}手 |",
        cli.mate_window
    );
    let _ = writeln!(
        report,
        "| 時間の閾値 | 序盤{}手までに{}秒以上、1秒以下が{}手連続 |",
        cli.opening_ply, cli.long_think, cli.short_run
    );
    let _ = writeln!(report, "| 棋譜 | {}件 |", files.len());
    report.push('\n');
}

struct Section {
    text: String,
    blunders: usize,
    mate_misses: usize,
}

fn analyse_game(cli: &Cli, eval: &Path, path: &Path) -> Result<Section> {
    let raw = std::fs::read(path).with_context(|| format!("読めない: {}", path.display()))?;
    // 対局者名はASCIIだが、コメントに非UTF-8が混じる棋譜がある
    let text = String::from_utf8_lossy(&raw);
    let game = csa::parse(&text).map_err(anyhow::Error::new)?;
    let me = game
        .side_of(&cli.player)
        .ok_or_else(|| anyhow::anyhow!("対局者に「{}」がいない", cli.player))?;

    let replayed = replay(&game)?;
    let analysed = analyse_positions(cli, eval, &replayed)?;
    let blunders = find_blunders(cli, &game, me, &analysed);
    let turns = turn_evals(&game, me, &analysed);
    let flips = find_flips(cli, &turns);
    let mate_misses = find_mate_misses(cli, &game, me, &analysed);
    let time = time_stat(cli, &game, me);

    let mut text = String::new();
    write_section(
        &mut text,
        cli,
        &game,
        me,
        path,
        &analysed,
        &blunders,
        &mate_misses,
        &time,
    );
    write_flips_and_curve(&mut text, cli, &turns, &flips);
    Ok(Section {
        text,
        blunders: blunders.len(),
        mate_misses: mate_misses.len(),
    })
}

/// 棋譜を再生した結果。局面はSFENで持つ。
#[derive(Debug)]
struct Replay {
    /// USI表記の指し手列。
    moves: Vec<String>,
    /// 初期局面から最終局面までのSFEN（指し手数+1個）。
    sfens: Vec<String>,
    /// 合法手がない局面（詰み上がり）。SFENと同じ並び。
    mated: Vec<bool>,
}

/// CSAの指し手を合法手へ解決しながら盤を進める。
///
/// 解決できない手が1つでもあれば、そこから先の局面がすべてずれる。
/// エンジンへ読ませる前に止めて、棋譜ごとエラーにする。
fn replay(game: &CsaGame) -> Result<Replay> {
    let mut r = Replay {
        moves: Vec::with_capacity(game.moves.len()),
        sfens: Vec::with_capacity(game.moves.len() + 1),
        mated: Vec::with_capacity(game.moves.len() + 1),
    };
    let mut pos = Position::from_sfen(SFEN_STARTPOS).expect("平手初期局面");
    for (i, m) in game.moves.iter().enumerate() {
        record(&pos, &mut r);
        let mv = csa::resolve_move(&pos, m)
            .ok_or_else(|| anyhow::anyhow!("{}手目を合法手に解決できない: {}", i + 1, m.text))?;
        pos.do_move(mv);
        r.moves.push(mv.to_usi());
    }
    record(&pos, &mut r);
    Ok(r)
}

fn record(pos: &Position, r: &mut Replay) {
    r.sfens.push(pos.to_sfen());
    let mut legal = MoveList::default();
    generate_legal(pos, true, &mut legal);
    r.mated.push(legal.is_empty());
}

/// 全局面を固定ノード数で読む。局面ごとに `usinewgame` でTTを消し、
/// 前の局面の探索結果が次に効かないようにする（決定論の要件）。
///
/// 局面はSFENでなく `position startpos moves ...` で渡す。SFENだと
/// そこまでの手順が消え、千日手の判定が実戦と変わる。
fn analyse_positions(cli: &Cli, eval: &Path, replayed: &Replay) -> Result<Vec<Analysed>> {
    let mut options = single_thread_options(eval);
    // 定跡を引くと探索せずに手が返り、評価値が付かない
    options.push(("USI_OwnBook".to_string(), "false".to_string()));
    let mut eng = UsiEngine::launch(path_str(&cli.engine)?, &options).or_bail()?;

    let mut out = Vec::with_capacity(replayed.sfens.len());
    for (i, sfen) in replayed.sfens.iter().enumerate() {
        if replayed.mated[i] {
            // 詰み上がり。読ませてもbestmoveが返らない
            out.push(Analysed {
                score_cp: MATED_SCORE,
                score: "mate 0".to_string(),
                bestmove: "(詰み)".to_string(),
                sfen: sfen.clone(),
                mated: true,
            });
            continue;
        }
        let position_cmd = if i == 0 {
            "position startpos".to_string()
        } else {
            format!("position startpos moves {}", replayed.moves[..i].join(" "))
        };
        eng.new_game().or_bail()?;
        let result = eng
            .think(
                &position_cmd,
                &format!("go nodes {}", cli.nodes),
                Duration::from_secs(cli.timeout),
            )
            .or_bail()?;
        out.push(Analysed {
            score_cp: result.score_cp.unwrap_or(0),
            score: result.last_info.score.unwrap_or_else(|| "n/a".to_string()),
            bestmove: result.bestmove,
            sfen: sfen.clone(),
            mated: false,
        });
    }
    eng.quit();
    Ok(out)
}

/// 自分の手番かどうか。0始まりの指し手番号で数える。
fn is_mine(index: usize, me: Color) -> bool {
    let side = if index.is_multiple_of(2) {
        Color::Black
    } else {
        Color::White
    };
    side == me
}

struct Blunder {
    ply: usize,
    played: String,
    before: i32,
    after: i32,
    best: String,
    sfen: String,
}

/// 自分の手番から見た評価の1点。指す前と、指した直後（符号を自分視点へ
/// 反転）を持つ。
struct TurnEval {
    ply: usize,
    played: String,
    before: i32,
    after: i32,
    sfen: String,
    best: String,
}

/// 自分の手番ごとの評価列を作る。レポートの「評価の推移」と逆転検出が使う。
fn turn_evals(game: &CsaGame, me: Color, a: &[Analysed]) -> Vec<TurnEval> {
    let mut out = Vec::new();
    for i in 0..game.moves.len() {
        if !is_mine(i, me) || a[i].mated {
            continue;
        }
        out.push(TurnEval {
            ply: i + 1,
            played: game.moves[i].text.clone(),
            before: a[i].score_cp,
            after: -a[i + 1].score_cp,
            sfen: a[i].sfen.clone(),
            best: a[i].bestmove.clone(),
        });
    }
    out
}

/// 評価の逆転。どちらの種類かで意味が違う。
enum FlipKind {
    /// 自分の手で±flip_cpをまたいだ。1手での逆転（実際の悪手）
    OwnMove,
    /// 自分が指した直後は+側だったのに、相手の手を挟んだ次の手番で
    /// −側になっていた。相手の応手で評価が剥がれた＝直前の評価が
    /// 過大だった（水平線・過大評価の露呈）
    OpponentReveal,
}

struct Flip {
    kind: FlipKind,
    ply: usize,
    played: String,
    from: i32,
    to: i32,
    sfen: String,
    best: String,
}

/// 評価がプラスから一気にマイナスへ落ちた点を探す（2026-08-09オーナー指摘）。
/// 大悪手検出は落差の大きさしか見ないため、+160→−80のような「小さいが
/// 決定的な」反転を取りこぼす。「符号がまたがり、かつ振れ幅が閾値以上」を
/// 別枠で挙げる。
fn find_flips(cli: &Cli, turns: &[TurnEval]) -> Vec<Flip> {
    let t = cli.flip_cp;
    let crossed = |from: i32, to: i32| from > 0 && to < 0 && from - to >= t;
    let mut out = Vec::new();
    for (k, e) in turns.iter().enumerate() {
        // 自分の手の中での反転
        if crossed(e.before, e.after) {
            out.push(Flip {
                kind: FlipKind::OwnMove,
                ply: e.ply,
                played: e.played.clone(),
                from: e.before,
                to: e.after,
                sfen: e.sfen.clone(),
                best: e.best.clone(),
            });
        }
        // 相手の手を挟んだ反転（指した直後は+側、次の手番では−側）
        if let Some(next) = turns.get(k + 1)
            && crossed(e.after, next.before)
        {
            out.push(Flip {
                kind: FlipKind::OpponentReveal,
                ply: next.ply,
                played: e.played.clone(),
                from: e.after,
                to: next.before,
                sfen: next.sfen.clone(),
                best: next.best.clone(),
            });
        }
    }
    out
}

/// 逆転と評価の推移の節を書く。推移は自分の手番の全評価で、
/// 大悪手の閾値に届かない悪化やジリ貧もここで追える。
fn write_flips_and_curve(out: &mut String, cli: &Cli, turns: &[TurnEval], flips: &[Flip]) {
    use std::fmt::Write;
    let _ = writeln!(
        out,
        "### 逆転（符号がプラスからマイナスへ変わり、振れ幅{}cp以上）\n",
        cli.flip_cp
    );
    if flips.is_empty() {
        let _ = writeln!(out, "なし\n");
    } else {
        for f in flips {
            let (label, detail) = match f.kind {
                FlipKind::OwnMove => ("自分の手で逆転", "この手自体が敗着の候補"),
                FlipKind::OpponentReveal => (
                    "相手の応手で逆転",
                    "指した直後は+側で、相手の手を挟むと−側。直前の評価が過大だった（水平線・過大評価の露呈）",
                ),
            };
            let _ = writeln!(
                out,
                "- {}手目 `{}` : {:+} → {:+}（{}）",
                f.ply, f.played, f.from, f.to, label
            );
            let _ = writeln!(out, "  - SFEN: `{}`", f.sfen);
            let _ = writeln!(out, "  - 再解析の最善手: {}。{}", f.best, detail);
        }
        out.push('\n');
    }

    let _ = writeln!(out, "### 評価の推移（自分の手番、自分視点）\n");
    let _ = writeln!(out, "| 手数 | 実戦の手 | 指す前 | 指した後 |");
    let _ = writeln!(out, "|---|---|---|---|");
    for e in turns {
        let _ = writeln!(
            out,
            "| {} | {} | {:+} | {:+} |",
            e.ply, e.played, e.before, e.after
        );
    }
    out.push('\n');
}

/// 自分の手の前後で評価がどれだけ落ちたかを見る。
/// 評価は手番視点なので、指した後の局面は符号を反転して揃える。
fn find_blunders(cli: &Cli, game: &CsaGame, me: Color, a: &[Analysed]) -> Vec<Blunder> {
    let mut out = Vec::new();
    for i in 0..game.moves.len() {
        if !is_mine(i, me) || a[i].mated {
            continue;
        }
        let before = a[i].score_cp;
        let after = -a[i + 1].score_cp;
        if before - after < cli.blunder_cp {
            continue;
        }
        out.push(Blunder {
            ply: i + 1,
            played: game.moves[i].text.clone(),
            before,
            after,
            best: a[i].bestmove.clone(),
            sfen: a[i].sfen.clone(),
        });
    }
    out
}

struct MateMiss {
    ply: usize,
    played: String,
    score: String,
    after: String,
    best: String,
    sfen: String,
}

/// 終局前の自分の手番から `--mate-window` 手ぶんを見る。詰みを見つけている
/// のに、指した後の局面で詰みが消えていれば見逃しとして挙げる。
///
/// `go mate` を持たないので、固定ノード探索のmateスコアで代用する。
/// 読み切れていない詰みは挙がらないが、条件が同じなら結果は毎回同じになる。
fn find_mate_misses(cli: &Cli, game: &CsaGame, me: Color, a: &[Analysed]) -> Vec<MateMiss> {
    let mine: Vec<usize> = (0..game.moves.len()).filter(|&i| is_mine(i, me)).collect();
    let window = mine.len().saturating_sub(cli.mate_window);
    let mut out = Vec::new();
    for &i in &mine[window..] {
        if !a[i].is_mate_win() {
            continue;
        }
        // 指した後も自分の詰みが残っていれば見逃していない
        if a[i + 1].mated || a[i + 1].score_cp <= -MATE_MIN {
            continue;
        }
        out.push(MateMiss {
            ply: i + 1,
            played: game.moves[i].text.clone(),
            score: a[i].score.clone(),
            after: a[i + 1].score.clone(),
            best: a[i].bestmove.clone(),
            sfen: a[i].sfen.clone(),
        });
    }
    out
}

/// T行から消費時間を集計する。残り時間はフィッシャー方式の推定
/// （持ち時間 + 加算×手数 − 消費）で、切り上げや通信遅延は見ない。
fn time_stat(cli: &Cli, game: &CsaGame, me: Color) -> TimeStat {
    let inc = cli.increment.or(game.increment_s).unwrap_or(0);
    let mut stat = TimeStat {
        total_s: 0,
        max_s: 0,
        max_ply: 0,
        min_left_s: cli.base_time as i64,
        min_left_ply: 0,
        long_thinks: Vec::new(),
        longest_short_run: (0, 0),
        increment_s: inc,
    };
    let mut left = cli.base_time as i64;
    let mut run_start = 0usize;
    let mut run_len = 0usize;
    for (i, m) in game.moves.iter().enumerate() {
        if m.color != me {
            continue;
        }
        let ply = i + 1;
        let used = m.time_s.unwrap_or(0);
        stat.total_s += used;
        if used > stat.max_s {
            stat.max_s = used;
            stat.max_ply = ply;
        }
        left += inc as i64 - used as i64;
        if left < stat.min_left_s {
            stat.min_left_s = left;
            stat.min_left_ply = ply;
        }
        if ply <= cli.opening_ply && used >= cli.long_think {
            stat.long_thinks.push((ply, used));
        }
        if used <= 1 {
            if run_len == 0 {
                run_start = ply;
            }
            run_len += 1;
            if run_len > stat.longest_short_run.1 {
                stat.longest_short_run = (run_start, run_len);
            }
        } else {
            run_len = 0;
        }
    }
    stat
}

#[allow(clippy::too_many_arguments)]
fn write_section(
    out: &mut String,
    cli: &Cli,
    game: &CsaGame,
    me: Color,
    path: &Path,
    a: &[Analysed],
    blunders: &[Blunder],
    mate_misses: &[MateMiss],
    time: &TimeStat,
) {
    let side_name = if me == Color::Black {
        "先手"
    } else {
        "後手"
    };
    let result = match game.winner() {
        Some(w) if w == me => "勝ち",
        Some(_) => "負け",
        None => "引き分け・不明",
    };
    let _ = writeln!(out, "## {}\n", basename(path));
    let _ = writeln!(out, "| 項目 | 値 |");
    let _ = writeln!(out, "|---|---|");
    let _ = writeln!(out, "| 先手 | {} |", game.black);
    let _ = writeln!(out, "| 後手 | {} |", game.white);
    let _ = writeln!(out, "| 解析対象 | {}（{}） |", side_name, game.player(me));
    let _ = writeln!(
        out,
        "| 結果 | {} {}（{}） |",
        game.end.as_deref().unwrap_or("(終局の記録なし)"),
        result,
        game.summary.as_deref().unwrap_or("-")
    );
    let _ = writeln!(out, "| 手数 | {} |", game.moves.len());
    let _ = writeln!(
        out,
        "| 最終局面の評価 | {} |",
        a.last().map(|x| x.score.as_str()).unwrap_or("n/a")
    );
    out.push('\n');

    let _ = writeln!(out, "### 大悪手（{}cp以上の落差）\n", cli.blunder_cp);
    if blunders.is_empty() {
        let _ = writeln!(out, "なし\n");
    } else {
        let _ = writeln!(out, "| 手数 | 実戦の手 | 前 | 後 | 落差 | 再解析の最善手 |");
        let _ = writeln!(out, "|---|---|---|---|---|---|");
        for b in blunders {
            let _ = writeln!(
                out,
                "| {} | {} | {:+} | {:+} | {} | {} |",
                b.ply,
                b.played,
                b.before,
                b.after,
                b.before - b.after,
                b.best
            );
        }
        out.push('\n');
        for b in blunders {
            let _ = writeln!(out, "- {}手目 `{}` の直前", b.ply, b.played);
            let _ = writeln!(out, "  - SFEN: `{}`", b.sfen);
            let _ = writeln!(
                out,
                "  - 根拠: 指す前 {:+}cp、指した後 {:+}cp（自分視点）。再解析の最善手は {}",
                b.before, b.after, b.best
            );
        }
        out.push('\n');
    }

    let _ = writeln!(
        out,
        "### 詰み見逃し（終局前の自分の手番{}手）\n",
        cli.mate_window
    );
    if mate_misses.is_empty() {
        let _ = writeln!(out, "なし\n");
    } else {
        for m in mate_misses {
            let _ = writeln!(out, "- {}手目 `{}` の直前", m.ply, m.played);
            let _ = writeln!(out, "  - SFEN: `{}`", m.sfen);
            let _ = writeln!(
                out,
                "  - 根拠: 再解析は {} で詰みを読み、最善手は {}。実戦は {} を指し、直後の評価は {} になった",
                m.score, m.best, m.played, m.after
            );
        }
        out.push('\n');
    }

    let _ = writeln!(out, "### 時間\n");
    let _ = writeln!(out, "| 項目 | 値 |");
    let _ = writeln!(out, "|---|---|");
    let _ = writeln!(
        out,
        "| 推定の前提 | 持ち時間{}秒＋1手{}秒加算 |",
        cli.base_time, time.increment_s
    );
    let _ = writeln!(out, "| 総消費 | {}秒 |", time.total_s);
    let _ = writeln!(
        out,
        "| 最長の1手 | {}秒（{}手目） |",
        time.max_s, time.max_ply
    );
    let _ = writeln!(
        out,
        "| 残り時間の推定の最小 | {}秒（{}手目） |",
        time.min_left_s, time.min_left_ply
    );
    let long = if time.long_thinks.is_empty() {
        "なし".to_string()
    } else {
        time.long_thinks
            .iter()
            .map(|(ply, s)| format!("{ply}手目 {s}秒"))
            .collect::<Vec<_>>()
            .join("、")
    };
    let _ = writeln!(
        out,
        "| 序盤{}手までの長考（{}秒以上） | {} |",
        cli.opening_ply, cli.long_think, long
    );
    let (start, len) = time.longest_short_run;
    let short = if len >= cli.short_run {
        format!("{start}手目から{len}手")
    } else {
        format!("最長{len}手（閾値{}手に満たない）", cli.short_run)
    };
    let _ = writeln!(out, "| 1秒以下の連続 | {short} |");
    out.push('\n');
}

/// 引数のパスからCSAを集める。ディレクトリは再帰し、順はパスの昇順に固定する。
fn collect_csa(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut set = BTreeSet::new();
    for path in paths {
        collect_into(path, &mut set)?;
    }
    Ok(set.into_iter().collect())
}

fn collect_into(path: &Path, set: &mut BTreeSet<PathBuf>) -> Result<()> {
    let meta =
        std::fs::metadata(path).with_context(|| format!("読めないパス: {}", path.display()))?;
    if meta.is_dir() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(path)
            .with_context(|| format!("読めないディレクトリ: {}", path.display()))?
            .collect::<std::io::Result<Vec<_>>>()?
            .into_iter()
            .map(|e| e.path())
            .collect();
        entries.sort();
        for entry in entries {
            collect_into(&entry, set)?;
        }
        return Ok(());
    }
    if path.extension().is_some_and(|e| e == "csa") {
        set.insert(path.to_path_buf());
    }
    Ok(())
}

/// 今日の日付（YYYY-MM-DD、UTC）。既定の出力ファイル名にだけ使う。
/// レポートの中身には入れない（決定論の要件）。
fn today_ymd() -> Result<String> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("システム時刻が1970年より前")?
        .as_secs();
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    Ok(format!("{y:04}-{m:02}-{d:02}"))
}

/// 1970-01-01からの通日を暦日へ直す（Howard Hinnantの `civil_from_days`）。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        // 閏日をまたぐ
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        assert_eq!(civil_from_days(20_674), (2026, 8, 9));
    }

    /// 逆転の判定。符号がプラスからマイナスへ変わり、振れ幅が閾値以上の
    /// ものだけを挙げる。実戦で取りこぼした+160→−80（振れ幅240）の型と、
    /// 相手の応手で剥がれる型の両方を固定する。
    #[test]
    fn flips_require_sign_change_with_enough_swing() {
        let cli = cli(&["--flip-cp", "200"]);
        let turn = |ply: usize, before: i32, after: i32| TurnEval {
            ply,
            played: String::new(),
            before,
            after,
            sfen: String::new(),
            best: String::new(),
        };
        // 自分の手での逆転: +160→−80（振れ幅240 ≥ 200）
        let flips = find_flips(&cli, &[turn(1, 160, -80)]);
        assert_eq!(flips.len(), 1);
        assert!(matches!(flips[0].kind, FlipKind::OwnMove));
        // 振れ幅不足（+90→−90 = 180）は挙げない
        assert!(find_flips(&cli, &[turn(1, 90, -90)]).is_empty());
        // 符号が変わらない大差の悪化（+900→+100）は逆転ではない
        assert!(find_flips(&cli, &[turn(1, 900, 100)]).is_empty());
        // 相手の応手での逆転: 指した後+250、次の手番の前に−100
        let flips = find_flips(&cli, &[turn(1, 300, 250), turn(3, -100, -120)]);
        assert_eq!(flips.len(), 1);
        assert!(matches!(flips[0].kind, FlipKind::OpponentReveal));
        assert_eq!(flips[0].ply, 3);
    }

    /// 自分の手番の判定。先手なら偶数番目、後手なら奇数番目。
    #[test]
    fn is_mine_alternates_by_color() {
        assert!(is_mine(0, Color::Black));
        assert!(!is_mine(1, Color::Black));
        assert!(!is_mine(0, Color::White));
        assert!(is_mine(1, Color::White));
    }

    /// PIは平手初期局面の略記。ヘッダを最小にして再生だけを見る
    const KIFU: &str = "N+A\nN-B\nPI\n+\n+2726FU\nT1\n-3334FU\nT1\n%TORYO\n";

    /// 局面はSFENで持つ。指し手より1つ多い（初期局面と最終局面の両方）。
    #[test]
    fn replay_records_one_more_position_than_moves() {
        let game = csa::parse(KIFU).expect("パースできる");
        let r = replay(&game).expect("再生できる");
        assert_eq!(r.moves, vec!["2g2f", "3c3d"]);
        assert_eq!(r.sfens.len(), 3);
        assert!(r.sfens[0].starts_with("lnsgkgsnl"));
        assert_eq!(r.mated, vec![false, false, false]);
    }

    /// 解決できない手が1つでもあれば棋譜ごと落とす。以降の局面がずれる。
    #[test]
    fn replay_fails_on_an_unresolvable_move() {
        let game = csa::parse(&KIFU.replace("-3334FU", "-9999FU")).expect("パースできる");
        let err = replay(&game).expect_err("解決できない");
        assert!(err.to_string().contains("2手目"), "{err}");
    }

    fn cli(extra: &[&str]) -> Cli {
        let mut argv = vec!["kifu", "engine", "dummy.csa"];
        argv.extend_from_slice(extra);
        Cli::parse_from(argv)
    }

    /// 消費時間だけを与えた棋譜を作る。指し手の中身は検査に効かない。
    fn game_with_times(times: &[u64]) -> CsaGame {
        let sq = himawari_core::Square::new(himawari_core::File(0), himawari_core::Rank(0));
        CsaGame {
            moves: times
                .iter()
                .enumerate()
                .map(|(i, &t)| csa::CsaMove {
                    color: if i.is_multiple_of(2) {
                        Color::Black
                    } else {
                        Color::White
                    },
                    from: Some(sq),
                    to: sq,
                    piece: himawari_core::PieceType::PAWN,
                    time_s: Some(t),
                    text: format!("手{}", i + 1),
                    eval_cp: None,
                })
                .collect(),
            increment_s: Some(10),
            ..CsaGame::default()
        }
    }

    fn analysed(scores: &[i32]) -> Vec<Analysed> {
        scores
            .iter()
            .map(|&s| Analysed {
                score_cp: s,
                score: format!("cp {s}"),
                bestmove: "7g7f".to_string(),
                sfen: "sfen".to_string(),
                mated: false,
            })
            .collect()
    }

    /// 評価は手番視点なので、指した後は符号を反転して落差を測る。
    #[test]
    fn blunder_compares_scores_in_own_view() {
        let cli = cli(&["--blunder-cp", "300"]);
        let game = game_with_times(&[1, 1, 1, 1]);
        // 1手目: +100 → 相手番 +250（自分視点 -250）で落差350
        // 3手目: +100 → 相手番 -50（自分視点 +50）で落差50
        let a = analysed(&[100, 250, 100, -50, 0]);
        let found = find_blunders(&cli, &game, Color::Black, &a);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].ply, 1);
        assert_eq!((found[0].before, found[0].after), (100, -250));
    }

    /// 詰ました手を大悪手にしない。詰み上がりの局面を丸めた値で持つと、
    /// mate 1から落ちたように見えて挙がってしまう（実測で1件出た）。
    #[test]
    fn delivering_mate_is_not_a_blunder() {
        let cli = cli(&["--blunder-cp", "300"]);
        let game = game_with_times(&[1]);
        let mut a = analysed(&[29_999]); // mate 1
        a.push(Analysed {
            score_cp: MATED_SCORE,
            score: "mate 0".to_string(),
            bestmove: "(詰み)".to_string(),
            sfen: "sfen".to_string(),
            mated: true,
        });
        assert!(find_blunders(&cli, &game, Color::Black, &a).is_empty());
    }

    /// 相手の手は検査しない。自分の手だけを見る。
    #[test]
    fn blunder_ignores_opponent_moves() {
        let cli = cli(&["--blunder-cp", "300"]);
        let game = game_with_times(&[1, 1]);
        let a = analysed(&[0, 100, 900]);
        assert!(find_blunders(&cli, &game, Color::Black, &a).is_empty());
        // 同じ並びを後手視点で見ると、2手目が大悪手になる
        let found = find_blunders(&cli, &game, Color::White, &a);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].ply, 2);
    }

    /// 詰みを読んだのに指した後で消えていれば見逃し。残っていれば挙げない。
    #[test]
    fn mate_miss_needs_the_mate_to_disappear() {
        let cli = cli(&["--mate-window", "10"]);
        let game = game_with_times(&[1, 1, 1, 1]);
        // 1手目は詰みを逃し、3手目は詰ましている（相手番が詰まされ側）
        let a = analysed(&[29_900, 100, 29_900, -29_900, 0]);
        let found = find_mate_misses(&cli, &game, Color::Black, &a);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].ply, 1);
    }

    /// 窓の外は見ない。終局前の自分の手番だけを数える。
    #[test]
    fn mate_miss_only_looks_at_the_last_window() {
        let cli = cli(&["--mate-window", "1"]);
        let game = game_with_times(&[1, 1, 1, 1]);
        let a = analysed(&[29_900, 100, 29_900, 100, 0]);
        let found = find_mate_misses(&cli, &game, Color::Black, &a);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].ply, 3);
    }

    /// 残り時間はフィッシャー方式で積む。加算はCSAの 'Increment: を使う。
    #[test]
    fn time_stat_tracks_remaining_and_extremes() {
        let cli = cli(&[
            "--base-time",
            "300",
            "--long-think",
            "30",
            "--short-run",
            "3",
        ]);
        // 先手の消費は 40, 1, 1, 1（1・3・5・7手目）
        let game = game_with_times(&[40, 5, 1, 5, 1, 5, 1, 5]);
        let stat = time_stat(&cli, &game, Color::Black);
        assert_eq!(stat.increment_s, 10);
        assert_eq!(stat.total_s, 43);
        assert_eq!((stat.max_s, stat.max_ply), (40, 1));
        // 300 +10 -40 = 270 が最小。以降は加算のほうが大きく増えていく
        assert_eq!((stat.min_left_s, stat.min_left_ply), (270, 1));
        assert_eq!(stat.long_thinks, vec![(1, 40)]);
        assert_eq!(stat.longest_short_run, (3, 3));
    }

    /// --increment を渡したらCSAの値より優先する。
    #[test]
    fn time_stat_prefers_explicit_increment() {
        let cli = cli(&["--increment", "0"]);
        let game = game_with_times(&[10, 10]);
        let stat = time_stat(&cli, &game, Color::Black);
        assert_eq!(stat.increment_s, 0);
        assert_eq!(stat.min_left_s, 290);
    }
}
