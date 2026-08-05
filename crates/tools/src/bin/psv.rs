//! PackedSfenValue教師データの前処理ツール（ADR-0038）。
//!
//! 使い方:
//!   psv stats   --in file [--limit N]          統計表示（局面数・score分布・勝敗）
//!   psv dump    --in file [--limit N]          SFENと教師信号を1行ずつ表示
//!   psv head    --in file --out file --count N [--skip M]   部分抽出
//!   psv shuffle --in file[,file...] --out file [--seed N] [--tmp DIR]  全体シャッフル
//!   psv quiet   --in file --out file [--limit N] [--max-plies N] [--hash MB]
//!                                              qsearchのPV葉へ置き換える（ADR-0136）
//!
//! shuffleは2パスのバケット法で動く（ADR-0065）。メモリ使用量は
//! バケット1個分（2GB）に収まるため、入力サイズの制限はない。
//! 一時ファイルは出力と同じディレクトリに作り、使い終わり次第消す。

use std::io::{BufWriter, Read, Seek, SeekFrom, Write};

use std::sync::Arc;

use himawari_core::packed_sfen::{PSV_BYTES, PackedSfenValue, pack, unpack, unpack_sfen};
use himawari_engine::eval::Evaluator;
use himawari_engine::movepick::Histories;
use himawari_engine::search::{Shared, Worker};
use himawari_engine::timeman::{Limits, TimeManager, TimeOptions};

fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1)
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.iter().position(|a| a == key).map(|i| {
        args.get(i + 1)
            .unwrap_or_else(|| die(&format!("{key} に値がありません")))
            .clone()
    })
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

fn open_reader(path: &str) -> std::io::BufReader<std::fs::File> {
    let f = std::fs::File::open(path).unwrap_or_else(|e| die(&format!("開けません: {path}: {e}")));
    std::io::BufReader::new(f)
}

fn stats(input: &str, limit: Option<u64>) {
    let mut r = open_reader(input);
    let mut buf = [0u8; PSV_BYTES];
    let mut n = 0u64;
    let mut decode_err = 0u64;
    let mut score_sum = 0i64;
    let (mut score_min, mut score_max) = (i16::MAX, i16::MIN);
    let mut results = [0u64; 3]; // 負け・引き分け・勝ち
    let mut ply_max = 0u16;
    let mut hist = [0u64; 9]; // |score|の桁別ヒストグラム
    while r.read_exact(&mut buf).is_ok() {
        let rec = PackedSfenValue::from_bytes(&buf);
        if n < 1000 && unpack_sfen(&rec.sfen, rec.game_ply).is_err() {
            decode_err += 1;
        }
        score_sum += i64::from(rec.score);
        score_min = score_min.min(rec.score);
        score_max = score_max.max(rec.score);
        match rec.game_result {
            -1 => results[0] += 1,
            0 => results[1] += 1,
            1 => results[2] += 1,
            _ => decode_err += 1,
        }
        ply_max = ply_max.max(rec.game_ply);
        let a = i32::from(rec.score).unsigned_abs();
        let bucket = match a {
            0..=99 => 0,
            100..=299 => 1,
            300..=599 => 2,
            600..=999 => 3,
            1000..=1999 => 4,
            2000..=2999 => 5,
            3000..=9999 => 6,
            10000..=29999 => 7,
            _ => 8,
        };
        hist[bucket] += 1;
        n += 1;
        if limit.is_some_and(|l| n >= l) {
            break;
        }
    }
    if n == 0 {
        die("レコードがありません");
    }
    println!("局面数: {n}");
    println!("先頭1000件のデコード失敗: {decode_err}");
    println!(
        "score: 平均{:.1} 最小{score_min} 最大{score_max}",
        score_sum as f64 / n as f64
    );
    println!(
        "勝敗（手番視点）: 勝ち{} 引き分け{} 負け{}",
        results[2], results[1], results[0]
    );
    println!("gamePly最大: {ply_max}");
    let labels = [
        "0-99",
        "100-299",
        "300-599",
        "600-999",
        "1000-1999",
        "2000-2999",
        "3000-9999",
        "10000-29999",
        "30000-",
    ];
    for (l, c) in labels.iter().zip(hist.iter()) {
        println!("|score| {l:>11}: {c}");
    }
}

