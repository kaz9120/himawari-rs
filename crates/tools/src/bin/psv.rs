//! PackedSfenValue教師データの前処理ツール（ADR-0038）。
//!
//! 使い方:
//!   psv stats   --in file [--limit N]          統計表示（局面数・score分布・勝敗・勝率帯）
//!   psv dump    --in file [--limit N]          SFENと教師信号を1行ずつ表示
//!   psv head    --in file --out file --count N [--skip M]   部分抽出
//!   psv shuffle --in file[,file...] --out file [--seed N] [--tmp DIR]
//!               [--consume] [--parts N]        全体シャッフル。--consumeは読み終えた
//!                                              入力を消してピークを約1倍に抑える。
//!                                              --parts Nは出力を.partNNNへ分割する
//!   psv quiet   --in file --out file [--limit N] [--max-plies N（既定1）] [--hash MB]
//!               [--append] [--consume] [--jobs N]  qsearchのPV葉へ置き換える（ADR-0136）。
//!                                              --appendは出力へ追記、--consumeは完了後に
//!                                              入力を消す（分割入力の逐次処理用）。--jobsの
//!                                              並列出力はjobs固定で決定論（逐次とは不一致）
//!   psv rank    --in file --out file [--limit N] [--skip N] [--hash MB]
//!                                              兄弟局面の葉の群を作る（ADR-0185）
//!   psv thin    --in file --out file [--threshold N] [--keep P] [--seed N] [--group B]
//!                                              決着圏の局面を確率で間引く（ADR-0190）
//!   psv phase   --in file --out file.tsv [--limit N] [--eval-file NET]
//!                                              進行度の指標と静的評価をTSVへ書く（ADR-0198）
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
    let mut wp_hist = [0u64; 100]; // 勝率1%刻み（非詰み）。ADR-0190の診断
    let (mut mate_win, mut mate_lose) = (0u64, 0u64);
    // 盤上の駒数（玉2枚を含む）。出力バケットの境界を決める材料（ADR-0137）
    let mut piece_hist = [0u64; 41];
    let mut piece_n = 0u64;
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
        // 復元はSFENの展開を伴うので、先頭の標本だけで分布を見る
        if piece_n < 200_000
            && let Ok(pos) = unpack(&rec.sfen, rec.game_ply)
            && let count = pos.occupied().count() as usize
            && count < piece_hist.len()
        {
            piece_hist[count] += 1;
            piece_n += 1;
        }
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
        if i32::from(rec.score).abs() >= MATE_ABS {
            if rec.score > 0 {
                mate_win += 1;
            } else {
                mate_lose += 1;
            }
        } else {
            // 学習の損失と同じ勝率変換（crates/py の SIGMOID_SCALE=600）
            let wp = 1.0 / (1.0 + (-f64::from(rec.score) / 600.0).exp());
            let bin = ((wp * 100.0) as usize).min(99);
            wp_hist[bin] += 1;
        }
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
    let pct = |c: u64| c as f64 / n as f64 * 100.0;
    println!("勝率帯（sigmoid s/600、非詰み、全体比）:");
    for band in 0..10 {
        let c: u64 = wp_hist[band * 10..(band + 1) * 10].iter().sum();
        println!(
            "  {:>3}-{:<3}%: {:5.1}%",
            band * 10,
            (band + 1) * 10,
            pct(c)
        );
    }
    println!(
        "詰みスコア（手番視点 勝ち/負け）: {:.1}% / {:.1}%",
        pct(mate_win),
        pct(mate_lose)
    );
    println!(
        "端1%ビン（勝率0-1 / 99-100）: {:.1}% / {:.1}%",
        pct(wp_hist[0]),
        pct(wp_hist[99])
    );
    println!(
        "スパイク質量（端1%ビン＋詰み）: {:.1}%",
        pct(wp_hist[0] + wp_hist[99] + mate_win + mate_lose)
    );
    let inner = &wp_hist[5..95];
    let mean = inner.iter().sum::<u64>() as f64 / inner.len() as f64;
    if mean > 0.0 {
        let var = inner
            .iter()
            .map(|&c| (c as f64 - mean).powi(2))
            .sum::<f64>()
            / inner.len() as f64;
        println!("内側5〜95%の変動係数: {:.2}", var.sqrt() / mean);
    }

    if piece_n > 0 {
        // 出力バケットの境界は分布の四分位から決める（ADR-0137）
        println!("盤上の駒数（玉を含む、先頭{piece_n}局面）:");
        let mut acc = 0u64;
        let mut quartile = 1;
        for (count, &c) in piece_hist.iter().enumerate() {
            if c == 0 {
                continue;
            }
            acc += c;
            let pct = 100.0 * acc as f64 / piece_n as f64;
            let mark = if quartile <= 3 && pct >= 25.0 * f64::from(quartile) {
                quartile += 1;
                "  ← 四分位"
            } else {
                ""
            };
            println!(
                "  {count:2}枚: {:5.1}%（累積{pct:5.1}%）{mark}",
                100.0 * c as f64 / piece_n as f64
            );
        }
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

/// 詰みスコアの下限。この絶対値以上は間引かず全件残す（詰み域を欠かさない、ADR-0188）。
const MATE_ABS: i32 = 29000;

/// 決着圏の局面を確率で間引く（ADR-0190）。
///
/// レコードのscore（groupが120ならば先頭レコードのscore）を見て、
/// 非詰みかつ|score|がthreshold以上のものをkeepの確率で残す。
/// それ以外（互角圏〜優勢圏と詰みスコア）は全件残す。複製はしないので、
/// 分布の山を新たに作ることはない。
fn thin(input: &str, output: &str, threshold: i32, keep: f64, seed: u64, group: usize) {
    if !group.is_multiple_of(PSV_BYTES) {
        die(&format!("--group は{PSV_BYTES}の倍数にしてください"));
    }
    let mut r = open_reader(input);
    let mut w = BufWriter::new(
        std::fs::File::create(output).unwrap_or_else(|e| die(&format!("作成できません: {e}"))),
    );
    let mut rng = Rng(seed | 1);
    let mut buf = vec![0u8; group];
    let (mut total, mut decided, mut kept_decided) = (0u64, 0u64, 0u64);
    while r.read_exact(&mut buf).is_ok() {
        total += 1;
        let score = i32::from(i16::from_le_bytes([buf[32], buf[33]]));
        let is_decided = score.abs() >= threshold && score.abs() < MATE_ABS;
        if is_decided {
            decided += 1;
            let p = (rng.next() >> 11) as f64 / (1u64 << 53) as f64;
            if p >= keep {
                continue;
            }
            kept_decided += 1;
        }
        w.write_all(&buf)
            .unwrap_or_else(|e| die(&format!("書き込み失敗: {e}")));
    }
    w.flush().unwrap();
    let written = total - (decided - kept_decided);
    println!(
        "入力{total}件のうち決着圏（{threshold}<=|score|<{MATE_ABS}）{decided}件を\
         {kept_decided}件へ間引き、{written}件を書き出しました"
    );
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
///
/// consumeは読み終えた入力ファイルから順に消し、ディスクのピークを
/// 入力1倍強に抑える（ADR-0192）。再取得できる生データにだけ使う。
/// partsが2以上なら、出力を `<出力名>.partNNN` のほぼ等分な連番へ分ける。
/// 分割してもレコードの割り付けはseedだけで決まり、連結すればparts=1と
/// 同じ並びになる。
fn shuffle(
    inputs: &[&str],
    output: &str,
    seed: u64,
    tmp_dir: Option<&str>,
    consume: bool,
    parts: usize,
    bucket_bytes: u64,
) {
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
    let n_buckets = (total.div_ceil(bucket_bytes)).max(1) as usize;
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
            if consume {
                // バケットへ写し終えた入力から順に消し、ピークを抑える
                for w in &mut writers {
                    w.flush()
                        .unwrap_or_else(|e| die(&format!("flush失敗: {e}")));
                }
                std::fs::remove_file(path)
                    .unwrap_or_else(|e| die(&format!("入力を消せません: {path}: {e}")));
            }
        }
        for w in &mut writers {
            w.flush()
                .unwrap_or_else(|e| die(&format!("flush失敗: {e}")));
        }
    }

    eprintln!("パス2: バケットごとにシャッフルして連結します");
    if parts > n_buckets {
        eprintln!("注意: バケットが{n_buckets}個しかないため、分割数を{n_buckets}に丸めます");
    }
    let parts = parts.max(1).min(n_buckets);
    let out_name = |part: usize| -> String {
        if parts == 1 {
            output.to_string()
        } else {
            format!("{output}.part{part:03}")
        }
    };
    let mut written = 0u64;
    for part in 0..parts {
        // バケットをほぼ等分な連番グループに割り、グループごとに1本書く
        let lo = n_buckets * part / parts;
        let hi = n_buckets * (part + 1) / parts;
        let name = out_name(part);
        let mut w = BufWriter::with_capacity(
            BUCKET_BUF,
            std::fs::File::create(&name)
                .unwrap_or_else(|e| die(&format!("作成できません: {name}: {e}"))),
        );
        for p in &paths[lo..hi] {
            let mut data = std::fs::read(p).unwrap_or_else(|e| die(&format!("読み込み失敗: {e}")));
            shuffle_in_place(&mut data, &mut rng);
            w.write_all(&data)
                .unwrap_or_else(|e| die(&format!("書き込み失敗: {e}")));
            written += (data.len() / PSV_BYTES) as u64;
            let _ = std::fs::remove_file(p);
        }
        w.flush().unwrap();
    }
    if parts == 1 {
        println!("{written}局面をシャッフルして{output}へ書き出しました (seed={seed})");
    } else {
        println!(
            "{written}局面をシャッフルして{output}.part000〜{:03}の{parts}本へ書き出しました (seed={seed})",
            parts - 1
        );
    }
}

