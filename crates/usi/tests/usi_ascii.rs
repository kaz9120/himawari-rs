//! USIへ出す行がASCIIだけになることを固定する（issue #745）。
//!
//! Windowsの将棋所はエンジンの出力をcp932として読む。UTF-8の日本語を
//! 出すと化けるので、日本語のパスや読めないファイルを渡した異常系も含めて、
//! `usi`・`setoption`・`isready` への応答を実際のバイナリで確かめる。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// エンジンへ入力を流し切り、終了まで待つ。
fn run_engine(input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_himawari"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn himawari");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input)
        .expect("write stdin");
    child.wait_with_output().expect("wait himawari")
}

/// 日本語を含む作業ディレクトリを作る。
fn work_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("himawari-将棋-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create_dir_all");
    dir
}

fn assert_ascii(stdout: &[u8]) -> String {
    for (i, line) in stdout.split(|&b| b == b'\n').enumerate() {
        assert!(
            line.is_ascii(),
            "{}行目がASCIIでない: {}",
            i + 1,
            String::from_utf8_lossy(line)
        );
    }
    String::from_utf8(stdout.to_vec()).expect("ascii is utf-8")
}

fn setoption(name: &str, value: &Path) -> String {
    format!("setoption name {name} value {}\n", value.display())
}

#[test]
fn responses_are_ascii_even_with_japanese_paths() {
    let dir = work_dir("ascii");
    let book = dir.join("定跡.db");
    std::fs::write(
        &book,
        "#YANEURAOU-DB2016 1.00\n\
         sfen lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1\n\
         7g7f none 0 32 1\n",
    )
    .expect("write book");

    let mut input = Vec::new();
    input.extend_from_slice(b"usi\n");
    input.extend_from_slice(setoption("BookFile", &book).as_bytes());
    input.extend_from_slice(setoption("BookFile", &dir.join("無い.db")).as_bytes());
    input.extend_from_slice(setoption("DebugLogFile", &dir.join("ログ.txt")).as_bytes());
    input.extend_from_slice(b"setoption name DebugLogFile value <empty>\n");
    input.extend_from_slice(
        setoption("DebugLogFile", &dir.join("無いフォルダ").join("ログ.txt")).as_bytes(),
    );
    // cp932の「将棋」。UTF-8として不正な行でも、読み取りを止めない
    input.extend_from_slice(b"setoption name EvalFile value ");
    input.extend_from_slice(dir.to_string_lossy().as_bytes());
    input.extend_from_slice(b"/\x8f\xab\x8a\xfb.hmwr\n");
    input.extend_from_slice(b"isready\n");

    let out = run_engine(&input);
    let stdout = assert_ascii(&out.stdout);
    assert!(stdout.contains("usiok"), "{stdout}");
    assert!(stdout.contains("BookFile loaded: "), "{stdout}");
    assert!(stdout.contains("(1 positions)"), "{stdout}");
    assert!(stdout.contains("cannot read BookFile"), "{stdout}");
    assert!(stdout.contains("debug log -> "), "{stdout}");
    assert!(stdout.contains("debug log off"), "{stdout}");
    assert!(stdout.contains("cannot open "), "{stdout}");
    // 非UTF-8の行のあとのisreadyまで届き、EvalFileを開けずに終わる
    assert!(stdout.contains("cannot open EvalFile"), "{stdout}");
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_eval_file_error_is_ascii() {
    let out = run_engine(b"usi\nisready\n");
    let stdout = assert_ascii(&out.stdout);
    assert!(stdout.contains("EvalFile is not set"), "{stdout}");
    assert_eq!(out.status.code(), Some(1), "{stdout}");
}
