//! 探索・置換表・評価・時間管理（P2のADR群に基づく）。

pub mod eval;
pub mod movepick;
pub mod search;
pub mod thread;
pub mod timeman;
pub mod tt;
pub mod value;

pub use eval::Evaluator;
pub use search::{IterInfo, SearchResult, Shared, Worker};
pub use thread::{EngineOptions, ThreadPool};
pub use timeman::{Limits, TimeManager};
pub use tt::{Bound, Tt, TtData};
pub use value::{MAX_PLY, VALUE_INFINITE, VALUE_MATE, Value, mate_in, mated_in};