/// 教師局面をqsearchのPV葉へ置き換える（ADR-0136）。
///
/// 評価関数が探索中に見るのは静止局面である。ところがhao_depth9は
/// qsearch PV葉への置換なしで配られており、駒の取り合いの途中の局面へ
/// 取り合いが収束した後の探索値が付いている。この不整合を消す。
///
/// 評価値と勝敗は元の値を保つ。ただしどちらも手番視点なので、奇数手
/// 進めたときは符号を戻す。手数は進めた分を足し、PVの初手は捨てる。
/// 並列quietの1チャンクのレコード数。並列時の決定論の単位になる。
const QUIET_CHUNK: usize = 8192;

/// quietの1チャンクぶんの統計。
#[derive(Default)]
struct QuietStats {
    n: u64,
    replaced: u64,
    failed: u64,
    moved_plies: u64,
    gaps: Vec<(u32, u32, u8)>,
}

/// quiet用のWorkerを作る。並列時はスレッドごとに1つ持ち、TTも独立になる。
fn quiet_worker(net: &Arc<himawari_engine::nnue::NnueNetwork>, hash_mb: usize) -> Worker {
    let shared = Arc::new(Shared::new(hash_mb));
    let limits = Limits::default();
    let start_pos = himawari_core::Position::from_sfen(himawari_core::SFEN_STARTPOS)
        .unwrap_or_else(|e| die(&format!("初期局面を作れません: {e:?}")));
    let tm = TimeManager::new(
        &limits,
        start_pos.side_to_move(),
        start_pos.game_ply(),
        &TimeOptions::default(),
    );
    Worker::new(
        start_pos,
        shared,
        limits,
        tm,
        0,
        1,
        Evaluator::nnue(Arc::clone(net)),
        Histories::default(),
    )
}

