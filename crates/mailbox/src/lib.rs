//! 配列盤と利きの配列による盤面表現（ADR-0241）。探索の盤の既定である。
//!
//! Bitboardを持たない。盤は壁付きの1次元配列で、升ごとに手番別の利き数と
//! 飛び利きの方向ビットを持ち、do_move/undo_moveで差分更新する。
//! 指し手生成は駒のいる升の集合（81升のu128）から引き、王手・ピン・玉の移動の合法性は利きの配列で決める。
//! 既存の `himawari-core` には依存しない（照合のテストだけが使う）。

#![feature(portable_simd)]

/// 生成の並びがcoreの `generate` と同じか（ADR-0241の第16回）。ビルド時の環境変数
/// `HIMAWARI_MB_ORDER` で決まる（build.rs）。偽の構成では、照合は手の集合で比べる。
pub const ORDER_MATCHES_CORE: bool = !cfg!(mb_order_type);

pub mod board;
pub mod check;
pub mod mate;
pub mod movegen;
pub mod moves;
pub mod position;
pub mod rules;
pub mod zobrist;

pub use board::{Color, SFEN_STARTPOS};
pub use mate::MateCount;
pub use movegen::{GenType, MoveList, MoveSink, perft};
pub use moves::Move;
pub use position::{DirtyPiece, Position, StateInfo};
pub use rules::{HistoryKey, Repetition};
