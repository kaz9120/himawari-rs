//! PackedSfenValue教師データの前処理ツール（ADR-0038）。
//!
//! 使い方:
//!   psv stats   --in file [--limit N]          統計表示（局面数・score分布・勝敗）
//!   psv dump    --in file [--limit N]          SFENと教師信号を1行ずつ表示
//!   psv head    --in file --out file --count N [--skip M]   部分抽出
//!   psv shuffle --in file[,file...] --out file [--seed N] [--tmp DIR]  全体シャッフル
//!
//! shuffleは2パスのバケット法で動く（ADR-0065）。メモリ使用量は
//! バケット1個分（2GB）に収まるため、入力サイズの制限はない。
//! 一時ファイルは出力と同じディレクトリに作り、使い終わり次第消す。

use std::io::{BufWriter, Read, Seek, SeekFrom, Write};

use himawari_core::packed_sfen::{PSV_BYTES, PackedSfenValue, unpack_sfen};

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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        die("サブコマンドが必要です: stats / dump / head / shuffle");
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
        other => die(&format!("不明なサブコマンド: {other}")),
    }
}