/// 1レコードを静止局面へ置き換える。統計はstへ足す。
fn quiet_record(
    worker: &mut Worker,
    buf: &[u8; PSV_BYTES],
    max_plies: usize,
    st: &mut QuietStats,
) -> [u8; PSV_BYTES] {
    st.n += 1;
    let mut rec = PackedSfenValue::from_bytes(buf);
    let Ok(pos) = unpack(&rec.sfen, rec.game_ply) else {
        st.failed += 1;
        return *buf;
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
        st.gaps.push((
            (before - score).unsigned_abs(),
            (after - score).unsigned_abs(),
            plies.min(7) as u8,
        ));
    }
    if plies > 0 {
        match pack(&worker.pos) {
            Ok(packed) => {
                rec.sfen = packed;
                // score・game_resultはどちらも手番視点である。奇数手
                // 進めると手番が入れ替わるので、符号を戻さないと
                // ラベルが逆になる
                if plies % 2 == 1 {
                    rec.score = rec.score.saturating_neg();
                    rec.game_result = -rec.game_result;
                }
                rec.game_ply = rec.game_ply.saturating_add(plies as u16);
                // PVの初手は元局面のものである。葉では指せないので捨てる
                rec.move16 = 0;
                st.replaced += 1;
                st.moved_plies += plies as u64;
            }
            Err(_) => st.failed += 1,
        }
    }
    rec.to_bytes()
}

/// quietの入出力の扱い。分割入力を1本へ集約する逐次処理で使う。
struct QuietMode {
    /// 出力へ追記する（既定は新規作成）
    append: bool,
    /// 完了後に入力を消す
    consume: bool,
    /// 並列数。2以上でチャンク並列になる（jobs固定で決定論）
    jobs: usize,
}

