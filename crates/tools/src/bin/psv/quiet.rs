//! 教師局面をqsearchのPV葉へ置き換えるサブコマンド（quiet）。

#[allow(clippy::wildcard_imports)]
use super::*;

/// 教師局面をqsearchのPV葉へ置き換える。
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
pub(super) fn quiet_worker(
    net: &Arc<himawari_engine::nnue::NnueNetwork>,
    hash_mb: usize,
) -> Worker {
    let shared = Arc::new(Shared::new(hash_mb));
    let limits = Limits::default();
    let start_pos = himawari_core::Position::from_sfen(himawari_core::SFEN_STARTPOS)
        .unwrap_or_else(|e| die(&format!("初期局面を作れません: {e:?}")));
    Worker::standalone(start_pos, shared, limits, Evaluator::nnue(Arc::clone(net)))
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
        match pack(&worker.pos.to_position()) {
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
pub(super) struct QuietMode {
    /// 出力へ追記する（既定は新規作成）
    pub(super) append: bool,
    /// 完了後に入力を消す
    pub(super) consume: bool,
    /// 並列数。2以上でチャンク並列になる（jobs固定で決定論）
    pub(super) jobs: usize,
}

pub(super) fn quiet(
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
