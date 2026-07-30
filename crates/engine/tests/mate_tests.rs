//! mate_1plyの健全性テスト（ADR-0029）。
//!
//! ランダムプレイアウトの全局面で、案B（mate_1ply）の返す手が
//! 実際に詰みであること（誤検出ゼロ）をオラクル同等の検証で確認し、
//! 近接王手クラスの詰みを見逃さないことを固定局面で確認する。
//!
//! プレイアウトの局面は詰みがまれなので、見逃し率は玉と数駒だけの
//! 疎な乱数局面で測る（ADR-0109）。乱数局面は合法な到達可能性を
//! 問わないが、健全性の反例探しにはそれで足りる。

use himawari_core::{Color, Move, MoveList, PieceType, Position, SFEN_STARTPOS, generate_legal};
use himawari_engine::mate::{mate_1ply, mate_1ply_oracle};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

fn verify_is_mate(pos: &mut Position, m: Move) {
    pos.do_move(m);
    assert!(pos.in_check(), "詰み手のはずが王手でない: {}", m.to_usi());
    let mut ev = MoveList::default();
    generate_legal(pos, true, &mut ev);
    assert!(ev.is_empty(), "詰み手のはずが回避がある: {}", m.to_usi());
    pos.undo_move(m);
}

/// 1手詰めの代表局面（tsumeスモークと同種）で見逃さないこと。
#[test]
fn finds_known_mates() {
    for sfen in [
        // 桂に支えられた金打ち
        "4k4/9/9/5N3/9/9/9/9/4K4 b G 1",
        "3k5/9/9/4N4/9/9/9/9/4K4 b G 1",
        // 頭金（5cの歩が支え）
        "4k4/9/4P4/9/9/9/9/9/4K4 b G 1",
    ] {
        let mut pos = Position::from_sfen(sfen).unwrap();
        let m = mate_1ply(&pos);
        assert!(m.is_some(), "1手詰めを見逃した: {sfen}");
        verify_is_mate(&mut pos, m.unwrap());
    }
}

/// ランダムプレイアウト全局面で誤検出ゼロ。オラクルとの関係も確認する。
#[test]
fn random_playouts_soundness() {
    let mut found = 0u32;
    let mut oracle_found = 0u32;
    for seed in 1..=100u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
        for _ in 0..80 {
            let mut list = MoveList::default();
            generate_legal(&pos, true, &mut list);
            if list.is_empty() {
                break;
            }
            if !pos.in_check() {
                if let Some(m) = mate_1ply(&pos) {
                    found += 1;
                    verify_is_mate(&mut pos, m);
                }
                if mate_1ply_oracle(&mut pos).is_some() {
                    oracle_found += 1;
                }
            }
            let m = list.as_slice()[(rng.next() % list.len() as u64) as usize];
            pos.do_move(m);
        }
    }
    // 見逃しは許容するが、オラクルが見つける詰みの大半は拾えるはず
    assert!(
        found * 10 >= oracle_found * 5,
        "検出率が低すぎる: mate_1ply {found} vs oracle {oracle_found}"
    );
}