fn quiet(
    input: &str,
    output: &str,
    limit: u64,
    max_plies: usize,
    hash_mb: usize,
    eval: &str,
    mode: QuietMode,
) {
    let jobs = mode.jobs.max(1);
    let mut r = open_reader(input);
    let file = if mode.append {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(output)
    } else {
        std::fs::File::create(output)
    };
    let mut w = BufWriter::new(file.unwrap_or_else(|e| die(&format!("作れません: {output}: {e}"))));
    let mut f = std::fs::File::open(eval)
        .unwrap_or_else(|e| die(&format!("評価関数を開けません: {eval}: {e}")));
    let (net, _lineage) = himawari_engine::nnue_io::load(&mut f)
        .unwrap_or_else(|e| die(&format!("評価関数を読めません: {eval}: {e}")));
    let net = Arc::new(net);

    let (mut n, mut replaced, mut failed) = (0u64, 0u64, 0u64);
    let mut moved_plies = 0u64;
    // 教師のscoreと静的評価の乖離。静止化でこれが縮むかが本質である
    let mut gaps: Vec<(u32, u32, u8)> = Vec::new();
    let start = std::time::Instant::now();

    if jobs <= 1 {
        // 逐次。従来と同じ順でTTを引き継ぎながら処理する
        let mut worker = quiet_worker(&net, hash_mb);
        let mut buf = [0u8; PSV_BYTES];
        let mut st = QuietStats::default();
        while n < limit && r.read_exact(&mut buf).is_ok() {
            let out = quiet_record(&mut worker, &buf, max_plies, &mut st);
            w.write_all(&out)
                .unwrap_or_else(|e| die(&format!("書けません: {e}")));
            n += 1;
            if n % 100_000 == 0 {
                let sec = start.elapsed().as_secs_f64();
                eprintln!(
                    "{n}局面 置換{} ({:.1}%) {:.0}局面/秒",
                    st.replaced,
                    st.replaced as f64 * 100.0 / n as f64,
                    n as f64 / sec
                );
            }
        }
        replaced = st.replaced;
        failed = st.failed;
        moved_plies = st.moved_plies;
        gaps = st.gaps;
    } else {
        // 並列。チャンクを chunk_id % jobs のワーカーへ固定に配り、出力は
        // chunk_id順へ並べ直して書く。**ワーカーごとのTTは自分のチャンク列
        // だけを見るので、jobsとQUIET_CHUNKを固定すれば出力は決定論に
        // なる**。逐次とは一致しない（逐次はTTを全レコードで引き継ぐ）。
        std::thread::scope(|scope| {
            let (out_tx, out_rx) =
                std::sync::mpsc::sync_channel::<(u64, Vec<u8>, QuietStats)>(jobs * 2);
            let mut in_txs = Vec::with_capacity(jobs);
            for _ in 0..jobs {
                let (tx, rx) = std::sync::mpsc::sync_channel::<(u64, Vec<u8>)>(2);
                in_txs.push(tx);
                let out_tx = out_tx.clone();
                let net = Arc::clone(&net);
                scope.spawn(move || {
                    let mut worker = quiet_worker(&net, hash_mb);
                    for (id, data) in rx {
                        let mut out = Vec::with_capacity(data.len());
                        let mut st = QuietStats::default();
                        for chunk in data.as_chunks::<PSV_BYTES>().0 {
                            out.extend_from_slice(&quiet_record(
                                &mut worker,
                                chunk,
                                max_plies,
                                &mut st,
                            ));
                        }
                        if out_tx.send((id, out, st)).is_err() {
                            return;
                        }
                    }
                });
            }
            drop(out_tx);

            scope.spawn(move || {
                let mut id = 0u64;
                let mut left = limit;
                let mut buf = [0u8; PSV_BYTES];
                'read: loop {
                    let take = (QUIET_CHUNK as u64).min(left) as usize;
                    if take == 0 {
                        break;
                    }
                    let mut data = Vec::with_capacity(take * PSV_BYTES);
                    for _ in 0..take {
                        if r.read_exact(&mut buf).is_err() {
                            if !data.is_empty() {
                                let dest = (id % jobs as u64) as usize;
                                let _ = in_txs[dest].send((id, data));
                            }
                            break 'read;
                        }
                        data.extend_from_slice(&buf);
                    }
                    left -= (data.len() / PSV_BYTES) as u64;
                    let dest = (id % jobs as u64) as usize;
                    if in_txs[dest].send((id, data)).is_err() {
                        break;
                    }
                    id += 1;
                }
                drop(in_txs);
            });

            let mut pending: std::collections::BTreeMap<u64, (Vec<u8>, QuietStats)> =
                std::collections::BTreeMap::new();
            let mut next = 0u64;
            for (id, data, st) in out_rx {
                pending.insert(id, (data, st));
                while let Some((data, st)) = pending.remove(&next) {
                    w.write_all(&data)
                        .unwrap_or_else(|e| die(&format!("書けません: {e}")));
                    let before_m = n / 1_000_000;
                    n += st.n;
                    replaced += st.replaced;
                    failed += st.failed;
                    moved_plies += st.moved_plies;
                    gaps.extend(st.gaps);
                    if n / 1_000_000 != before_m {
                        let sec = start.elapsed().as_secs_f64();
                        eprintln!(
                            "{n}局面 置換{replaced} ({:.1}%) {:.0}局面/秒",
                            replaced as f64 * 100.0 / n as f64,
                            n as f64 / sec
                        );
                    }
                    next += 1;
                }
            }
        });
    }

    w.flush()
        .unwrap_or_else(|e| die(&format!("書けません: {e}")));
    if mode.consume {
        std::fs::remove_file(input)
            .unwrap_or_else(|e| die(&format!("入力を消せません: {input}: {e}")));
        println!("入力を消しました: {input}");
    }
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
        let stat = |sel: &dyn Fn(&(u32, u32, u8)) -> bool| {
            let v: Vec<&(u32, u32, u8)> = gaps.iter().filter(|g| sel(g)).collect();
            if v.is_empty() {
                return None;
            }
            let m = v.len();
            let mut b: Vec<u32> = v.iter().map(|g| g.0).collect();
            let mut a: Vec<u32> = v.iter().map(|g| g.1).collect();
            b.sort_unstable();
            a.sort_unstable();
            let closer = v.iter().filter(|g| g.1 < g.0).count();
            Some((m, b[m / 2], a[m / 2], closer as f64 * 100.0 / m as f64))
        };
        println!("--- 置換した局面のうち |score| <= 2000 の {k} 件 ---");
        println!("| 進み手数 | 件数 | |評価-score| 中央値 前→後 | 近づいた |");
        println!("|---|---|---|---|");
        if let Some((m, bm, am, c)) = stat(&|_| true) {
            println!("| 全体 | {m} | {bm} → {am} | {c:.1}% |");
        }
        for p in 1..=5u8 {
            let label = if p == 5 { "5手以上" } else { "" };
            let sel = |g: &(u32, u32, u8)| if p == 5 { g.2 >= 5 } else { g.2 == p };
            if let Some((m, bm, am, c)) = stat(&sel) {
                if label.is_empty() {
                    println!("| {p}手 | {m} | {bm} → {am} | {c:.1}% |");
                } else {
                    println!("| {label} | {m} | {bm} → {am} | {c:.1}% |");
                }
            }
        }
    }
    println!(
        "所要          : {sec:.1}秒（{:.0}局面/秒）",
        n as f64 / sec.max(1e-9)
    );
}

