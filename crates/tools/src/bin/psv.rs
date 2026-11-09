//! PackedSfenValue教師データの前処理ツール。
//!
//! 使い方:
//!   psv stats   --in file [--limit N]          統計表示（局面数・score分布・勝敗・勝率帯）
//!   psv dump    --in file [--limit N]          SFENと教師信号を1行ずつ表示
//!   psv head    --in file --out file --count N [--skip M]   部分抽出
//!   psv shuffle --in file[,file...] --out file [--seed N] [--tmp DIR]
//!               [--consume] [--parts N] [--limit N]
//!                                              全体シャッフル。--consumeは読み終えた
//!                                              入力を消してピークを約1倍に抑える。
//!                                              --parts Nは出力を.partNNNへ分割する。
//!                                              --limit Nは入力の先頭N局面だけを読む
//!   psv quiet   --in file --out file [--limit N] [--max-plies N（既定1）] [--hash MB]
//!               [--append] [--consume] [--jobs N]  qsearchのPV葉へ置き換える。
//!                                              --appendは出力へ追記、--consumeは完了後に
//!                                              入力を消す（分割入力の逐次処理用）。--jobsの
//!                                              並列出力はjobs固定で決定論（逐次とは不一致）
//!   psv rank    --in file --out file [--limit N] [--skip N] [--hash MB]
//!                                              兄弟局面の葉の群を作る
//!   psv dedup   --in file --out file [--count N] [--limit N]
//!                                              同じ盤面の2回目以降を捨て、初出の順に残す。
//!                                              --count Nで書く件数を、--limit Nで
//!                                              読む件数を打ち切る
//!
//! shuffleは2パスのバケット法で動く。メモリ使用量は
//! バケット1個分（2GB）に収まるため、入力サイズの制限はない。
//! 一時ファイルは出力と同じディレクトリに作り、使い終わり次第消す。

use std::io::{BufWriter, Read, Seek, SeekFrom, Write};

use std::sync::Arc;

use himawari_core::packed_sfen::{PSV_BYTES, PackedSfenValue, pack, unpack, unpack_sfen};
use himawari_engine::eval::Evaluator;
use himawari_engine::search::{Shared, Worker};
use himawari_engine::timeman::Limits;

// バイナリのルートなので、サブモジュールはsrc/bin/psv/の下に置いてpathで指す。
// Cargo.tomlのpathを変えずに済む
#[path = "psv/dedup.rs"]
mod dedup;
#[path = "psv/quiet.rs"]
mod quiet;
#[path = "psv/rank.rs"]
mod rank;
#[path = "psv/shuffle.rs"]
mod shuffle;
#[path = "psv/stats.rs"]
mod stats;

use dedup::dedup;
use quiet::{QuietMode, quiet};
use rank::rank;
use shuffle::{BUCKET_BYTES, ShuffleCfg, shuffle};
use stats::{dump, head, stats};

fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1)
}

/// 件数の上限を読む。省くと上限なし（u64::MAX）になる
fn limit_arg(args: &[String], key: &str) -> u64 {
    arg_value(args, key)
        .map(|s| s.parse().unwrap_or_else(|_| die(&format!("{key} は整数"))))
        .unwrap_or(u64::MAX)
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.iter().position(|a| a == key).map(|i| {
        args.get(i + 1)
            .unwrap_or_else(|| die(&format!("{key} に値がありません")))
            .clone()
    })
}

/// 評価関数のパスを引数か環境変数 `EVAL_FILE` から決める。フォールバックは
/// クレート共通の `eval_file()` が持ち、存在確認までそこで行う。`psv` は
/// 引数も評価関数も文字列で扱うため、受け渡しの変換だけをここに置く。
/// フラグ名は `arg_value` と同じく呼び出し側が渡す。hmwrの表との食い違いを
/// 見るテストが、サブコマンドの節に書かれたフラグ名を読むためである
/// （tests/test_dataops.py の test_every_flag_exists_in_psv）。
fn eval_arg(args: &[String], key: &str) -> String {
    let explicit = arg_value(args, key).map(std::path::PathBuf::from);
    let path = himawari_tools::eval_file(explicit).unwrap_or_else(|e| die(&e.to_string()));
    himawari_tools::path_str(&path)
        .unwrap_or_else(|e| die(&e.to_string()))
        .to_string()
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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        die("サブコマンドが必要です: stats / dump / head / shuffle / quiet / rank / dedup");
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
            let limit = arg_value(rest, "--limit")
                .map(|s| s.parse().unwrap_or_else(|_| die("--limit は整数")))
                .unwrap_or(u64::MAX);
            shuffle(
                &inputs,
                &output.unwrap_or_else(|| die("--out が必要です")),
                ShuffleCfg {
                    seed,
                    tmp_dir: tmp,
                    consume,
                    parts,
                    bucket_bytes,
                    limit,
                },
            );
        }
        "quiet" => {
            let limit = arg_value(rest, "--limit")
                .map(|s| s.parse().unwrap_or(u64::MAX))
                .unwrap_or(u64::MAX);
            // 既定は採択した構成の1手。hmwr data quietの既定と
            // 揃える。ここが16のままだと、直接呼んだときだけ別条件の
            // データができる（2026-08-30の前処理事故の再発防止）
            let max_plies = arg_value(rest, "--max-plies")
                .map(|s| s.parse().unwrap_or(1))
                .unwrap_or(1);
            let hash_mb = arg_value(rest, "--hash")
                .map(|s| s.parse().unwrap_or(64))
                .unwrap_or(64);
            let eval = eval_arg(rest, "--eval-file");
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
            let eval = eval_arg(rest, "--eval-file");
            rank(
                &input.unwrap_or_else(|| die("--in が必要です")),
                &output.unwrap_or_else(|| die("--out が必要です")),
                limit,
                skip,
                hash_mb,
                &eval,
            );
        }
        "dedup" => {
            dedup(
                &input.unwrap_or_else(|| die("--in が必要です")),
                &output.unwrap_or_else(|| die("--out が必要です")),
                limit_arg(rest, "--count"),
                limit_arg(rest, "--limit"),
            );
        }
        other => die(&format!("不明なサブコマンド: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{arg_value, eval_arg};

    fn tmp_file(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("psv-test-{}-{tag}.psv", std::process::id()))
    }

    /// `--eval-file` の明示指定は、共有ヘルパーを通しても旧実装の
    /// 2行フォールバックと同じ文字列になる（issue #714）。旧実装が
    /// 先へ進めたのはファイルがある場合だけなので、その条件で比べる。
    #[test]
    fn an_explicit_eval_file_resolves_as_the_inlined_fallback_did() {
        let path = tmp_file("eval");
        std::fs::write(&path, b"dummy").expect("一時ファイルを作れない");
        let given = path.to_str().expect("UTF-8のパス").to_string();
        let args = vec!["--eval-file".to_string(), given.clone()];
        let inlined = arg_value(&args, "--eval-file")
            .or_else(|| std::env::var("EVAL_FILE").ok())
            .expect("明示指定があるので値が取れる");
        assert_eq!(eval_arg(&args, "--eval-file"), inlined);
        assert_eq!(eval_arg(&args, "--eval-file"), given);
        std::fs::remove_file(&path).ok();
    }
}
