//! 第1層へ入る活性の非ゼロパターンを集める（ADR-0168）。
//!
//! `--features actdump` を付けたビルドにだけ入る調査用の仕組みで、
//! 通常のビルドには一切コードが残らない。集めるのは値ではなく
//! 「どの次元が非ゼロだったか」のビットマスクで、1サンプル
//! `CONCAT / 8` バイトになる。
//!
//! 出力先は環境変数 `HIMAWARI_ACT_OUT`（既定 `data/profile/act.bin`）。
//! 目標数に達した時点で書き出し、以降は何もしない。

use std::sync::Mutex;

use crate::nnue::CONCAT;

/// 何回の評価につき1つ記録するか。連続する評価は同じ枝の局面で
/// 似通うため、間隔を空けて相関を薄める。
const STRIDE: usize = 251;

/// 集めるサンプル数。
const TARGET: usize = 12_500;

/// 1サンプルのバイト数。
const BYTES: usize = CONCAT / 8;

struct State {
    seen: usize,
    buf: Vec<u8>,
    done: bool,
}

static STATE: Mutex<State> = Mutex::new(State {
    seen: 0,
    buf: Vec::new(),
    done: false,
});

/// 活性を1つ受け取る。`NnueState::evaluate` が第1層へ渡す直前に呼ぶ。
pub fn record(concat: &[u8; CONCAT]) {
    let mut s = match STATE.lock() {
        Ok(s) => s,
        Err(e) => e.into_inner(),
    };
    if s.done {
        return;
    }
    s.seen += 1;
    if s.seen % STRIDE != 0 {
        return;
    }
    let mut bits = [0u8; BYTES];
    for (i, &v) in concat.iter().enumerate() {
        if v != 0 {
            bits[i / 8] |= 1 << (i % 8);
        }
    }
    s.buf.extend_from_slice(&bits);
    if s.buf.len() >= TARGET * BYTES {
        let path = std::env::var("HIMAWARI_ACT_OUT")
            .unwrap_or_else(|_| "data/profile/act.bin".to_string());
        match std::fs::write(&path, &s.buf) {
            Ok(()) => eprintln!("info string 活性ダンプ: {TARGET}サンプルを{path}へ書いた"),
            Err(e) => eprintln!("info string 活性ダンプの書き出しに失敗した: {e}"),
        }
        s.done = true;
    }
}
