//! 同じ盤面の2回目以降を捨てるサブコマンド（dedup）。

#[allow(clippy::wildcard_imports)]
use super::*;

/// 同じ盤面の2回目以降を捨て、初出の順に残す。
///
/// 盤面はpacked sfenの32バイト（盤・手番・持ち駒）で比べ、手数・教師信号は
/// 見ない。比べるのは32バイトの64ビットハッシュで、1億件の出力で誤って
/// 同一視する組の期待値は約3×10^-4である。集合はメモリに持つので、出力
/// 1億件で約1.5GBを使う。
pub(super) fn dedup(input: &str, output: &str, count: u64, limit: u64) {
    let mut r = open_reader(input);
    let mut w = BufWriter::new(
        std::fs::File::create(output).unwrap_or_else(|e| die(&format!("作成できません: {e}"))),
    );
    let mut seen: std::collections::HashSet<u64> = std::collections::HashSet::new();
    let mut buf = [0u8; PSV_BYTES];
    let (mut total, mut written) = (0u64, 0u64);
    while written < count && total < limit && r.read_exact(&mut buf).is_ok() {
        total += 1;
        if !seen.insert(board_hash(&buf[..32])) {
            continue;
        }
        w.write_all(&buf)
            .unwrap_or_else(|e| die(&format!("書き込み失敗: {e}")));
        written += 1;
    }
    w.flush().unwrap();
    println!(
        "入力{total}件のうち重複{}件を捨て、{written}件を書き出しました",
        total - written
    );
}

/// packed sfenの32バイトを64ビットへ畳む。4語をそれぞれ別の奇数で掛けて
/// 混ぜ、最後にsplitmix64の終段で拡散する
fn board_hash(sfen: &[u8]) -> u64 {
    const K: [u64; 4] = [
        0x9E37_79B9_7F4A_7C15,
        0xC2B2_AE3D_27D4_EB4F,
        0x1656_67B1_9E37_79F9,
        0x85EB_CA77_C2B2_AE63,
    ];
    let mut h = 0u64;
    let (words, _) = sfen.as_chunks::<8>();
    for (i, c) in words.iter().enumerate() {
        let x = u64::from_le_bytes(*c);
        h = (h ^ x.wrapping_mul(K[i])).rotate_left(27);
    }
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^ (h >> 31)
}