/// 兄弟局面の葉の群を作る（ADR-0185）。
///
/// 教師の最善手の子と、他の合法手から引いた負例2手の子を、それぞれ
/// qsearchのPV葉まで進めて 正例・負例・負例 の順に書く。1群は
/// 40バイト×3。予備バイト（b[39]）に親からの手数の偶奇を入れ、学習側は
/// これで葉の評価値を親視点の符号へ戻す。
///
/// 乱数はレコードの通し番号（--skipを含む絶対位置）から決定論で引く。
/// 分割して並列に走らせても、結合結果は1本で走らせた場合と一致する。
fn rank(input: &str, output: &str, limit: u64, skip: u64, hash_mb: usize, eval: &str) {
    use himawari_core::Move16;
    use std::io::Seek;

    let mut r = open_reader(input);
    r.seek(std::io::SeekFrom::Start(skip * PSV_BYTES as u64))
        .unwrap_or_else(|e| die(&format!("シークできません: {e}")));
    let mut w = BufWriter::new(
        std::fs::File::create(output)
            .unwrap_or_else(|e| die(&format!("作れません: {output}: {e}"))),
    );
    let mut f = std::fs::File::open(eval)
        .unwrap_or_else(|e| die(&format!("評価関数を開けません: {eval}: {e}")));
    let (net, _lineage) = himawari_engine::nnue_io::load(&mut f)
        .unwrap_or_else(|e| die(&format!("評価関数を読めません: {eval}: {e}")));
    let net = Arc::new(net);
    let mut worker = quiet_worker(&net, hash_mb);

    // xorshift。シードはレコードの絶対位置で、0を避けるため定数を混ぜる
    let rng_next = |s: &mut u64| {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    };

    let mut buf = [0u8; PSV_BYTES];
    let (mut n, mut groups, mut skipped) = (0u64, 0u64, 0u64);
    let mut skip_why = [0u64; 5];
    let start = std::time::Instant::now();
    while n < limit && r.read_exact(&mut buf).is_ok() {
        let rec = PackedSfenValue::from_bytes(&buf);
        n += 1;
        let Ok(pos) = unpack(&rec.sfen, rec.game_ply) else {
            skipped += 1;
            skip_why[0] += 1;
            continue;
        };
        let Some(m16) = Move16::from_yaneura(rec.move16) else {
            skipped += 1;
            skip_why[1] += 1;
            continue;
        };
        let Some(best) = pos.to_move(m16) else {
            skipped += 1;
            skip_why[1] += 1;
            continue;
        };
        if !pos.pseudo_legal(best) || !pos.is_legal(best) {
            skipped += 1;
            skip_why[2] += 1;
            continue;
        }
        let mut list = himawari_core::MoveList::default();
        himawari_core::generate_legal(&pos, true, &mut list);
        let others: Vec<himawari_core::Move> = list
            .as_slice()
            .iter()
            .copied()
            .filter(|&m| m != best)
            .collect();
        if others.len() < 2 {
            skipped += 1;
            skip_why[3] += 1;
            continue;
        }
        let mut seed = (skip + n) ^ 0x9E37_79B9_7F4A_7C15;
        let i1 = (rng_next(&mut seed) as usize) % others.len();
        let i2 = {
            let mut j = (rng_next(&mut seed) as usize) % (others.len() - 1);
            if j >= i1 {
                j += 1;
            }
            j
        };

        // 3つの子を葉へ進めてpackする。1つでも失敗したら群ごと捨てる
        let mut out_recs: Vec<[u8; PSV_BYTES]> = Vec::with_capacity(3);
        for m in [best, others[i1], others[i2]] {
            let mut p = pos.clone();
            p.do_move(m);
            worker.set_position(p);
            let plies = 1 + worker.walk_to_quiet(16);
            let Ok(packed) = pack(&worker.pos) else {
                break;
            };
            let mut child = rec;
            child.sfen = packed;
            child.move16 = 0;
            child.game_ply = rec.game_ply.saturating_add(plies as u16);
            let mut bytes = child.to_bytes();
            // 予備バイトへ親からの手数の偶奇を入れる（ADR-0185）
            bytes[39] = (plies % 2) as u8;
            out_recs.push(bytes);
        }
        if out_recs.len() != 3 {
            skipped += 1;
            skip_why[4] += 1;
            continue;
        }
        for bytes in &out_recs {
            w.write_all(bytes)
                .unwrap_or_else(|e| die(&format!("書けません: {e}")));
        }
        groups += 1;
        if n % 100_000 == 0 {
            let sec = start.elapsed().as_secs_f64();
            eprintln!(
                "{n}局面 群{groups} ({:.0}局面/秒)",
                n as f64 / sec.max(1e-9)
            );
        }
    }
    w.flush()
        .unwrap_or_else(|e| die(&format!("書けません: {e}")));
    let sec = start.elapsed().as_secs_f64();
    println!("読んだ局面 : {n}");
    println!("書いた群   : {groups}");
    println!(
        "捨てた局面 : {skipped}（復元{} 手復号{} 非合法{} 手不足{} pack{}）",
        skip_why[0], skip_why[1], skip_why[2], skip_why[3], skip_why[4]
    );
    println!(
        "所要       : {sec:.1}秒（{:.0}局面/秒）",
        n as f64 / sec.max(1e-9)
    );
}

