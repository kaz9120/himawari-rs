//! 将棋の盤面表現・指し手生成・SFEN入出力を提供するクレート。
//!
//! 探索・評価には依存しない。perft、学習器、対局マネージャは
//! このクレートだけに依存して動作する（ADR-0002）。

pub mod attacks;
pub mod bitboard;
pub mod bonapiece;
pub mod effect;
pub mod hand;
pub mod movegen;
pub mod moves;
pub mod packed_sfen;
pub mod piece;
pub mod position;
pub mod types;
pub mod zobrist;

pub use bitboard::Bitboard;
pub use hand::Hand;
pub use movegen::{GenType, generate, generate_legal, perft, perft_slow};
pub use moves::{Move, Move16, MoveList};
pub use piece::{Piece, PieceType};
pub use position::{
    DirtyPiece, Position, Repetition, SFEN_STARTPOS, SfenError, StateInfo, piece_value,
};
pub use types::{Color, File, Rank, Square};