fn dump(input: &str, limit: u64) {
    let mut r = open_reader(input);
    let mut buf = [0u8; PSV_BYTES];
    let mut n = 0u64;
    while n < limit && r.read_exact(&mut buf).is_ok() {
        let rec = PackedSfenValue::from_bytes(&buf);
        match unpack_sfen(&rec.sfen, rec.game_ply) {
            Ok(sfen) => println!(
                "{sfen} | score {} result {} ply {}",
                rec.score, rec.game_result, rec.game_ply
            ),
            Err(e) => println!("(デコード失敗: {e})"),
        }
        n += 1;
    }
}

fn head(input: &str, output: &str, count: u64, skip: u64) {
    let mut f =
        std::fs::File::open(input).unwrap_or_else(|e| die(&format!("開けません: {input}: {e}")));
    f.seek(SeekFrom::Start(skip * PSV_BYTES as u64))
        .unwrap_or_else(|e| die(&format!("seek失敗: {e}")));
    let mut r = std::io::BufReader::new(f);
    let mut w = BufWriter::new(
        std::fs::File::create(output).unwrap_or_else(|e| die(&format!("作成できません: {e}"))),
    );
    let mut buf = [0u8; PSV_BYTES];
    let mut n = 0u64;
    while n < count && r.read_exact(&mut buf).is_ok() {
        w.write_all(&buf)
            .unwrap_or_else(|e| die(&format!("書き込み失敗: {e}")));
        n += 1;
    }
    w.flush().unwrap();
    println!("{n}局面を{output}へ書き出しました（{skip}局面スキップ）");
}

/// 1バケットの目標サイズ。パス2でバケット1個をメモリに載せる（ADR-0065）。
const BUCKET_BYTES: u64 = 2 << 30;
/// バケットごとの書き込みバッファ。40バイト単位の書き込みをまとめる。
const BUCKET_BUF: usize = 1 << 20;

fn shuffle_in_place(buf: &mut [u8], rng: &mut Rng) {
    let n = buf.len() / PSV_BYTES;
    for i in (1..n).rev() {
        let j = (rng.next() % (i as u64 + 1)) as usize;
        if i != j {
            let (a, b) = (i * PSV_BYTES, j * PSV_BYTES);
            for k in 0..PSV_BYTES {
                buf.swap(a + k, b + k);
            }
        }
    }
}

/// 2パスのバケット法で全体をシャッフルする（ADR-0065）。
///
/// パス1で各レコードをランダムなバケットへ振り分け、パス2でバケット単位に
/// メモリ上でシャッフルして連結する。メモリ使用量はバケット1個分に収まる。
fn shuffle(inputs: &[&str], output: &str, seed: u64, tmp_dir: Option<&str>) {
    let total: u64 = inputs
        .iter()
        .map(|p| {
            std::fs::metadata(p)
                .unwrap_or_else(|e| die(&format!("開けません: {p}: {e}")))
                .len()
        })
        .sum();
    if !total.is_multiple_of(PSV_BYTES as u64) {
        die(&format!("入力サイズが40の倍数でない: {total}バイト"));
    }
    let n_buckets = (total.div_ceil(BUCKET_BYTES)).max(1) as usize;
    let dir = tmp_dir.map(std::path::PathBuf::from).unwrap_or_else(|| {
        std::path::Path::new(output)
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf()
    });
    let base = std::path::Path::new(output)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "psv".to_string());
    let paths: Vec<std::path::PathBuf> = (0..n_buckets)
        .map(|i| dir.join(format!(".{base}.bucket{i:04}")))
        .collect();

    let mut rng = Rng(seed.max(1));
    eprintln!(
        "パス1: {}局面を{n_buckets}バケットへ振り分けます",
        total / PSV_BYTES as u64
    );
    {
        let mut writers: Vec<BufWriter<std::fs::File>> = paths
            .iter()
            .map(|p| {
                let f = std::fs::File::create(p)
                    .unwrap_or_else(|e| die(&format!("作成できません: {}: {e}", p.display())));
                BufWriter::with_capacity(BUCKET_BUF, f)
            })
            .collect();
        let mut buf = [0u8; PSV_BYTES];
        for path in inputs {
            let mut r = open_reader(path);
            while r.read_exact(&mut buf).is_ok() {
                let b = (rng.next() % n_buckets as u64) as usize;
                writers[b]
                    .write_all(&buf)
                    .unwrap_or_else(|e| die(&format!("書き込み失敗: {e}")));
            }
        }
        for w in &mut writers {
            w.flush()
                .unwrap_or_else(|e| die(&format!("flush失敗: {e}")));
        }
    }

    eprintln!("パス2: バケットごとにシャッフルして連結します");
    let mut w = BufWriter::with_capacity(
        BUCKET_BUF,
        std::fs::File::create(output).unwrap_or_else(|e| die(&format!("作成できません: {e}"))),
    );
    let mut written = 0u64;
    for p in &paths {
        let mut data = std::fs::read(p).unwrap_or_else(|e| die(&format!("読み込み失敗: {e}")));
        shuffle_in_place(&mut data, &mut rng);
        w.write_all(&data)
            .unwrap_or_else(|e| die(&format!("書き込み失敗: {e}")));
        written += (data.len() / PSV_BYTES) as u64;
        let _ = std::fs::remove_file(p);
    }
    w.flush().unwrap();
    println!("{written}局面をシャッフルして{output}へ書き出しました (seed={seed})");
}