/// 疎な乱数局面を1つ作る。玉2枚と数駒だけを置き、先手番にする。
/// 到達可能性は問わないが、二歩と行き所のない駒は避ける。
fn random_position(rng: &mut Rng) -> Option<Position> {
    // index = 筋 * 9 + 段。段0が1段目（先手から見た最奥）
    let mut board = [None::<char>; 81];
    let wk = (rng.next() % 81) as usize;
    let bk = (rng.next() % 81) as usize;
    if bk == wk {
        return None;
    }
    // 玉が隣接する局面は非合法
    let (bf, br) = ((bk / 9) as i32, (bk % 9) as i32);
    let (wf, wr) = ((wk / 9) as i32, (wk % 9) as i32);
    if (bf - wf).abs() <= 1 && (br - wr).abs() <= 1 {
        return None;
    }
    board[wk] = Some('k');
    board[bk] = Some('K');

    // 攻め方（先手）の駒は玉の5×5近傍へ置く。詰みが出やすくなる
    let attackers = ['R', 'B', 'G', 'S', 'N', 'L', 'P', '+', 'D'];
    let n_att = 1 + rng.next() % 4;
    let mut black_pawn_files = [false; 9];
    for _ in 0..n_att {
        let df = (rng.next() % 5) as i32 - 2;
        let dr = (rng.next() % 5) as i32 - 2;
        let (f, r) = (wf + df, wr + dr);
        if !(0..9).contains(&f) || !(0..9).contains(&r) {
            continue;
        }
        let sq = (f * 9 + r) as usize;
        if board[sq].is_some() {
            continue;
        }
        let c = attackers[(rng.next() % attackers.len() as u64) as usize];
        // 先手の歩・香は1段目、桂は1〜2段目に置けない
        let ok = match c {
            'P' | 'L' => r >= 1,
            'N' => r >= 2,
            _ => true,
        };
        if !ok {
            continue;
        }
        if c == 'P' {
            if black_pawn_files[f as usize] {
                continue;
            }
            black_pawn_files[f as usize] = true;
        }
        // '+' は竜、'D' は馬の代わり（SFENは+R / +B）
        board[sq] = Some(c);
    }

    // 受け方（後手）の駒を数枚。玉の守りになる
    let defenders = ['g', 's', 'p', 'n', 'l'];
    let n_def = rng.next() % 3;
    let mut white_pawn_files = [false; 9];
    for _ in 0..n_def {
        let df = (rng.next() % 5) as i32 - 2;
        let dr = (rng.next() % 5) as i32 - 2;
        let (f, r) = (wf + df, wr + dr);
        if !(0..9).contains(&f) || !(0..9).contains(&r) {
            continue;
        }
        let sq = (f * 9 + r) as usize;
        if board[sq].is_some() {
            continue;
        }
        let c = defenders[(rng.next() % defenders.len() as u64) as usize];
        // 後手の歩・香は9段目、桂は8〜9段目に置けない
        let ok = match c {
            'p' | 'l' => r <= 7,
            'n' => r <= 6,
            _ => true,
        };
        if !ok {
            continue;
        }
        if c == 'p' {
            if white_pawn_files[f as usize] {
                continue;
            }
            white_pawn_files[f as usize] = true;
        }
        board[sq] = Some(c);
    }

    // 盤面のSFEN。1段目から順に、9筋から1筋へ
    let mut s = String::new();
    for r in 0..9 {
        let mut run = 0;
        for f in (0..9).rev() {
            match board[f * 9 + r] {
                None => run += 1,
                Some(c) => {
                    if run > 0 {
                        s.push_str(&run.to_string());
                        run = 0;
                    }
                    match c {
                        '+' => s.push_str("+R"),
                        'D' => s.push_str("+B"),
                        c => s.push(c),
                    }
                }
            }
        }
        if run > 0 {
            s.push_str(&run.to_string());
        }
        if r < 8 {
            s.push('/');
        }
    }
    s.push_str(" b ");
    // 持ち駒。先手に打ち駒を持たせて駒打ちの詰みも試す
    let hands = ["-", "G", "S", "N", "L", "GS", "R", "B", "GSNL", "P", "Gp"];
    s.push_str(hands[(rng.next() % hands.len() as u64) as usize]);
    s.push_str(" 1");

    let pos = Position::from_sfen(&s).ok()?;
    // 先手番なので先手玉に王手がかかっていてはならない
    if pos.in_check() {
        return None;
    }
    // 後手玉に王手がかかっている局面は非合法（手番側が取れてしまう）
    let occ = pos.occupied();
    let wksq = pos.king(Color::White);
    if !pos.attackers_to(Color::Black, wksq, occ).is_empty() {
        return None;
    }
    Some(pos)
}

/// 疎な乱数局面で誤検出ゼロを確かめ、見逃し率を測る。
#[test]
fn random_positions_soundness_and_miss_rate() {
    let mut rng = Rng(0x1234_5678_9ABC_DEF0);
    let mut positions = 0u32;
    let mut found = 0u32;
    let mut oracle_found = 0u32;
    let mut miss = 0u32;
    while positions < 40_000 {
        let Some(mut pos) = random_position(&mut rng) else {
            continue;
        };
        positions += 1;
        let ours = mate_1ply(&pos);
        let oracle = mate_1ply_oracle(&mut pos);
        if oracle.is_some() {
            oracle_found += 1;
        }
        if let Some(m) = ours {
            found += 1;
            // 返した手が本当に詰みか（偽陽性ゼロの確認）
            verify_is_mate(&mut pos, m);
            assert!(
                oracle.is_some(),
                "オラクルが詰みでない局面で詰みを返した: {}",
                pos.to_sfen()
            );
        } else if oracle.is_some() {
            miss += 1;
        }
    }
    println!(
        "random_positions: positions={positions} oracle_mate={oracle_found} \
         ours={found} miss={miss} miss_rate={:.2}%",
        f64::from(miss) / f64::from(found + miss) * 100.0
    );
    // 見逃しは許すが、オラクルが見つける詰みの過半は拾えていること
    assert!(found >= miss, "見逃しが多すぎる: ours={found} miss={miss}");
}

/// 駒打ちの詰みを見逃さないこと。近接打ちは方式を変えても拾えるはず。
#[test]
fn finds_drop_mates() {
    for (sfen, expect) in [
        // 頭金。5cの歩が支え
        ("4k4/9/4P4/9/9/9/9/9/4K4 b G 1", PieceType::GOLD),
        // 桂の王手（打ち桂）
        ("4k4/9/9/9/9/9/9/9/4K4 b N 1", PieceType::KNIGHT),
    ] {
        let mut pos = Position::from_sfen(sfen).unwrap();
        let m = mate_1ply(&pos);
        if expect == PieceType::KNIGHT {
            // 玉が広いので詰まない。偽陽性が出ないことだけ見る
            assert!(m.is_none(), "詰まない局面で詰みを返した: {sfen}");
            continue;
        }
        let m = m.expect("駒打ちの1手詰めを見逃した");
        assert!(m.is_drop(), "駒打ちのはず: {}", m.to_usi());
        verify_is_mate(&mut pos, m);
    }
}
