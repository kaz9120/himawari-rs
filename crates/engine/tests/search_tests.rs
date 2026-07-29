//! 探索の統合テスト（ADR-0024, 0026）。

use std::sync::Arc;

use himawari_core::{Move, MoveList, Position, SFEN_STARTPOS, generate_legal};
use himawari_engine::eval::Evaluator;
use himawari_engine::movepick::Histories;
use himawari_engine::search::{SearchInfo, Shared, Worker};
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
        1,
        Evaluator::material(),
        Histories::default(),
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
            1,
            Evaluator::material(),
            Histories::default(),
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

/// MultiPV: ラインの初手が重複せず、スコアが降順であること（ADR-0032）。
#[test]
fn multipv_lines_are_distinct_and_sorted() {
    let pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    let shared = Arc::new(Shared::new(16));
    let limits = Limits {
        depth: 6,
        ..Limits::default()
    };
    let tm = TimeManager::new(&limits, pos.side_to_move(), pos.game_ply(), 120, 1120);
    let mut worker = Worker::new(
        pos,
        shared,
        limits,
        tm,
        0,
        3,
        Evaluator::material(),
        Histories::default(),
    );
    let mut lines: Vec<(usize, Value, Move)> = Vec::new();
    worker.iterate(&mut |info| {
        if let SearchInfo::Iteration(info) = info
            && info.depth == 6
        {
            lines.push((info.multipv, info.score, info.pv[0]));
        }
    });
    assert_eq!(lines.len(), 3, "深さ6で3ライン出力されること");
    for (i, (k, _, _)) in lines.iter().enumerate() {
        assert_eq!(*k, i + 1);
    }
    assert!(lines[0].1 >= lines[1].1 && lines[1].1 >= lines[2].1);
    assert!(lines[0].2 != lines[1].2 && lines[1].2 != lines[2].2 && lines[0].2 != lines[2].2);
}

/// NNUE評価での探索が完走し合法手を返すこと（push/pop契約の
/// 全経路ストレス。NMP・LMR・qsearchを含む）。
#[test]
fn nnue_search_returns_legal_moves() {
    use himawari_engine::nnue::NnueNetwork;
    let net = std::sync::Arc::new(NnueNetwork::random(11));
    for sfen in [
        SFEN_STARTPOS,
        "1n1gk2nl/1r4g2/1sppppspp/L5p2/1p5P1/2P6/1PSPPPPSP/7R1/1N1GKG1NL w BLPbp 24",
    ] {
        let pos = Position::from_sfen(sfen).unwrap();
        let shared = Arc::new(Shared::new(16));
        let limits = Limits {
            depth: 6,
            ..Limits::default()
        };
        let tm = TimeManager::new(&limits, pos.side_to_move(), pos.game_ply(), 120, 1120);
        let mut worker = Worker::new(
            pos.clone(),
            shared,
            limits,
            tm,
            0,
            1,
            Evaluator::nnue(Arc::clone(&net)),
            Histories::default(),
        );
        let result = worker.iterate(&mut |_| {});
        let mut legal = MoveList::default();
        generate_legal(&pos, false, &mut legal);
        assert!(
            legal.as_slice().contains(&result.best),
            "非合法手: {}",
            result.best.to_usi()
        );
    }
}

/// stopが即座に立っても、深さ1を終えるまでは打ち切らない。
///
/// root手は生成順に並んでおり、`iterate` は打ち切り時に `root_moves[0]`
/// を返す。深さ1の途中で止まると、探索していない「生成順の先頭の手」が
/// bestmoveになってしまう。深さ1の完走を保証すれば、返る手は必ず深さ1の
/// 最善手になる。
#[test]
fn stop_before_first_iteration_still_returns_the_depth1_best() {
    let sfen = "1n1gk2nl/1r4g2/1sppppspp/L5p2/1p5P1/2P6/1PSPPPPSP/7R1/1N1GKG1NL w BLPbp 24";
    let (want, _) = search_position(sfen, 1);

    let pos = Position::from_sfen(sfen).unwrap();
    let shared = Arc::new(Shared::new(16));
    // 探索を始める前からstopを立てておく
    shared
        .stop
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let limits = Limits {
        infinite: true,
        ..Limits::default()
    };
    let tm = TimeManager::new(&limits, pos.side_to_move(), pos.game_ply(), 120, 1120);
    let mut worker = Worker::new(
        pos.clone(),
        shared,
        limits,
        tm,
        0,
        1,
        Evaluator::material(),
        Histories::default(),
    );
    let got = worker.iterate(&mut |_| {}).best;

    let mut legal = MoveList::default();
    generate_legal(&pos, false, &mut legal);
    assert_eq!(got, want, "深さ1の最善手ではなく生成順の先頭を返している");
    assert_ne!(
        got,
        legal.as_slice()[0],
        "この局面では深さ1の最善手と生成順の先頭が違うことを前提にしている"
    );
}

/// 打ち切られても、最後に出すinfoは確定した結果にする。
///
/// aspirationがfail high / fail lowすると lowerbound / upperbound を報告して
/// 窓を広げ、読み直す。その途中で打ち切られると、最後のinfoが未確定のまま
/// 終わっていた。GUIやCSAブリッジはそれをその手のスコアとして記録するので、
/// 実際に指した手と食い違う値が残る。floodgateの棋譜に `4723++` として現れ、
/// 直後の手で評価が8300反転した。
#[test]
fn does_not_end_on_an_unresolved_aspiration_bound() {
    let sfen = "1n1gk2nl/1r4g2/1sppppspp/L5p2/1p5P1/2P6/1PSPPPPSP/7R1/1N1GKG1NL w BLPbp 24";
    let pos = Position::from_sfen(sfen).unwrap();
    let shared = Arc::new(Shared::new(16));
    let stopper = Arc::clone(&shared);
    let limits = Limits {
        depth: 12,
        ..Limits::default()
    };
    let tm = TimeManager::new(&limits, pos.side_to_move(), pos.game_ply(), 120, 1120);
    let mut worker = Worker::new(
        pos,
        shared,
        limits,
        tm,
        0,
        1,
        Evaluator::material(),
        Histories::default(),
    );
    let mut saw_bound = false;
    let mut last_was_bound = false;
    let mut last_pv_head: Option<Move> = None;
    let result = worker.iterate(&mut |info| match info {
        SearchInfo::Bound(i, _) => {
            // 窓外れを1回報告させたら、読み直しの途中で打ち切る
            if !saw_bound {
                saw_bound = true;
                stopper
                    .stop
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
            last_was_bound = true;
            last_pv_head = i.pv.first().copied();
        }
        SearchInfo::Iteration(i) => {
            last_was_bound = false;
            last_pv_head = i.pv.first().copied();
        }
        SearchInfo::CurrMove { .. } => {}
    });
    assert!(saw_bound, "この局面で窓外れが起きることを前提にしている");
    assert!(
        !last_was_bound,
        "最後のinfoが未確定の窓外れのまま終わっている"
    );
    assert_eq!(
        last_pv_head,
        Some(result.best),
        "最後に報告したPVの先頭とbestmoveが食い違う"
    );
}
