//! 教師データの中身を見る・切り出すサブコマンド（stats・dump・head）。

#[allow(clippy::wildcard_imports)]
use super::*;

pub(super) fn stats(input: &str, limit: Option<u64>) {
    let mut r = open_reader(input);
    let mut buf = [0u8; PSV_BYTES];
    let mut n = 0u64;
    let mut decode_err = 0u64;
    let mut score_sum = 0i64;
    let (mut score_min, mut score_max) = (i16::MAX, i16::MIN);
    let mut results = [0u64; 3]; // 負け・引き分け・勝ち
    let mut ply_max = 0u16;
    let mut hist = [0u64; 9]; // |score|の桁別ヒストグラム
    let mut wp_hist = [0u64; 100]; // 勝率1%刻み（非詰み）。教師の勝率の偏りを見る
    let (mut mate_win, mut mate_lose) = (0u64, 0u64);
    // 盤上の駒数（玉2枚を含む）。出力バケットの境界を決める材料
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
        // 出力バケットの境界は分布の四分位から決める
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

pub(super) fn dump(input: &str, limit: u64) {
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

pub(super) fn head(input: &str, output: &str, count: u64, skip: u64) {
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

/// 詰みスコアの下限。statsはこの絶対値以上を詰みとして別に数える。
const MATE_ABS: i32 = 29000;
