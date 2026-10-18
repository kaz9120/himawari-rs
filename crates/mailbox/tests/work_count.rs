//! 進め戻し1往復で、利きの表へ書く仕事の量を数える（ADR-0241の第8回）。時間は測らない。
//!
//! `mbbench` と同じ局面・同じ歩き方（全幅で深さ2、葉では駒取りだけ1手）で、進める手ごとに
//! 差分更新が触る量を、指す前の盤から見積もる。
//!
//! - 短い利き: 動いた駒・置いた駒・取られた駒の短い利きの数（1つが利き数1バイトの読み書き）
//! - 飛び利き: 線の本数と、線が触る升の数（1升が利き数と方向ビットの2バイトの読み書き）
//!
//! 滑り（自分の線に沿う飛び駒の移動）の付け替えは、駒の利きを全部引き直すものとして数える。
//! 戻しは進めと同じ量とみなす。どれも上限寄りの見積もりである。
//!
//! 1手詰めは、王手でないノードごとに `mate_1ply` の列挙と判定の量を数える（第10回）。
//!
//! ```text
//! cargo test --release -p himawari-mailbox --test work_count -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use himawari_mailbox::board::{DELTA, EMPTY, MOVES, WALL};
use himawari_mailbox::{Color, MateCount, Move, MoveList, Position};
use himawari_tools::positions::{builtin_positions, parse_position, read_positions};

#[derive(Default, Debug)]
struct Work {
    moves: u64,
    drops: u64,
    captures: u64,
    /// 動いた駒を引く・置いた駒を足す・取られた駒を引く短い利き
    short: u64,
    /// 動く駒・取られる駒自身の飛び利きの線と升
    own_lines: u64,
    own_squares: u64,
    /// 行き先で切れる他の駒の線と升
    cut_lines: u64,
    cut_squares: u64,
    /// 移動元で伸びる他の駒の線と升
    ext_lines: u64,
    ext_squares: u64,
}

/// 升sから方向dへ、最初の駒の升まで（壁は含めない）の升の数。blockの升は駒があるとみなし、
/// emptyの升は空きとみなす。
fn ray_len(pos: &Position, s: u8, d: usize, block: u8, empty: u8) -> u64 {
    let delta = DELTA[d] as i32;
    let mut t = (s as i32 + delta) as u8;
    let mut n = 0;
    loop {
        let v = pos.piece_on(t);
        if v == WALL {
            return n;
        }
        n += 1;
        if t == block || (v != EMPTY && t != empty) {
            return n;
        }
        t = (t as i32 + delta) as u8;
    }
}

fn piece_lines(pos: &Position, p: u8, s: u8, block: u8, empty: u8, w: &mut Work) {
    let m = &MOVES[p as usize];
    w.short += u64::from(m.n_short);
    for d in 0..8 {
        if m.long & (1 << d) != 0 {
            w.own_lines += 1;
            w.own_squares += ray_len(pos, s, d, block, empty);
        }
    }
}

fn count_move(pos: &Position, m: Move, w: &mut Work) {
    w.moves += 1;
    let to = m.to();
    let np = m.piece_after();
    const NONE: u8 = 0xFF;
    if m.is_drop() {
        w.drops += 1;
    } else {
        let from = m.from();
        let p = pos.piece_on(from);
        piece_lines(pos, p, from, NONE, NONE, w);
        piece_lines(pos, np, to, NONE, from, w);
        // 移動元で伸びる線（指した後の盤。行き先には駒がある）
        for c in [Color::Black, Color::White] {
            let dirs = pos.long_dirs(c, from);
            for d in 0..8 {
                if dirs & (1 << d) != 0 {
                    w.ext_lines += 1;
                    w.ext_squares += ray_len(pos, from, d, to, NONE);
                }
            }
        }
    }
    let cap = pos.piece_on(to);
    if cap != EMPTY {
        w.captures += 1;
        piece_lines(pos, cap, to, NONE, NONE, w);
    } else {
        if m.is_drop() {
            piece_lines(pos, np, to, NONE, NONE, w);
        }
        // 行き先で切れる線
        for c in [Color::Black, Color::White] {
            let dirs = pos.long_dirs(c, to);
            for d in 0..8 {
                if dirs & (1 << d) != 0 {
                    w.cut_lines += 1;
                    w.cut_squares += ray_len(pos, to, d, NONE, NONE);
                }
            }
        }
    }
}

