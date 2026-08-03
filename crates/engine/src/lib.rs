//! 探索・置換表・評価・時間管理（P2のADR群に基づく）。
#![feature(portable_simd)]

pub mod eval;
pub mod mate;
pub mod movepick;
pub mod nnue;
pub mod nnue_acc;
pub mod nnue_compat;
pub mod nnue_io;
pub mod nnue_simd;
pub mod posgen;
pub mod search;
pub mod thread;
pub mod timeman;
pub mod tt;
pub mod value;

pub use eval::Evaluator;
pub use search::{IterInfo, ScoreBound, SearchInfo, SearchResult, Shared, Worker};
pub use thread::{EngineOptions, ThreadPool};
pub use timeman::{Limits, TimeManager, TimeOptions};
pub use tt::{Bound, Tt, TtData};
pub use value::{MAX_PLY, VALUE_INFINITE, VALUE_MATE, Value, mate_in, mated_in};
