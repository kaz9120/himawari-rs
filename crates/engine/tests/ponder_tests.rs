//! ponderの状態機械の結合テスト（ADR-0033）。
//!
//! go ponder中はbestmoveが出ないこと（2手指し防御）、
//! ponderhit/stopで正しく1回だけbestmoveが出ることを固定する。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use himawari_core::SFEN_STARTPOS;
use himawari_engine::{EngineOptions, Limits, ThreadPool};

type Lines = Arc<Mutex<Vec<String>>>;

fn make_pool() -> (ThreadPool, Lines) {
    let lines: Lines = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&lines);
    let pool = ThreadPool::new(
        16,
        1,
        None,
        Arc::new(move |s: &str| sink.lock().unwrap().push(s.to_string())),
    );
    (pool, lines)
}

fn bestmove_count(lines: &Lines) -> usize {
    lines
        .lock()
        .unwrap()
        .iter()
        .filter(|l| l.starts_with("bestmove"))
        .count()
}

/// info行のdepthを出力順に集める。
fn info_depths(lines: &Lines) -> Vec<u32> {
    lines
        .lock()
        .unwrap()
        .iter()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            (it.next()? == "info" && it.next()? == "depth")
                .then(|| it.next()?.parse::<u32>().ok())
                .flatten()
        })
        .collect()
}

fn max_depth(lines: &Lines) -> u32 {
    info_depths(lines).into_iter().max().unwrap_or(0)
}

fn wait_for_bestmove(lines: &Lines, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if bestmove_count(lines) > 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn pos(sfen: &str) -> himawari_core::Position {
    himawari_core::Position::from_sfen(sfen).unwrap()
}

/// go ponder → ponderhit → 探索を継続 → bestmoveが1回だけ出る。
#[test]
fn ponderhit_continues_and_emits_once() {
    let (pool, lines) = make_pool();
    let limits = Limits {
        movetime: 300,
        ..Limits::default()
    };
    let opts = EngineOptions {
        network_delay: 0,
        network_delay2: 0,
        ..EngineOptions::default()
    };
    pool.go_ponder(pos(SFEN_STARTPOS), limits, opts);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(bestmove_count(&lines), 0, "ponder中にbestmoveが出た");
    pool.ponderhit();
    assert!(
        wait_for_bestmove(&lines, Duration::from_secs(5)),
        "ponderhit後にbestmoveが出ない"
    );
    pool.wait_idle();
    assert_eq!(bestmove_count(&lines), 1);
    pool.quit();
}

/// ponderhitで探索を再起動しない（ADR-0109のG8）。再起動すると
/// 反復深化が深さ1からやり直しになるので、infoのdepthが巻き戻る。
#[test]
fn ponderhit_does_not_restart_iterative_deepening() {
    let (pool, lines) = make_pool();
    // ponder中は時間で止まらない。ponderhit後にmovetime超過で止まる
    let limits = Limits {
        movetime: 800,
        ..Limits::default()
    };
    let opts = EngineOptions {
        network_delay: 0,
        network_delay2: 0,
        ..EngineOptions::default()
    };
    pool.go_ponder(pos(SFEN_STARTPOS), limits, opts);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(bestmove_count(&lines), 0, "ponder中にbestmoveが出た");
    let depth_before = max_depth(&lines);
    assert!(depth_before > 0, "ponder中にinfoが出ていない");
    pool.ponderhit();
    assert!(
        wait_for_bestmove(&lines, Duration::from_secs(5)),
        "ponderhit後にbestmoveが出ない"
    );
    pool.wait_idle();
    // 深さが巻き戻っていないこと。再起動なら1へ戻る
    let depths = info_depths(&lines);
    assert!(
        depths.windows(2).all(|w| w[0] <= w[1]),
        "反復深化の深さが巻き戻った（再起動している）: {depths:?}"
    );
    pool.quit();
}

/// go ponder → stop → 即bestmove（GUIは破棄する）。
#[test]
fn stop_during_ponder_emits_bestmove() {
    let (pool, lines) = make_pool();
    let limits = Limits {
        movetime: 300,
        ..Limits::default()
    };
    pool.go_ponder(pos(SFEN_STARTPOS), limits, EngineOptions::default());
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(bestmove_count(&lines), 0);
    pool.stop();
    assert!(
        wait_for_bestmove(&lines, Duration::from_secs(5)),
        "stop後にbestmoveが出ない"
    );
    pool.wait_idle();
    assert_eq!(bestmove_count(&lines), 1);
    pool.quit();
}

/// ponder中に探索が終わって（詰み発見）もbestmoveを保留し、
/// ponderhitで即座に出す。
#[test]
fn mate_found_in_ponder_is_held_until_ponderhit() {
    let (pool, lines) = make_pool();
    // 頭金の1手詰め局面。深さ上限まで一瞬で読み切って保留に入る
    let limits = Limits {
        depth: 5,
        ..Limits::default()
    };
    pool.go_ponder(
        pos("4k4/9/4P4/9/9/9/9/9/4K4 b G 1"),
        limits,
        EngineOptions::default(),
    );
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        bestmove_count(&lines),
        0,
        "ponder中に探索が終わってもbestmoveは保留されるべき"
    );
    pool.ponderhit();
    assert!(
        wait_for_bestmove(&lines, Duration::from_secs(5)),
        "保留解除でbestmoveが出ない"
    );
    pool.wait_idle();
    assert_eq!(bestmove_count(&lines), 1);
    pool.quit();
}
