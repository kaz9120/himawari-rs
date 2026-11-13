//! 3手詰めの判定が、詰みでない手を詰みと言わないことを、全幅の総当たりと突き合わせて確かめる。

use himawari_mailbox::{Move, MoveList, Position};

fn legal(pos: &Position) -> Vec<Move> {
    let mut l = MoveList::default();
    pos.generate_legal(true, &mut l);
    l.as_slice().to_vec()
}

/// 手番側に合法手が無い（詰み）か。
fn no_moves(pos: &Position) -> bool {
    legal(pos).is_empty()
}

/// 手番側に1手で相手を詰ます手があるか（全幅）。
fn mate1_brute(pos: &mut Position) -> bool {
    for m in legal(pos) {
        pos.do_move(m);
        let mated = pos.in_check() && no_moves(pos);
        pos.undo_move(m);
        if mated {
            return true;
        }
    }
    false
}

/// 王手mの後、玉側のどの応手にも1手詰めがあるか（全幅）。
fn mate3_by(pos: &mut Position, m: Move) -> bool {
    pos.do_move(m);
    let mut ok = pos.in_check();
    if ok {
        for e in legal(pos) {
            pos.do_move(e);
            let r = mate1_brute(pos);
            pos.undo_move(e);
            if !r {
                ok = false;
                break;
            }
        }
    }
    pos.undo_move(m);
    ok
}

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

fn start_positions() -> Vec<Position> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../openings/profile_late_positions.txt"
    );
    let text = std::fs::read_to_string(path).unwrap();
    let mut out = Vec::new();
    for line in text.lines().filter(|l| l.starts_with("sfen ")) {
        let body = &line[5..];
        let (sfen, moves) = match body.split_once(" moves ") {
            Some((s, m)) => (s, m),
            None => (body, ""),
        };
        let mut pos = Position::from_sfen(sfen).unwrap();
        for usi in moves.split_whitespace() {
            let m = legal(&pos).into_iter().find(|m| m.to_usi() == usi).unwrap();
            pos.do_move(m);
        }
        out.push(pos);
    }
    out
}

#[test]
fn generate_checks_matches_filtered_legal() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut total = 0usize;
    for start in start_positions() {
        for _ in 0..20 {
            let mut pos = start.clone();
            for _ in 0..12 {
                if !pos.in_check() {
                    for all in [false, true] {
                        let mut want: Vec<u32> = {
                            let mut l = MoveList::default();
                            pos.generate(himawari_mailbox::GenType::NonEvasions, all, &mut l);
                            l.as_slice()
                                .iter()
                                .filter(|&&m| pos.gives_check(m))
                                .map(|m| m.raw())
                                .collect()
                        };
                        let mut got: Vec<u32> = {
                            let mut l = MoveList::default();
                            pos.generate_checks(all, &mut l);
                            l.as_slice().iter().map(|m| m.raw()).collect()
                        };
                        want.sort_unstable();
                        got.sort_unstable();
                        assert_eq!(got, want, "{}", pos.to_sfen());
                        total += got.len();
                    }
                }
                let moves = legal(&pos);
                if moves.is_empty() {
                    break;
                }
                pos.do_move(moves[(rng.next() % moves.len() as u64) as usize]);
            }
        }
    }
    assert!(total > 0);
}

#[test]
fn mate3_never_claims_a_non_mate() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (mut probed, mut found) = (0u32, 0u32);
    for start in start_positions() {
        for _ in 0..40 {
            let mut pos = start.clone();
            for _ in 0..12 {
                if !pos.in_check() {
                    probed += 1;
                    if let Some(m) = pos.mate_3ply() {
                        found += 1;
                        let real = mate1_brute(&mut pos) || mate3_by(&mut pos, m);
                        assert!(real, "{} {}", pos.to_sfen(), m.to_usi());
                    }
                }
                let moves = legal(&pos);
                if moves.is_empty() {
                    break;
                }
                let m = moves[(rng.next() % moves.len() as u64) as usize];
                pos.do_move(m);
            }
        }
    }
    println!("調べた局面 {probed}、詰みと判定 {found}");
    assert!(found > 0);
}