fn walk(pos: &mut Position, depth: u32, w: &mut Work) {
    let mut list = MoveList::default();
    pos.generate_legal(true, &mut list);
    for &m in list.as_slice() {
        let capture = !m.is_drop() && pos.piece_on(m.to()) != EMPTY;
        if depth == 0 && !capture {
            continue;
        }
        count_move(pos, m, w);
        pos.do_move(m);
        if depth > 0 {
            walk(pos, depth - 1, w);
        }
        pos.undo_move(m);
    }
}

#[test]
#[ignore = "見積もりの表示だけを行う。cargo test --release --test work_count -- --ignored --nocapture"]
fn do_undo_work() {
    let late =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../openings/profile_late_positions.txt");
    let mut lines = builtin_positions();
    lines.extend(read_positions(&late).unwrap());
    let mut w = Work::default();
    for line in &lines {
        let core = parse_position(line).unwrap();
        let mut pos = Position::from_sfen(&core.to_sfen()).unwrap();
        walk(&mut pos, 2, &mut w);
    }
    let n = w.moves as f64;
    let per = |x: u64| x as f64 / n;
    println!("局面 {}、進めた手 {}", lines.len(), w.moves);
    println!(
        "駒打ち {:.3}、駒取り {:.3}（手の割合）",
        per(w.drops),
        per(w.captures)
    );
    println!("1手あたり（進めだけ。戻しも同じ量）");
    println!("  短い利き {:.2}", per(w.short));
    println!(
        "  自分の線 {:.2}本 {:.2}升",
        per(w.own_lines),
        per(w.own_squares)
    );
    println!(
        "  行き先で切れる線 {:.2}本 {:.2}升",
        per(w.cut_lines),
        per(w.cut_squares)
    );
    println!(
        "  移動元で伸びる線 {:.2}本 {:.2}升",
        per(w.ext_lines),
        per(w.ext_squares)
    );
    let bytes = per(w.short) + 2.0 * per(w.own_squares + w.cut_squares + w.ext_squares);
    println!(
        "  利きの表の読み書き {bytes:.1}バイト（1往復で約{:.0}）",
        2.0 * bytes
    );
}

fn walk_mate(pos: &mut Position, depth: u32, c: &mut MateCount) {
    let mut list = MoveList::default();
    pos.generate_legal(true, &mut list);
    if !pos.in_check() {
        pos.mate_1ply_counted(c);
    }
    for &m in list.as_slice() {
        let capture = !m.is_drop() && pos.piece_on(m.to()) != EMPTY;
        if depth == 0 && !capture {
            continue;
        }
        pos.do_move(m);
        if depth > 0 {
            walk_mate(pos, depth - 1, c);
        }
        pos.undo_move(m);
    }
}

#[test]
#[ignore = "見積もりの表示だけを行う。cargo test --release --test work_count -- --ignored --nocapture"]
fn mate_1ply_work() {
    let late =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../openings/profile_late_positions.txt");
    let mut lines = builtin_positions();
    lines.extend(read_positions(&late).unwrap());
    let mut c = MateCount::default();
    for line in &lines {
        let core = parse_position(line).unwrap();
        let mut pos = Position::from_sfen(&core.to_sfen()).unwrap();
        walk_mate(&mut pos, 2, &mut c);
    }
    let per = |x: u64| x as f64 / c.calls as f64;
    println!("1手詰めの呼び出し {}", c.calls);
    println!("1回あたり");
    println!("  回した駒 {:.2}", per(c.pieces_seen));
    println!("  候補の表で残った駒 {:.2}", per(c.pieces_kept));
    println!("  盤上の駒の候補の行き先 {:.2}", per(c.targets));
    println!("  駒打ちの候補の升 {:.2}", per(c.drop_targets));
    println!("  判定まで届いた候補 {:.4}", per(c.judged));
    println!(
        "  近傍から飛び利きの元へ歩いた回数 {:.2}",
        per(c.line_walks)
    );
}