/// 教師局面をqsearchのPV葉へ置き換える（ADR-0136）。
///
/// 評価関数が探索中に見るのは静止局面である。ところがhao_depth9は
/// qsearch PV葉への置換なしで配られており、駒の取り合いの途中の局面へ
/// 取り合いが収束した後の探索値が付いている。この不整合を消す。
///
/// score・result・plyは元のまま残す（SF系の前処理と同じ扱い）。
fn quiet(input: &str, output: &str, limit: u64, max_plies: usize, hash_mb: usize, eval: &str) {
    let mut r = open_reader(input);
    let mut w = BufWriter::new(
        std::fs::File::create(output)
            .unwrap_or_else(|e| die(&format!("作れません: {output}: {e}"))),
    );
    let shared = Arc::new(Shared::new(hash_mb));
    let mut f = std::fs::File::open(eval)
        .unwrap_or_else(|e| die(&format!("評価関数を開けません: {eval}: {e}")));
    let (net, _lineage) = himawari_engine::nnue_io::load(&mut f)
        .unwrap_or_else(|e| die(&format!("評価関数を読めません: {eval}: {e}")));
    let net = Arc::new(net);

    // Workerは1つ作って使い回す。局面ごとに作るとhistory一式の確保が
    // 律速になり、実測で679局面/秒まで落ちた
    let limits = Limits::default();
    let start_pos = himawari_core::Position::from_sfen(himawari_core::SFEN_STARTPOS)
        .unwrap_or_else(|e| die(&format!("初期局面を作れません: {e:?}")));
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

    let mut buf = [0u8; PSV_BYTES];
    let (mut n, mut replaced, mut failed) = (0u64, 0u64, 0u64);
    let mut moved_plies = 0u64;
    // 教師のscoreと静的評価の乖離。静止化でこれが縮むかが本質である
    let mut gaps: Vec<(u32, u32)> = Vec::new();
    let start = std::time::Instant::now();
    while n < limit && r.read_exact(&mut buf).is_ok() {
        let mut rec = PackedSfenValue::from_bytes(&buf);
        let Ok(pos) = unpack(&rec.sfen, rec.game_ply) else {
            failed += 1;
            n += 1;
            continue;
        };
        worker.set_position(pos);
        // 置換前の静的評価。教師のscoreは元局面の手番から見た値である
        let before = worker.evaluator.evaluate(&worker.pos);
        let plies = worker.walk_to_quiet(max_plies);
        // 葉の手番が反転していたら符号を戻す
        let after_raw = worker.evaluator.evaluate(&worker.pos);
        let after = if plies % 2 == 1 {
            -after_raw
        } else {
            after_raw
        };
        let score = i32::from(rec.score);
        // 詰みスコア（±30000近傍）は乖離を支配するので統計から外す。
        // 置換していない局面は前後で同値なので、これも外す
        if plies > 0 && score.abs() <= 2000 {
            gaps.push((
                (before - score).unsigned_abs(),
                (after - score).unsigned_abs(),
            ));
        }
        if plies > 0 {
            match pack(&worker.pos) {
                Ok(packed) => {
                    rec.sfen = packed;
                    replaced += 1;
                    moved_plies += plies as u64;
                }
                Err(_) => failed += 1,
            }
        }
        w.write_all(&rec.to_bytes())
            .unwrap_or_else(|e| die(&format!("書けません: {e}")));
        n += 1;
        if n % 100_000 == 0 {
            let sec = start.elapsed().as_secs_f64();
            eprintln!(
                "{n}局面 置換{replaced} ({:.1}%) {:.0}局面/秒",
                replaced as f64 * 100.0 / n as f64,
                n as f64 / sec
            );
        }
    }
    w.flush()
        .unwrap_or_else(|e| die(&format!("書けません: {e}")));
    let sec = start.elapsed().as_secs_f64();
    println!("局面数        : {n}");
    println!(
        "置換率        : {replaced} ({:.2}%)",
        replaced as f64 * 100.0 / n.max(1) as f64
    );
    println!(
        "平均の進み手数: {:.2}（置換したものだけ）",
        moved_plies as f64 / replaced.max(1) as f64
    );
    println!("復元失敗      : {failed}");
    if gaps.is_empty() {
        println!("|静的評価-score| : 標本なし");
    } else {
        let k = gaps.len();
        let mean = |f: fn(&(u32, u32)) -> u32| {
            gaps.iter().map(|g| u64::from(f(g))).sum::<u64>() as f64 / k as f64
        };
        let median = |f: fn(&(u32, u32)) -> u32| {
            let mut v: Vec<u32> = gaps.iter().map(f).collect();
            v.sort_unstable();
            v[k / 2]
        };
        let closer = gaps.iter().filter(|g| g.1 < g.0).count();
        println!("--- 置換した局面のうち |score| <= 2000 の {k} 件 ---");
        println!(
            "|静的評価-score| : 平均 {:.1} → {:.1}  中央値 {} → {}",
            mean(|g| g.0),
            mean(|g| g.1),
            median(|g| g.0),
            median(|g| g.1)
        );
        println!(
            "scoreに近づいた  : {closer} ({:.1}%)",
            closer as f64 * 100.0 / k as f64
        );
    }
    println!(
        "所要          : {sec:.1}秒（{:.0}局面/秒）",
        n as f64 / sec.max(1e-9)
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        die("サブコマンドが必要です: stats / dump / head / shuffle / quiet");
    };
    let rest = &args[1..];
    let input = arg_value(rest, "--in");
    let output = arg_value(rest, "--out");
    match cmd.as_str() {
        "stats" => {
            let limit = arg_value(rest, "--limit").map(|s| s.parse().unwrap_or(u64::MAX));
            stats(&input.unwrap_or_else(|| die("--in が必要です")), limit);
        }
        "dump" => {
            let limit = arg_value(rest, "--limit")
                .map(|s| s.parse().unwrap_or(10))
                .unwrap_or(10);
            dump(&input.unwrap_or_else(|| die("--in が必要です")), limit);
        }
        "head" => {
            let count: u64 = arg_value(rest, "--count")
                .unwrap_or_else(|| die("--count が必要です"))
                .parse()
                .unwrap_or_else(|_| die("--count は整数"));
            let skip: u64 = arg_value(rest, "--skip")
                .map(|s| s.parse().unwrap_or(0))
                .unwrap_or(0);
            head(
                &input.unwrap_or_else(|| die("--in が必要です")),
                &output.unwrap_or_else(|| die("--out が必要です")),
                count,
                skip,
            );
        }
        "shuffle" => {
            let seed: u64 = arg_value(rest, "--seed")
                .map(|s| s.parse().unwrap_or(1))
                .unwrap_or(1);
            let input = input.unwrap_or_else(|| die("--in が必要です"));
            let inputs: Vec<&str> = input.split(',').collect();
            let tmp = arg_value(rest, "--tmp");
            shuffle(
                &inputs,
                &output.unwrap_or_else(|| die("--out が必要です")),
                seed,
                tmp.as_deref(),
            );
        }
        "quiet" => {
            let limit = arg_value(rest, "--limit")
                .map(|s| s.parse().unwrap_or(u64::MAX))
                .unwrap_or(u64::MAX);
            let max_plies = arg_value(rest, "--max-plies")
                .map(|s| s.parse().unwrap_or(16))
                .unwrap_or(16);
            let hash_mb = arg_value(rest, "--hash")
                .map(|s| s.parse().unwrap_or(64))
                .unwrap_or(64);
            let eval = arg_value(rest, "--eval-file")
                .or_else(|| std::env::var("EVAL_FILE").ok())
                .unwrap_or_else(|| die("--eval-file か EVAL_FILE が必要です"));
            quiet(
                &input.unwrap_or_else(|| die("--in が必要です")),
                &output.unwrap_or_else(|| die("--out が必要です")),
                limit,
                max_plies,
                hash_mb,
                &eval,
            );
        }
        other => die(&format!("不明なサブコマンド: {other}")),
    }
}
