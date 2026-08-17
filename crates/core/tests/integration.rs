//! Position・movegen・perftの統合テスト（ADR-0006, 0018）。
//!
//! do/undo往復の完全一致と、差分更新キー＝全計算キーの一致を
//! ランダムプレイアウトで検証する。perftは公開値と照合する。

use himawari_core::{
    Color, GenType, Move, MoveList, Position, Repetition, SFEN_STARTPOS, generate, generate_legal,
    perft, perft_slow,
};

fn apply(pos: &mut Position, moves: &[&str]) {
    for s in moves {
        let m = pos
            .move_from_usi(s)
            .unwrap_or_else(|| panic!("illegal: {s}"));
        pos.do_move(m);
    }
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

#[test]
fn sfen_startpos_roundtrip() {
    let pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    assert_eq!(pos.to_sfen(), SFEN_STARTPOS);
    assert_eq!(pos.side_to_move(), Color::Black);
    assert!(!pos.in_check());
}

#[test]
fn sfen_rejects_invalid() {
    // 玉がない
    assert!(Position::from_sfen("9/9/9/9/9/9/9/9/9 b - 1").is_err());
    // 二歩
    assert!(Position::from_sfen("4k4/9/9/9/9/9/4P4/4P4/4K4 b - 1").is_err());
    // 行き所のない桂
    assert!(Position::from_sfen("N3k4/9/9/9/9/9/9/9/4K4 b - 1").is_err());
    // 歩が19枚
    assert!(Position::from_sfen("4k4/9/9/9/9/9/9/9/4K4 b 19P 1").is_err());
}

#[test]
fn startpos_has_30_moves() {
    let pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    let mut list = MoveList::default();
    generate_legal(&pos, true, &mut list);
    assert_eq!(list.len(), 30);
}

#[test]
fn perft_startpos_shallow() {
    let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    assert_eq!(perft(&mut pos, 1), 30);
    assert_eq!(perft(&mut pos, 2), 900);
    assert_eq!(perft(&mut pos, 3), 25_470);
}

#[test]
fn perft_matches_slow_perft() {
    let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    for d in 1..=3 {
        assert_eq!(perft(&mut pos, d), perft_slow(&mut pos, d));
    }
}

// releaseビルドでのみ実行（CIのreleaseテストが回す。ADR-0006/0018）
#[cfg_attr(debug_assertions, ignore)]
#[test]
fn perft_startpos_depth4() {
    let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    assert_eq!(perft(&mut pos, 4), 719_731);
}

#[cfg_attr(debug_assertions, ignore)]
#[test]
fn perft_startpos_depth5() {
    let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    assert_eq!(perft(&mut pos, 5), 19_861_490);
}

/// 最大分岐の既知局面（593手）。
#[test]
fn max_branching_position() {
    let pos = Position::from_sfen("R8/2K1S1SSk/4B4/9/9/9/9/9/1L1L1L3 b RBGSNLP3G3N17P 1").unwrap();
    let mut list = MoveList::default();
    generate_legal(&pos, true, &mut list);
    assert_eq!(list.len(), 593);
}

/// 打ち歩詰め: 5二への歩打ちは詰みなので非合法。
/// 金5三が歩と4二・6二を守り、龍2枚が4一・6一を抑える。
#[test]
fn uchifuzume_is_illegal() {
    let pos = Position::from_sfen("4k4/9/4G4/9/9/9/3+R1+R3/9/K8 b P 1").unwrap();
    let mut list = MoveList::default();
    generate_legal(&pos, true, &mut list);
    let drop = list.as_slice().iter().find(|m| m.to_usi() == "P*5b");
    assert!(drop.is_none(), "打ち歩詰めが生成された");

    // 金がなければ玉が歩を取れるので合法
    let pos2 = Position::from_sfen("4k4/9/9/9/9/9/3+R1+R3/9/K8 b P 1").unwrap();
    let mut list2 = MoveList::default();
    generate_legal(&pos2, true, &mut list2);
    let drop2 = list2.as_slice().iter().find(|m| m.to_usi() == "P*5b");
    assert!(drop2.is_some(), "詰みでない歩打ちが消えた");
}

fn random_playout(seed: u64, plies: usize, all: bool) {
    let mut rng = Rng(seed);
    let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    let mut history: Vec<(Move, String, u64)> = Vec::new();

    for _ in 0..plies {
        let mut list = MoveList::default();
        generate_legal(&pos, all, &mut list);
        if list.is_empty() {
            break;
        }

        // 差分更新キー＝SFEN経由の全計算キー
        let fresh = Position::from_sfen(&pos.to_sfen()).unwrap();
        assert_eq!(pos.key(), fresh.key(), "差分キーと全計算キーの不一致");
        assert_eq!(
            pos.state().material,
            fresh.state().material,
            "差分materialと全計算の不一致"
        );
        // 歩構造キーの差分更新＝全計算の一致（ADR-0046）
        assert_eq!(
            pos.pawn_key(),
            fresh.pawn_key(),
            "差分pawn_keyと全計算の不一致"
        );
        assert_eq!(
            pos.pawn_key(),
            pos.compute_pawn_key(),
            "pawn_keyの差分と全計算の不一致"
        );
        // 歩以外の盤上駒キーも同様に検証する（ADR-0085, 0109）
        for c in [Color::Black, Color::White] {
            assert_eq!(
                pos.non_pawn_key(c),
                fresh.non_pawn_key(c),
                "差分non_pawn_keyと全計算の不一致"
            );
            assert_eq!(
                pos.non_pawn_key(c),
                pos.compute_non_pawn_key(c),
                "non_pawn_keyの差分と全計算の不一致"
            );
        }
        // 小駒キーも同様に検証する（ADR-0109）
        assert_eq!(
            pos.minor_piece_key(),
            fresh.minor_piece_key(),
            "差分minor_piece_keyと全計算の不一致"
        );
        assert_eq!(
            pos.minor_piece_key(),
            pos.compute_minor_piece_key(),
            "minor_piece_keyの差分と全計算の不一致"
        );

        // gives_checkの整合
        let idx = (rng.next() % list.len() as u64) as usize;
        let m = list.as_slice()[idx];
        let predicted = pos.gives_check(m);

        history.push((m, pos.to_sfen(), pos.key()));
        pos.do_move(m);
        assert_eq!(
            pos.in_check(),
            predicted,
            "gives_checkの不一致: {}",
            m.to_usi()
        );
    }

    // 全部巻き戻して一致を確認
    while let Some((m, sfen, key)) = history.pop() {
        pos.undo_move(m);
        assert_eq!(pos.to_sfen(), sfen, "undo後のSFEN不一致");
        assert_eq!(pos.key(), key, "undo後のキー不一致");
    }
    assert_eq!(pos.to_sfen(), SFEN_STARTPOS);
}

#[test]
fn do_undo_roundtrip_random_games() {
    for seed in 1..=8u64 {
        random_playout(seed, 256, true);
        random_playout(seed.wrapping_mul(0x9E37_79B9), 256, false);
    }
}

/// Captures ∪ Quiets == NonEvasions（多重集合として一致）。
#[test]
fn captures_plus_quiets_equals_non_evasions() {
    let mut rng = Rng(0xC0FF_EE00_1234_5678);
    let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    for _ in 0..300 {
        let mut legal = MoveList::default();
        generate_legal(&pos, true, &mut legal);
        if legal.is_empty() {
            break;
        }
        if !pos.in_check() {
            for all in [false, true] {
                let mut ne = MoveList::default();
                let mut cap = MoveList::default();
                let mut qt = MoveList::default();
                generate(&pos, GenType::NonEvasions, all, &mut ne);
                generate(&pos, GenType::Captures, all, &mut cap);
                generate(&pos, GenType::Quiets, all, &mut qt);
                let mut a: Vec<String> = ne.as_slice().iter().map(|m| m.to_usi()).collect();
                let mut b: Vec<String> = cap
                    .as_slice()
                    .iter()
                    .chain(qt.as_slice())
                    .map(|m| m.to_usi())
                    .collect();
                a.sort();
                b.sort();
                assert_eq!(a, b, "分類の合成がNonEvasionsと不一致 (all={all})");
            }
        }
        let idx = (rng.next() % legal.len() as u64) as usize;
        pos.do_move(legal.as_slice()[idx]);
    }
}

/// move_from_usiが合法手を復元できる。
#[test]
fn move_from_usi_roundtrip() {
    let pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    let m = pos.move_from_usi("7g7f").unwrap();
    assert_eq!(m.to_usi(), "7g7f");
    assert!(pos.move_from_usi("7g7e").is_none());
}

/// 飛車の往復による通常の千日手（ADR-0026）。
/// 千日手はrootを跨いでも成立する（ADR-0153）ので、ply=0でも返る。
#[test]
fn repetition_draw() {
    let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    assert_eq!(pos.repetition_state_all(), Repetition::None);
    apply(&mut pos, &["2h3h", "8b7b", "3h2h", "7b8b"]);
    assert_eq!(pos.repetition_state_all(), Repetition::Draw);
    assert_eq!(pos.repetition_state(0), Repetition::Draw);
}

/// 連続王手の千日手。王手を掛け続けた側（先手）がLose。
#[test]
fn perpetual_check_is_loss_for_checker() {
    let mut pos = Position::from_sfen("4k4/9/9/9/9/9/9/5R3/4K4 b - 1").unwrap();
    apply(&mut pos, &["4h5h", "5a4a", "5h4h", "4a5a"]);
    // ここで盤面はループ先頭と同一。先手の手はすべて王手だった
    assert_eq!(pos.repetition_state_all(), Repetition::Lose);
    assert_eq!(pos.repetition_state(0), Repetition::Lose);
}

/// 優等局面はrootを跨いで判定しない（ADR-0153）。
///
/// 8ply前と盤面が同一で、先手だけ歩を1枚得ている局面を作る。手順は
/// 先手が2六の歩を飛車で取り、玉の往復で手待ちし、後手が同じ2六へ
/// 歩を打ち直す。検出距離は8plyになる。
#[test]
fn superior_is_gated_by_search_ply() {
    let mut pos = Position::from_sfen("4k4/9/9/9/9/7p1/9/7R1/4K4 b p 1").unwrap();
    apply(
        &mut pos,
        &[
            "2h2f", "5a4a", // 飛車が歩を取り、後手玉が動く
            "2f2h", "P*2f", // 飛車が戻り、後手が同じ位置へ歩を打つ
            "5i4i", "4a4b", // 双方が手待ちする
            "4i5i", "4b5a",
        ],
    );
    // 検出距離は8ply。探索経路が9ply以上あれば優等として返る
    assert_eq!(pos.repetition_state(9), Repetition::Superior);
    assert_eq!(pos.repetition_state(usize::MAX), Repetition::Superior);
    assert_eq!(pos.repetition_state_all(), Repetition::Superior);
    // rootが検出位置と同じか、それより後ろなら返さない
    assert_eq!(pos.repetition_state(8), Repetition::None);
    assert_eq!(pos.repetition_state(4), Repetition::None);
    assert_eq!(pos.repetition_state(0), Repetition::None);
}

/// null moveの往復一致と、手番だけ違う局面のキー相違（ADR-0028）。
#[test]
fn null_move_roundtrip() {
    let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    let key = pos.key();
    let sfen = pos.to_sfen();
    pos.do_null_move();
    assert_eq!(pos.side_to_move(), Color::White);
    assert_ne!(pos.key(), key);
    pos.undo_null_move();
    assert_eq!(pos.side_to_move(), Color::Black);
    assert_eq!(pos.key(), key);
    assert_eq!(pos.to_sfen(), sfen);
}

/// 千日手の走査はnull moveを跨がない（ADR-0028）。
#[test]
fn repetition_scan_stops_at_null_move() {
    let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    apply(&mut pos, &["2h3h", "8b7b", "3h2h", "7b8b"]);
    assert_eq!(pos.repetition_state_all(), Repetition::Draw);
    // ループをもう1周するが、途中にnull moveを挟む
    apply(&mut pos, &["2h3h", "8b7b", "3h2h"]);
    pos.do_null_move();
    pos.do_null_move();
    apply(&mut pos, &["7b8b"]);
    assert_eq!(pos.repetition_state_all(), Repetition::None);
}

/// 入玉宣言勝ち（27点法、ADR-0030）の境界条件。
#[test]
fn declaration_win_boundaries() {
    // 先手: 敵陣に龍+と9枚（14点）+持駒RB4G（14点）= 28点、駒10枚
    let valid = "K+R+P+P+P+P+P+P+P/7+P+P/9/9/9/9/9/9/4k4 b RB4G 1";
    assert!(Position::from_sfen(valid).unwrap().can_declare_win());
    // 27点しかない先手は不成立
    let short = "K+R+P+P+P+P+P+P+P/7+P+P/9/9/9/9/9/9/4k4 b RB3G 1";
    assert!(!Position::from_sfen(short).unwrap().can_declare_win());
    // 後手は27点で成立
    let white = "4K4/9/9/9/9/9/9/+p+p7/k+r+p+p+p+p+p+p+p w rb3g 1";
    assert!(Position::from_sfen(white).unwrap().can_declare_win());
    // 後手26点は不成立
    let white_short = "4K4/9/9/9/9/9/9/+p+p7/k+r+p+p+p+p+p+p+p w rb2g 1";
    assert!(!Position::from_sfen(white_short).unwrap().can_declare_win());
    // 王手されていると不成立（9bの後手金が玉の背後に利く）
    let in_check = "K+R+P+P+P+P+P+P+P/g6+P+P/9/9/9/9/9/9/4k4 b RB3GS 1";
    assert!(!Position::from_sfen(in_check).unwrap().can_declare_win());
    // 敵陣内の駒が9枚では不成立
    let few = "K+R+P+P+P+P+P+P+P/8+P/9/9/9/9/9/9/4k4 b RB4G 1";
    assert!(!Position::from_sfen(few).unwrap().can_declare_win());
    // 玉が敵陣の外では不成立
    let outside = "9/9/9/K+R+P+P+P+P+P+P+P/9/9/9/7+P+P/4k4 b RB4G 1";
    assert!(!Position::from_sfen(outside).unwrap().can_declare_win());
}

/// SEE: 玉に守られた歩を香で取るのは損、守られていなければ得。
#[test]
fn see_defended_and_undefended() {
    let pos = Position::from_sfen("9/4k4/4p4/9/4L4/9/9/9/K8 b - 1").unwrap();
    let m = pos.move_from_usi("5e5c").unwrap();
    assert!(!pos.see_ge(m, 0), "守られた歩を取るのは損のはず");
    assert!(pos.see_ge(m, -300), "損は歩と香の差程度のはず");

    let pos2 = Position::from_sfen("5k3/9/4p4/9/4L4/9/9/9/K8 b - 1").unwrap();
    let m2 = pos2.move_from_usi("5e5c").unwrap();
    assert!(pos2.see_ge(m2, 0), "守られていない歩は取り得のはず");
}

/// pseudo_legalとto_moveの整合: 全合法手は復元・検査を通過する。
#[test]
fn pseudo_legal_accepts_generated_moves() {
    let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
    let mut rng = Rng(0xABCD_EF01_2345_6789);
    for _ in 0..120 {
        let mut list = MoveList::default();
        generate_legal(&pos, true, &mut list);
        if list.is_empty() {
            break;
        }
        for &m in &list {
            assert!(
                pos.pseudo_legal(m),
                "合法手がpseudo_legalで弾かれた: {}",
                m.to_usi()
            );
            let restored = pos.to_move(m.to_move16()).unwrap();
            assert_eq!(restored, m, "Move16復元の不一致: {}", m.to_usi());
        }
        let idx = (rng.next() % list.len() as u64) as usize;
        pos.do_move(list.as_slice()[idx]);
    }
}

/// 駒打ちのSEE（ADR-0091）。打った駒が取られる筋を解く。
#[test]
fn see_ge_solves_drops() {
    use himawari_core::{Color, PieceType, Square};

    // 5四に後手の金。先手が5五へ歩を打つと金に取られる
    let pos = Position::from_sfen("4k4/9/9/4g4/9/9/9/9/4K4 b P 1").unwrap();
    let to = Square::from_usi("5e").unwrap();
    let m = Move::new_drop(PieceType::PAWN, to, Color::Black);
    assert!(pos.pseudo_legal(m), "5五への歩打ちは擬似合法");
    assert!(!pos.see_ge(m, 0), "取られる歩打ちの静的交換評価は0未満");
    assert!(!pos.see_ge(m, -50), "損は歩の価値ぶんあり-50にも届かない");
    assert!(pos.see_ge(m, -90), "歩1枚(90)の損までは許容される");

    // 誰も利いていないマスへの打ちは損しない
    let safe = Square::from_usi("1g").unwrap();
    let m2 = Move::new_drop(PieceType::PAWN, safe, Color::Black);
    assert!(pos.pseudo_legal(m2), "1七への歩打ちは擬似合法");
    assert!(pos.see_ge(m2, 0), "取られない打ちは損しない");
}

/// 成りのSEE（ADR-0095）。成りによる駒の価値上昇を取り分に入れる。
#[test]
fn see_ge_accounts_for_promotion() {
    use himawari_core::{Color, Piece, PieceType, Square};

    // 3四の先手歩が3三の後手歩を取る。取り返す駒はない
    let pos = Position::from_sfen("4k4/9/6p2/6P2/9/9/9/9/4K4 b - 1").unwrap();
    let from = Square::from_usi("3d").unwrap();
    let to = Square::from_usi("3c").unwrap();
    let promo = Move::new_move(
        from,
        to,
        true,
        Piece::new(Color::Black, PieceType::PRO_PAWN),
    );
    let plain = Move::new_move(from, to, false, Piece::new(Color::Black, PieceType::PAWN));
    assert!(pos.pseudo_legal(promo) && pos.pseudo_legal(plain));

    // 取る歩は90。成れば と金(540) との差450が上乗せされる
    assert!(pos.see_ge(plain, 90), "成らない手の取り分は歩1枚");
    assert!(!pos.see_ge(plain, 400), "成らない手は400に届かない");
    assert!(
        pos.see_ge(promo, 400),
        "成る手は歩90＋成りの利得450で400を超える"
    );
}

/// 桂の3段目不成を通常の生成でも出す（ADR-0173）。
///
/// 成桂は金の動きなので、桂の王手とは利きが違う。「成ると王手にならないが、
/// 不成なら王手になる」形があり、これを落とすと相手の詰め手順ごと見えなくなる。
/// 局面はfloodgateの実戦（2026-08-16 vs Daigoro-20171029、104手目）で、
/// 後手の5五桂を6七へ不成で跳ねると5九の先手玉に王手がかかる。
#[test]
fn knight_non_promotion_check_is_generated_in_normal_mode() {
    let pos = Position::from_sfen(
        "lr7/2g3k2/p2Ppp2B/4s1pPp/2Pnn4/PP1+b1P1p1/1S4P1N/6S2/L3KG2L w RGSNL3Pg2p 104",
    )
    .unwrap();

    let usi_of = |all: bool| -> Vec<String> {
        let mut list = MoveList::default();
        generate_legal(&pos, all, &mut list);
        list.as_slice().iter().map(|m| m.to_usi()).collect()
    };
    let normal = usi_of(false);
    let full = usi_of(true);

    assert!(
        normal.iter().any(|m| m == "5e6g"),
        "3段目への桂不成が通常の生成から漏れている"
    );
    assert!(
        full.iter().any(|m| m == "5e6g"),
        "全生成にも桂不成が無い（局面の前提が違う）"
    );

    // 王手であること。成ると王手にならないことも確かめる
    let find = |usi: &str| -> Move {
        let mut list = MoveList::default();
        generate_legal(&pos, true, &mut list);
        *list
            .as_slice()
            .iter()
            .find(|m| m.to_usi() == usi)
            .unwrap_or_else(|| panic!("{usi} が生成されていない"))
    };
    assert!(pos.gives_check(find("5e6g")), "桂不成は王手のはず");
    assert!(
        !pos.gives_check(find("5e6g+")),
        "成ると金の動きになり王手にならないはず"
    );
}

/// 行き所のない駒になる桂の不成は、どちらのモードでも生成しない（ADR-0017）。
#[test]
fn knight_non_promotion_is_not_generated_on_last_two_ranks() {
    // 先手の2三桂は1一・3一（1段目）へしか跳べない。不成は行き所がない
    let pos = Position::from_sfen("4k4/9/2N6/9/9/9/9/9/4K4 b - 1").unwrap();
    for all in [false, true] {
        let mut list = MoveList::default();
        generate_legal(&pos, all, &mut list);
        for m in list.as_slice() {
            let usi = m.to_usi();
            assert!(
                !(usi.starts_with("7c") && !usi.ends_with('+')),
                "1段目への桂不成を生成した（all={all}）: {usi}"
            );
        }
    }
}
