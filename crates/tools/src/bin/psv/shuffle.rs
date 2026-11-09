//! 2パスのバケット法で全体をシャッフルするサブコマンド（shuffle）。

#[allow(clippy::wildcard_imports)]
use super::*;

/// 1バケットの目標サイズ。パス2でバケット1個をメモリに載せる。
pub(super) const BUCKET_BYTES: u64 = 2 << 30;
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

/// shuffleの走らせ方。
pub(super) struct ShuffleCfg {
    pub(super) seed: u64,
    /// 一時ファイルの置き場。省くと出力と同じディレクトリになる
    pub(super) tmp_dir: Option<String>,
    pub(super) consume: bool,
    pub(super) parts: usize,
    pub(super) bucket_bytes: u64,
    /// 入力の先頭から読む局面数
    pub(super) limit: u64,
}

/// 2パスのバケット法で全体をシャッフルする。
///
/// パス1で各レコードをランダムなバケットへ振り分け、パス2でバケット単位に
/// メモリ上でシャッフルして連結する。メモリ使用量はバケット1個分に収まる。
///
/// consumeは読み終えた入力ファイルから順に消し、ディスクのピークを
/// 入力1倍強に抑える。再取得できる生データにだけ使う。
/// partsが2以上なら、出力を `<出力名>.partNNN` のほぼ等分な連番へ分ける。
/// 分割してもレコードの割り付けはseedだけで決まり、連結すればparts=1と
/// 同じ並びになる。
///
/// limitは入力の先頭からこの局面数だけを読む。生データの先頭だけを元にする
/// ときに使う。上限で読み止めた入力は、consumeでも消さない。
pub(super) fn shuffle(inputs: &[&str], output: &str, cfg: ShuffleCfg) {
    let ShuffleCfg {
        seed,
        tmp_dir,
        consume,
        parts,
        bucket_bytes,
        limit,
    } = cfg;
    let tmp_dir = tmp_dir.as_deref();
    let bytes: u64 = inputs
        .iter()
        .map(|p| {
            std::fs::metadata(p)
                .unwrap_or_else(|e| die(&format!("開けません: {p}: {e}")))
                .len()
        })
        .sum();
    if !bytes.is_multiple_of(PSV_BYTES as u64) {
        die(&format!("入力サイズが40の倍数でない: {bytes}バイト"));
    }
    let total = bytes.min(limit.saturating_mul(PSV_BYTES as u64));
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
        let mut left = limit;
        for path in inputs {
            if left == 0 {
                break;
            }
            let mut r = open_reader(path);
            while left > 0 && r.read_exact(&mut buf).is_ok() {
                let b = (rng.next() % n_buckets as u64) as usize;
                writers[b]
                    .write_all(&buf)
                    .unwrap_or_else(|e| die(&format!("書き込み失敗: {e}")));
                left -= 1;
            }
            // 上限で止めた入力はまだ読み残しがあるので、consumeでも消さない
            if consume && left > 0 {
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
