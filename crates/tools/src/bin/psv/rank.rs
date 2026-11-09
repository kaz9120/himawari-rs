//! 兄弟局面の葉の群を作るサブコマンド（rank）。

#[allow(clippy::wildcard_imports)]
use super::*;
use crate::quiet::quiet_worker;

/// 兄弟局面の葉の群を作る。
///
/// 教師の最善手の子と、他の合法手から引いた負例2手の子を、それぞれ
/// qsearchのPV葉まで進めて 正例・負例・負例 の順に書く。1群は
/// 40バイト×3。予備バイト（b[39]）に親からの手数の偶奇を入れ、学習側は
/// これで葉の評価値を親視点の符号へ戻す。
///
/// 乱数はレコードの通し番号（--skipを含む絶対位置）から決定論で引く。
/// 分割して並列に走らせても、結合結果は1本で走らせた場合と一致する。
pub(super) fn rank(input: &str, output: &str, limit: u64, skip: u64, hash_mb: usize, eval: &str) {
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
            let Ok(packed) = pack(&worker.pos.to_position()) else {
                break;
            };
            let mut child = rec;
            child.sfen = packed;
            child.move16 = 0;
            child.game_ply = rec.game_ply.saturating_add(plies as u16);
            let mut bytes = child.to_bytes();
            // 予備バイトへ親からの手数の偶奇を入れる
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