/// 進行度の指標の候補（ADR-0198）。TSVの列名で、`phase_features` の
/// 戻り値と同じ順に並ぶ。
const PHASE_COLUMNS: [&str; 10] = [
    "board",
    "ply",
    "mixed",
    "contact",
    "advanced",
    "promoted",
    "majors",
    "camp",
    "king_adv",
    "king_zone",
];

/// 局面から進行度の指標を10本まとめて計算する。定義はADR-0198の表にある。
fn phase_features(pos: &himawari_core::Position) -> [u32; 10] {
    use himawari_core::attacks::king_attacks;
    use himawari_core::{Bitboard, Color, File, PieceType, Rank, Square};

    let occ = pos.occupied();
    // 2×2の窓に先手と後手の駒が両方あるか（Lichessの混合度の骨格）
    let mut mixed = 0;
    for f in 0..8u8 {
        for r in 0..8u8 {
            let mut has = [false; 2];
            for (df, dr) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                let pc = pos.piece_on(Square::new(File(f + df), Rank(r + dr)));
                if !pc.is_empty() {
                    has[pc.color().index()] = true;
                }
            }
            if has[0] && has[1] {
                mixed += 1;
            }
        }
    }
    let (mut contact, mut advanced, mut promoted, mut majors) = (0, 0, 0, 0);
    let mut camp = [0u32; 2];
    for sq in occ {
        let pc = pos.piece_on(sq);
        let (c, pt) = (pc.color(), pc.piece_type());
        if !pos.attackers_to(c.flip(), sq, occ).is_empty() {
            contact += 1;
        }
        let in_enemy = Bitboard::promotion_zone(c).test(sq);
        if pt != PieceType::KING && in_enemy {
            advanced += 1;
        }
        if pt.is_promoted() {
            promoted += 1;
        }
        let is_major = matches!(
            pt,
            PieceType::ROOK | PieceType::BISHOP | PieceType::DRAGON | PieceType::HORSE
        );
        if is_major && (pt.is_promoted() || in_enemy) {
            majors += 1;
        }
        // 自陣は相手から見た敵陣
        if Bitboard::promotion_zone(c.flip()).test(sq) {
            camp[c.index()] += 1;
        }
    }
    let (mut king_adv, mut king_zone) = (0, 0);
    for c in [Color::Black, Color::White] {
        let k = pos.king(c);
        // relativeは自陣の1段目を8にするので、進出度は8から引く
        king_adv = king_adv.max(8 - u32::from(k.rank().relative(c).0));
        let zone: u32 = king_attacks(k)
            .into_iter()
            .map(|sq| pos.attackers_to(c.flip(), sq, occ).count())
            .sum();
        king_zone = king_zone.max(zone);
    }
    [
        occ.count(),
        u32::from(pos.game_ply()),
        mixed,
        contact,
        advanced,
        promoted,
        majors,
        camp[0].min(camp[1]),
        king_adv,
        king_zone,
    ]
}

