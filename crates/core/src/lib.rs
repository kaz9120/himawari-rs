//! 将棋の盤面表現・指し手生成・SFEN入出力を提供するクレート。
//!
//! 探索・評価には依存しない。perft、学習器、対局マネージャは
//! このクレートだけに依存して動作する（ADR-0002）。

pub mod attacks;
pub mod bitboard;
pub mod piece;
pub mod types;

pub use bitboard::Bitboard;
pub use piece::{Piece, PieceType};
pub use types::{Color, File, Rank, Square};
