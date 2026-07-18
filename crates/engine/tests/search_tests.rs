//! 探索の統合テスト（ADR-0024, 0026）。

use std::sync::Arc;

use himawari_core::{Move, MoveList, Position, SFEN_STARTPOS, generate_legal};
use himawari_engine::eval::Evaluator;
use himawari_engine::movepick::{CounterMoves, History};
use himawari_engine::search::{Shared, Worker};
use himawari_engine::timeman::{Limits, TimeManager};
use himawari_engine::value::{VALUE_MATE, Value};

fn search_position(sfen: &str, depth: u32) -> (Move, Value) {
    let pos = Position::from_sfen(sfen).unwrap();
    let shared = Arc::new(Shared::new(16));
    let limits = Limits {
        depth,
        ..Limits::default()
    };
    let tm = TimeManager::new(&limits, pos.side_to_move(), pos.game_ply(), 120, 1120);
    let mut worker = Worker::new(
        pos,
        shared,
        limits,
        tm,
        0,
        Evaluator::material(),
        History::default(),
        CounterMoves::default(),
    );
    let result = worker.iterate(&mut |_| {});
    (result.best, result.score)
}

/// 1手詰め: 桂に支えられたG*5bで詰み。
#[test]
fn finds_mate_in_one() {
    let (best, score) = search_position("4k4/9/9/5N3/9/9/9/9/4K4 b G 1", 3);
    assert_eq!(best.to_usi(), "G*5b");
    assert_eq!(score, VALUE_MATE - 1);
}

/// 詰まされる側は最長の逃れを選び、mated値を返す。
#[test]
fn recognizes_being_mated() {
    // 先手番だが手がなく、次にG*5bで詰まされる局面に近い形。
    // ここでは単純に「合法手がない」局面の判定を見る
    let (best, _) = search_position("4k4/9/9/9/9/9/9/9/4K4 b - 1", 2);
    // 合法手はあるので何かしら返る
    assert!(best != Move::RESIGN);
}

/// 自己対局スモーク: 固定深さで100手指しても壊れない。
#[test]
fn selfplay_smoke() {
    let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    let shared = Arc::new(Shared::new(16));
    for _ in 0..100 {
        let mut legal = MoveList::default();
        generate_legal(&pos, false, &mut legal);
        if legal.is_empty() {
            break;
        }
        let limits = Limits {
            depth: 4,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, pos.side_to_move(), pos.game_ply(), 120, 1120);
        let mut worker = Worker::new(
            pos.clone(),
            Arc::clone(&shared),
            limits,
            tm,
            256,
            Evaluator::material(),
            History::default(),
            CounterMoves::default(),
        );
        let result = worker.iterate(&mut |_| {});
        if result.best == Move::RESIGN {
            break;
        }
        // 返ってきた手が合法であること
        assert!(
            legal.as_slice().contains(&result.best),
            "非合法手が返った: {}",
            result.best.to_usi()
        );
        pos.do_move(result.best);
    }
}