/// 局面ごとの教師信号・静的評価・進行度の指標をTSVへ書く（ADR-0198）。
/// 集計は `hmwr net phase` のPython側が持つ。
fn phase(input: &str, output: &str, limit: u64, eval: &str) {
    let mut r = open_reader(input);
    let mut w = BufWriter::new(
        std::fs::File::create(output)
            .unwrap_or_else(|e| die(&format!("作れません: {output}: {e}"))),
    );
    let mut f = std::fs::File::open(eval)
        .unwrap_or_else(|e| die(&format!("評価関数を開けません: {eval}: {e}")));
    let (net, _lineage) = himawari_engine::nnue_io::load(&mut f)
        .unwrap_or_else(|e| die(&format!("評価関数を読めません: {eval}: {e}")));
    let mut worker = quiet_worker(&Arc::new(net), 1);

    writeln!(w, "score\tresult\teval\t{}", PHASE_COLUMNS.join("\t"))
        .unwrap_or_else(|e| die(&format!("書けません: {e}")));
    let mut buf = [0u8; PSV_BYTES];
    let (mut n, mut failed) = (0u64, 0u64);
    while n < limit && r.read_exact(&mut buf).is_ok() {
        n += 1;
        let rec = PackedSfenValue::from_bytes(&buf);
        let Ok(pos) = unpack(&rec.sfen, rec.game_ply) else {
            failed += 1;
            continue;
        };
        let features = phase_features(&pos);
        worker.set_position(pos);
        // 教師のscoreと同じく手番視点の値になる
        let value = worker.evaluator.evaluate(&worker.pos);
        let cols: Vec<String> = features.iter().map(u32::to_string).collect();
        writeln!(
            w,
            "{}\t{}\t{}\t{}",
            rec.score,
            rec.game_result,
            value,
            cols.join("\t")
        )
        .unwrap_or_else(|e| die(&format!("書けません: {e}")));
    }
    w.flush()
        .unwrap_or_else(|e| die(&format!("書けません: {e}")));
    eprintln!("局面数: {n}（復元失敗{failed}）→ {output}");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        die("サブコマンドが必要です: stats / dump / head / shuffle / quiet / rank / thin / phase");
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
            let consume = rest.iter().any(|a| a == "--consume");
            let parts: usize = arg_value(rest, "--parts")
                .map(|s| s.parse().unwrap_or_else(|_| die("--parts は整数")))
                .unwrap_or(1);
            // バケット幅はテスト用の隠しノブ。変えると割り付けが変わる
            let bucket_bytes: u64 = arg_value(rest, "--bucket-bytes")
                .map(|s| s.parse().unwrap_or_else(|_| die("--bucket-bytes は整数")))
                .unwrap_or(BUCKET_BYTES);
            shuffle(
                &inputs,
                &output.unwrap_or_else(|| die("--out が必要です")),
                seed,
                tmp.as_deref(),
                consume,
                parts,
                bucket_bytes,
            );
        }
        "quiet" => {
            let limit = arg_value(rest, "--limit")
                .map(|s| s.parse().unwrap_or(u64::MAX))
                .unwrap_or(u64::MAX);
            // 既定はADR-0136の採択構成（1手）。hmwr data quietの既定と
            // 揃える。ここが16のままだと、直接呼んだときだけ別条件の
            // データができる（2026-08-30の前処理事故の再発防止）
            let max_plies = arg_value(rest, "--max-plies")
                .map(|s| s.parse().unwrap_or(1))
                .unwrap_or(1);
            let hash_mb = arg_value(rest, "--hash")
                .map(|s| s.parse().unwrap_or(64))
                .unwrap_or(64);
            let eval = arg_value(rest, "--eval-file")
                .or_else(|| std::env::var("EVAL_FILE").ok())
                .unwrap_or_else(|| die("--eval-file か EVAL_FILE が必要です"));
            let mode = QuietMode {
                append: rest.iter().any(|a| a == "--append"),
                consume: rest.iter().any(|a| a == "--consume"),
                jobs: arg_value(rest, "--jobs")
                    .map(|v| v.parse().unwrap_or_else(|_| die("--jobs は整数")))
                    .unwrap_or(1),
            };
            quiet(
                &input.unwrap_or_else(|| die("--in が必要です")),
                &output.unwrap_or_else(|| die("--out が必要です")),
                limit,
                max_plies,
                hash_mb,
                &eval,
                mode,
            );
        }
        "rank" => {
            let limit = arg_value(rest, "--limit")
                .map(|s| s.parse().unwrap_or(u64::MAX))
                .unwrap_or(u64::MAX);
            let skip: u64 = arg_value(rest, "--skip")
                .map(|s| s.parse().unwrap_or(0))
                .unwrap_or(0);
            let hash_mb = arg_value(rest, "--hash")
                .map(|s| s.parse().unwrap_or(64))
                .unwrap_or(64);
            let eval = arg_value(rest, "--eval-file")
                .or_else(|| std::env::var("EVAL_FILE").ok())
                .unwrap_or_else(|| die("--eval-file か EVAL_FILE が必要です"));
            rank(
                &input.unwrap_or_else(|| die("--in が必要です")),
                &output.unwrap_or_else(|| die("--out が必要です")),
                limit,
                skip,
                hash_mb,
                &eval,
            );
        }
        "thin" => {
            let threshold: i32 = arg_value(rest, "--threshold")
                .map(|s| s.parse().unwrap_or_else(|_| die("--threshold は整数")))
                .unwrap_or(1318);
            let keep: f64 = arg_value(rest, "--keep")
                .map(|s| s.parse().unwrap_or_else(|_| die("--keep は0〜1の実数")))
                .unwrap_or(0.5);
            if !(0.0..=1.0).contains(&keep) {
                die("--keep は0〜1の実数");
            }
            let seed: u64 = arg_value(rest, "--seed")
                .map(|s| s.parse().unwrap_or(1))
                .unwrap_or(1);
            let group: usize = arg_value(rest, "--group")
                .map(|s| s.parse().unwrap_or_else(|_| die("--group は整数")))
                .unwrap_or(PSV_BYTES);
            thin(
                &input.unwrap_or_else(|| die("--in が必要です")),
                &output.unwrap_or_else(|| die("--out が必要です")),
                threshold,
                keep,
                seed,
                group,
            );
        }
        "phase" => {
            let limit = arg_value(rest, "--limit")
                .map(|s| s.parse().unwrap_or(u64::MAX))
                .unwrap_or(u64::MAX);
            let eval = arg_value(rest, "--eval-file")
                .or_else(|| std::env::var("EVAL_FILE").ok())
                .unwrap_or_else(|| die("--eval-file か EVAL_FILE が必要です"));
            phase(
                &input.unwrap_or_else(|| die("--in が必要です")),
                &output.unwrap_or_else(|| die("--out が必要です")),
                limit,
                &eval,
            );
        }
        other => die(&format!("不明なサブコマンド: {other}")),
    }
}
