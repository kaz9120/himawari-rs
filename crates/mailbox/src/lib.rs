//! 配列盤と利きの配列による盤面表現の研究用試作（ADR-0241）。
//!
//! Bitboardを持たない。盤は壁付きの1次元配列で、升ごとに手番別の利き数と
//! 飛び利きの方向ビットを持ち、do_move/undo_moveで差分更新する。
//! 指し手生成は駒番号の一覧から引き、王手・ピン・玉の移動の合法性は利きの配列で決める。
//! 既存の `himawari-core` には依存しない（照合のテストだけが使う）。

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
