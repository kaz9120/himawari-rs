//! 配列盤の試作を、既存のcore（Bitboard）と突き合わせる（ADR-0241の段階3）。
//!
//! - SFENの往復がcoreの出力と一致する
//! - perft木の全ノードで、合法手の列が順まで含めてcoreと一致し、利きの配列が全計算と一致する。
//!   列は `generate_legal`（all の両方）と、Captures・Quiets（coreの擬似合法手を `is_legal` で濾した列）
//! - ランダムな対局の全手で同じ検査をする（終盤・駒打ち・王手の多い局面まで届かせる）
//! - 全ノードでZobristのキー5本・hand_key・駒割り・王手駒・連続王手の数がcoreと一致し、
//!   進めた直後のDirtyPieceがcoreと同じ中身になる
//! - 全ノードで、段階5の関数（is_legal・pseudo_legal・gives_check・see_ge・mate_1ply）の
//!   返す値がcoreと一致する
//! - perftの数がcoreと一致する（深い方は `--ignored` で走らせる）

use himawari_core::{
    GenType as CoreGen, Move as CoreMove, MoveList as CoreList, Position as CorePos, generate,
    generate_legal,
};
use himawari_engine::mate::mate_1ply;
use himawari_mailbox::{GenType, MoveList, Position, SFEN_STARTPOS, perft};

/// coreの統合テスト（generate_legal_matches_filtered_pseudo）と同じ6局面。
const CASES: [&str; 6] = [
    SFEN_STARTPOS,
    "+Bn1g2s1l/2skg2r1/ppppp1n1p/5bpp1/5p1P1/2P6/PP1PP1P1P/1SK2S1R1/LN1G1G1NL w Lp 24",
    "lr7/2g3k2/p2Ppp2B/4s1pPp/2Pnn4/PP1+b1P1p1/1S4P1N/6S2/L3KG2L w RGSNL3Pg2p 104",
    "l6nl/5+P1gk/2np1S3/p1p4Pp/3P2Sp1/1PPb2P1P/P5GS1/R8/LN4bKL w GR5pnsg 1",
    "R8/2K1S1SSk/4B4/9/9/9/9/9/1L1L1L3 b RBGSNLP3G3N17P 1",
    "K+R+P+P+P+P+P+P+P/g6+P+P/9/9/9/9/9/9/4k4 b RB3GS 1",
];

/// 王手・ピン・打ち歩詰めの周りを狙った局面。
const EXTRA: [&str; 9] = [
    // 打ち歩詰め（1一玉。1二の歩を2三の銀が支え、2一・2二は3二の金が押さえる）
    "8k/6G2/7S1/9/9/9/9/9/K8 b P 1",
    // 歩を打つと王手だが、玉が取れるので打ち歩詰めではない
    "8k/6G2/9/9/9/9/9/9/K8 b P 1",
    // 両王手の逃げ（5三の飛と7三の角）
    "4k4/9/2B1R4/9/9/9/9/9/K8 w - 1",
    // ピンされた駒（5筋の金）と飛び利きの延長
    "4k4/4g4/9/9/4R4/9/9/9/K8 w - 1",
    // 合駒の効く王手（駒打ちの合駒）
    "4k4/9/9/9/9/9/9/9/K3R4 w gsnlp 1",
    // 玉の背後の利き（玉が飛の線に沿って逃げられない）
    "9/9/9/4k4/9/9/9/4R4/K8 w - 1",
    // 同じ筋に香と飛が並ぶ（香が滑ると、後ろの飛の利きが香の後ろを延びる）
    "5k3/9/9/9/4L4/9/9/9/K3R4 b - 1",
    // 打ち歩詰め。歩を取れる2二の金が、4四の角にピンされている
    "8k/6Gg1/9/5B1N1/9/9/9/9/K8 b P 1",
    // 歩打ちが9二の飛の横利きを遮り、4二へ逃げ道ができるので打ち歩詰めではない
    "4k4/R8/2N3N2/5N3/9/9/9/9/K8 b P 1",
];

fn core_moves(pos: &CorePos) -> Vec<String> {
    let mut list = CoreList::default();
    generate_legal(pos, true, &mut list);
    list.as_slice().iter().map(|m| m.to_usi()).collect()
}

/// coreの生成の列。Noneならgenerate_legal、Someならその分類の擬似合法手を合法手で濾した列。
fn core_seq(pos: &CorePos, gt: Option<CoreGen>, all: bool) -> Vec<String> {
    let mut list = CoreList::default();
    match gt {
        None => generate_legal(pos, all, &mut list),
        Some(g) => {
            let mut pseudo = CoreList::default();
            generate(pos, g, all, &mut pseudo);
            for &m in pseudo.as_slice() {
                if pos.is_legal(m) {
                    list.push(m);
                }
            }
        }
    }
    list.as_slice().iter().map(|m| m.to_usi()).collect()
}

fn mb_seq(pos: &mut Position, gt: Option<GenType>, all: bool) -> Vec<String> {
    let mut list = MoveList::default();
    match gt {
        None => pos.generate_legal(all, &mut list),
        Some(g) => pos.generate(g, all, &mut list),
    }
    list.as_slice().iter().map(|m| m.to_usi()).collect()
}

/// 1ノードの検査。SFEN・利きの配列・合法手の列（順まで）。
/// 局面に付いて回る値（キー・駒割り・王手駒・連続王手・DirtyPiece）をcoreと比べる。
fn check_state(core: &CorePos, mb: &Position) {
    let c = core.state();
    let m = mb.state();
    let sfen = core.to_sfen();
    assert_eq!(mb.key(), core.key(), "key: {sfen}");
    assert_eq!(m.board_key, c.board_key, "board_key: {sfen}");
    assert_eq!(m.hand_key, c.hand_key, "hand_key: {sfen}");
    assert_eq!(m.pawn_key, c.pawn_key, "pawn_key: {sfen}");
    assert_eq!(m.non_pawn_key, c.non_pawn_key, "non_pawn_key: {sfen}");
    assert_eq!(
        m.minor_piece_key, c.minor_piece_key,
        "minor_piece_key: {sfen}"
    );
    assert_eq!(m.material, c.material, "material: {sfen}");
    let mut checkers = 0u128;
    for sq in c.checkers {
        checkers |= 1u128 << sq.index();
    }
    assert_eq!(m.checkers, checkers, "checkers: {sfen}");
    assert_eq!(
        m.continuous_check, c.continuous_check,
        "continuous_check: {sfen}"
    );
    assert_eq!(
        m.plies_from_null, c.plies_from_null,
        "plies_from_null: {sfen}"
    );
    let (cd, md) = (&c.dirty, &m.dirty);
    assert_eq!(md.count, cd.count, "dirty.count: {sfen}");
    assert_eq!(md.king_moved, cd.king_moved, "dirty.king_moved: {sfen}");
    for i in 0..2 {
        assert_eq!(
            md.piece_old[i], cd.piece_old[i].0,
            "dirty.piece_old[{i}]: {sfen}"
        );
        assert_eq!(
            md.piece_new[i], cd.piece_new[i].0,
            "dirty.piece_new[{i}]: {sfen}"
        );
        assert_eq!(
            md.from[i] as usize,
            cd.from[i].index(),
            "dirty.from[{i}]: {sfen}"
        );
        assert_eq!(md.to[i] as usize, cd.to[i].index(), "dirty.to[{i}]: {sfen}");
    }
}

/// coreの指し手を配列盤の指し手へ直す。
fn to_mb(m: CoreMove) -> himawari_mailbox::Move {
    use himawari_mailbox::board::IDX81;
    let pa = m.piece_after().0;
    if m.is_drop() {
        let c = if pa & 16 != 0 {
            himawari_mailbox::Color::White
        } else {
            himawari_mailbox::Color::Black
        };
        himawari_mailbox::Move::drop(m.drop_piece_type().0, IDX81[m.to().index()], c)
    } else {
        himawari_mailbox::Move::normal(
            IDX81[m.from_sq().index()],
            IDX81[m.to().index()],
            m.is_promote(),
            pa,
        )
    }
}

thread_local! {
    /// 直前に調べたノードの合法手。pseudo_legalの照合で「よその局面の手」として使う。
    static PREV_MOVES: std::cell::RefCell<Vec<CoreMove>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// 合法手のうち、1手詰めの判定（coreの取り決め）で詰みとされた手の数。照合の濃さを表示するために数える。
/// 並列に走るテストの間で共有するので、数を読むときは `--test-threads 1` で走らせる
static MATE_MOVES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 段階5の関数の返す値をcoreと比べる。
fn check_stage5(core: &CorePos, mb: &mut Position) {
    let sfen = core.to_sfen();
    let mut legal = CoreList::default();
    generate_legal(core, true, &mut legal);
    let legal: Vec<CoreMove> = legal.as_slice().to_vec();
    // is_legal: coreの擬似合法手の全部
    let mut pseudo = CoreList::default();
    let gt = if core.in_check() {
        CoreGen::Evasions
    } else {
        CoreGen::NonEvasions
    };
    generate(core, gt, true, &mut pseudo);
    for &m in pseudo.as_slice() {
        assert_eq!(
            mb.is_legal(to_mb(m)),
            core.is_legal(m),
            "is_legal {}: {sfen}",
            m.to_usi()
        );
    }
    // pseudo_legal: この局面の合法手・擬似合法手、直前のノードの合法手、成りと駒を入れ替えた手
    let mut cands: Vec<CoreMove> = legal.clone();
    cands.extend_from_slice(pseudo.as_slice());
    PREV_MOVES.with(|p| cands.extend_from_slice(&p.borrow()));
    let n = cands.len();
    for i in 0..n {
        let m = cands[i];
        if m.is_drop() {
            continue;
        }
        let pa = m.piece_after();
        let flipped = if m.is_promote() {
            CoreMove::new_move(m.from_sq(), m.to(), false, pa.unpromote())
        } else if pa.piece_type().can_promote() {
            CoreMove::new_move(m.from_sq(), m.to(), true, pa.promote())
        } else {
            continue;
        };
        cands.push(flipped);
        // 移動元の駒と合わない駒
        cands.push(CoreMove::new_move(
            m.from_sq(),
            m.to(),
            false,
            himawari_core::Piece(pa.0 ^ 1),
        ));
    }
    for &m in &cands {
        if m.piece_after().piece_type().0 < 2 {
            continue;
        }
        assert_eq!(
            mb.pseudo_legal(to_mb(m)),
            core.pseudo_legal(m),
            "pseudo_legal {}: {sfen}",
            m.to_usi()
        );
    }
    PREV_MOVES.with(|p| *p.borrow_mut() = legal.clone());
    // gives_check・see_ge: 合法手の全部
    for &m in &legal {
        let mm = to_mb(m);
        assert_eq!(
            mb.gives_check(mm),
            core.gives_check(m),
            "gives_check {}: {sfen}",
            m.to_usi()
        );
        for th in [-1000, -300, -90, 0, 1, 90, 315, 540, 1000] {
            assert_eq!(
                mb.see_ge(mm, th),
                core.see_ge(m, th),
                "see_ge {} {th}: {sfen}",
                m.to_usi()
            );
        }
    }
    // 1手詰めの候補の判定: 合法手の全部で、盤を動かさない判定と手を進める判定（第6回の実装）を比べる
    for &m in &legal {
        let mm = to_mb(m);
        let fast = mb.is_mate_move_for_test(mm);
        assert_eq!(
            fast,
            mb.is_mate_move_by_move(mm),
            "is_mate_move {}: {sfen}",
            m.to_usi()
        );
        MATE_MOVES.fetch_add(u64::from(fast), std::sync::atomic::Ordering::Relaxed);
    }
    // mate_1ply: 王手されていないとき
    if !core.in_check() {
        let a = mate_1ply(core).map(|m| m.to_usi());
        let b = mb.mate_1ply().map(|m| m.to_usi());
        assert_eq!(b, a, "mate_1ply: {sfen}");
    }
}

/// 段階6（探索への接続）で使う値をcoreと比べる。
///
/// - 置換表の `Move16`: 合法手の詰め方と、復元（`to_move`）の往復。よその局面の手と乱数のビット列も復元させる
/// - 利き数の段階の集合: engineの `EffectLevels::build` と升ごとに同じ段階
/// - null move: 進めた後のキー・王手駒・連続王手の数、戻した後の局面
fn check_stage6(core: &mut CorePos, mb: &mut Position) {
    use himawari_core::{Move16, Square};
    let sfen = core.to_sfen();
    assert_eq!(mb.game_ply(), core.game_ply() as u32, "game_ply: {sfen}");
    let mut legal = CoreList::default();
    generate_legal(core, true, &mut legal);
    let mut bits: Vec<u16> = Vec::new();
    for &m in legal.as_slice() {
        let mm = to_mb(m);
        let b = m.to_move16().0;
        assert_eq!(mm.to_move16(), b, "to_move16 {}: {sfen}", m.to_usi());
        assert_eq!(
            mb.move_from16(b),
            Some(mm),
            "move_from16 {}: {sfen}",
            m.to_usi()
        );
        bits.push(b);
    }
    PREV_MOVES.with(|p| bits.extend(p.borrow().iter().map(|m| m.to_move16().0)));
    let mut x = core.key() | 1;
    for _ in 0..64 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        bits.push(x as u16);
    }
    for b in bits {
        assert_eq!(
            mb.move_from16(b),
            core.to_move(Move16(b)).map(to_mb),
            "move_from16 {b:#06x}: {sfen}"
        );
    }
    let want = himawari_engine::nnue::EffectLevels::build(core);
    let got = mb.effect_levels();
    for (ci, c) in [himawari_core::Color::Black, himawari_core::Color::White]
        .into_iter()
        .enumerate()
    {
        for s in 0..81u8 {
            let lv = got[ci].iter().map(|g| (g >> s) as u8 & 1).sum::<u8>();
            assert_eq!(
                lv,
                want.level(c, Square::from_index(s)),
                "effect_levels 色{ci} 升{s}: {sfen}"
            );
        }
    }
    check_stage6_rules(core, mb);
    if !core.in_check() {
        core.do_null_move();
        mb.do_null_move();
        check_state(core, mb);
        assert_eq!(mb.in_check(), core.in_check(), "null: {sfen}");
        assert_eq!(
            mb.game_ply(),
            core.game_ply() as u32,
            "null game_ply: {sfen}"
        );
        core.undo_null_move();
        mb.undo_null_move();
        assert_eq!(mb.to_sfen(), sfen, "undo_null_moveで戻らない");
        check_state(core, mb);
    }
}

/// coreの升の集合を81升のビット列へ。
fn bits(b: himawari_core::Bitboard) -> u128 {
    let mut x = 0u128;
    for sq in b {
        x |= 1u128 << sq.index();
    }
    x
}

/// 第13回の規則の関数をcoreと比べる。千日手・入玉宣言・`check_squares`・駒種ごとの升の集合。
///
/// 千日手は、配列盤とcoreが同じ手順で進んでいれば同じ過去の局面を持つので、そのまま比べられる。
fn check_stage6_rules(core: &CorePos, mb: &Position) {
    use himawari_core::{Color as CC, PieceType};
    let sfen = core.to_sfen();
    for pt in 2..=15u8 {
        assert_eq!(
            mb.check_squares(pt),
            bits(core.check_squares(PieceType(pt))),
            "check_squares {pt}: {sfen}"
        );
    }
    assert_eq!(
        mb.can_declare_win(),
        core.can_declare_win(),
        "can_declare_win: {sfen}"
    );
    let sets = mb.piece_sets();
    for (ci, c) in [CC::Black, CC::White].into_iter().enumerate() {
        for pt in 2..=15u8 {
            assert_eq!(
                sets[ci][pt as usize],
                bits(core.pieces(c, PieceType(pt))),
                "piece_sets 色{ci} {pt}: {sfen}"
            );
        }
        let gold_like = [2u8, 3, 4, 5, 9]
            .iter()
            .fold(0u128, |a, &pt| a | bits(core.pieces(c, PieceType(pt))));
        assert_eq!(sets[ci][1], gold_like, "piece_sets 色{ci} 金の動き: {sfen}");
    }
    for ply in [0usize, 1, 4, 8, 64] {
        assert_eq!(
            format!("{:?}", mb.repetition_state(ply)),
            format!("{:?}", core.repetition_state(ply)),
            "repetition_state {ply}: {sfen}"
        );
    }
}

fn check_node(core: &mut CorePos, mb: &mut Position) {
    check_stage6(core, mb);
    check_stage5(core, mb);
    assert_eq!(mb.to_sfen(), core.to_sfen());
    if let Err(e) = mb.check_consistency() {
        panic!("{e}: {}", mb.to_sfen());
    }
    check_state(core, mb);
    assert_eq!(mb.in_check(), core.in_check(), "{}", core.to_sfen());
    check_key_after(mb);
    let mut kinds = vec![(None, None)];
    if !core.in_check() {
        kinds.push((Some(CoreGen::Captures), Some(GenType::Captures)));
        kinds.push((Some(CoreGen::Quiets), Some(GenType::Quiets)));
    }
    for (cg, mg) in kinds {
        for all in [true, false] {
            let mut a = core_seq(core, cg, all);
            let mut b = mb_seq(mb, mg, all);
            // 並びの変わる構成（HIMAWARI_MB_ORDER=type）では、手の集合で比べる
            if !himawari_mailbox::ORDER_MATCHES_CORE {
                a.sort();
                b.sort();
            }
            if a != b {
                let missing: Vec<_> = a.iter().filter(|m| !b.contains(m)).collect();
                let extra: Vec<_> = b.iter().filter(|m| !a.contains(m)).collect();
                panic!(
                    "生成の列の不一致（{mg:?}, all={all}） {}\n  足りない: {missing:?}\n  余分: {extra:?}\n  core: {a:?}\n  配列盤: {b:?}",
                    core.to_sfen()
                );
            }
        }
    }
}

/// 進める前に求めたキー（`key_after`）が、進めた後の `key` と一致する（ADR-0250）。
fn check_key_after(mb: &mut Position) {
    let mut list = MoveList::default();
    mb.generate_legal(true, &mut list);
    for &m in list.as_slice() {
        let want = mb.key_after(m);
        mb.do_move(m);
        assert_eq!(mb.key(), want, "key_after: {} {}", m.to_usi(), mb.to_sfen());
        mb.undo_move(m);
    }
}

fn find(mb: &mut Position, usi: &str) -> himawari_mailbox::Move {
    let mut list = MoveList::default();
    mb.generate_legal(true, &mut list);
    *list
        .as_slice()
        .iter()
        .find(|m| m.to_usi() == usi)
        .unwrap_or_else(|| panic!("手が無い: {usi}"))
}

fn walk(core: &mut CorePos, mb: &mut Position, depth: u32) {
    check_node(core, mb);
    if depth == 0 {
        return;
    }
    for usi in core_moves(core) {
        let cm = core.move_from_usi(&usi).unwrap();
        let mm = find(mb, &usi);
        let before = mb.to_sfen();
        core.do_move(cm);
        mb.do_move(mm);
        walk(core, mb, depth - 1);
        core.undo_move(cm);
        mb.undo_move(mm);
        assert_eq!(mb.to_sfen(), before, "undoで戻らない: {usi}");
        mb.check_consistency().unwrap();
    }
}

/// Zobristの乱数表がcoreと1ビットも違わない。
#[test]
fn zobrist_tables_match_core() {
    use himawari_core::{Piece, Square, zobrist};
    use himawari_mailbox::zobrist::{HAND_PAWN, PSQ};
    for p in 0..32u8 {
        for sq in 0..81u8 {
            let core = if p % 16 >= 2 {
                zobrist::psq(Piece(p), Square::from_index(sq))
            } else {
                0
            };
            assert_eq!(PSQ[p as usize][sq as usize], core, "PSQ[{p}][{sq}]");
        }
    }
    for (c, color) in [himawari_core::Color::Black, himawari_core::Color::White]
        .into_iter()
        .enumerate()
    {
        for n in 0..19u32 {
            assert_eq!(HAND_PAWN[c][n as usize], zobrist::hand_pawn(color, n));
        }
    }
}

#[test]
fn sfen_roundtrip_matches_core() {
    for sfen in CASES.iter().chain(EXTRA.iter()) {
        let core = CorePos::from_sfen(sfen).unwrap();
        let mb = Position::from_sfen(sfen).unwrap();
        assert_eq!(mb.to_sfen(), core.to_sfen());
    }
}

#[test]
fn every_node_matches_core_shallow() {
    let depths = [3, 2, 2, 2, 1, 2];
    for (sfen, d) in CASES.iter().zip(depths) {
        let mut core = CorePos::from_sfen(sfen).unwrap();
        let mut mb = Position::from_sfen(sfen).unwrap();
        walk(&mut core, &mut mb, d);
    }
    for sfen in EXTRA {
        let mut core = CorePos::from_sfen(sfen).unwrap();
        let mut mb = Position::from_sfen(sfen).unwrap();
        walk(&mut core, &mut mb, 3);
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

/// ランダムに指し進め、全手で突き合わせる。終局（合法手なし）か上限の手数で止める。
/// 届いた局面の種類を数え、検査が王手・駒打ち・詰みまで届いたことを確かめる。
fn playouts(games: u32, max_ply: u32, seed: u64) {
    let mut rng = Rng(seed);
    MATE_MOVES.store(0, std::sync::atomic::Ordering::Relaxed);
    let (mut nodes, mut checks, mut drops, mut mates) = (0u64, 0u64, 0u64, 0u64);
    for g in 0..games {
        let start = CASES[g as usize % CASES.len()];
        let mut core = CorePos::from_sfen(start).unwrap();
        let mut mb = Position::from_sfen(start).unwrap();
        let mut history = Vec::new();
        for _ in 0..max_ply {
            check_node(&mut core, &mut mb);
            nodes += 1;
            checks += u64::from(core.in_check());
            let moves = core_moves(&core);
            if moves.is_empty() {
                mates += 1;
                break;
            }
            drops += moves.iter().filter(|m| m.contains('*')).count() as u64;
            let usi = moves[(rng.next() % moves.len() as u64) as usize].clone();
            let cm = core.move_from_usi(&usi).unwrap();
            let mm = find(&mut mb, &usi);
            core.do_move(cm);
            mb.do_move(mm);
            history.push((cm, mm));
        }
        while let Some((cm, mm)) = history.pop() {
            core.undo_move(cm);
            mb.undo_move(mm);
            assert_eq!(mb.to_sfen(), core.to_sfen());
        }
        mb.check_consistency().unwrap();
    }
    println!(
        "局面 {nodes}、王手 {checks}、駒打ちの合法手 {drops}、詰み {mates}、詰みと判定した合法手 {}",
        MATE_MOVES.load(std::sync::atomic::Ordering::Relaxed)
    );
    assert!(checks > 0 && drops > 0 && mates > 0);
}

/// coreの過去の局面のキー（千日手の判定の材料）。
fn history_of(core: &CorePos) -> Vec<himawari_mailbox::HistoryKey> {
    core.state_history()
        .iter()
        .map(|s| himawari_mailbox::HistoryKey {
            board_key: s.board_key,
            hand_key: s.hand_key,
            continuous_check: s.continuous_check,
            plies_from_null: s.plies_from_null,
        })
        .collect()
}

/// 千日手の判定をcoreと比べる（第13回）。
///
/// ランダムな対局では同じ局面がほとんど現れないので、2手前の自分の手を戻す手を4回に3回選び、
/// 同じ局面を繰り返させる。手順を共有する配列盤のほか、SFENから作って過去の局面のキーを
/// `import_history` で写した配列盤（探索の根の作り方）でも、同じ判定になることを確かめる。
#[test]
fn repetition_matches_core() {
    let mut rng = Rng(0x5151_2026_1020_0001);
    let mut seen = std::collections::BTreeMap::new();
    for g in 0..300usize {
        let start = CASES[g % CASES.len()];
        let mut core = CorePos::from_sfen(start).unwrap();
        let mut mb = Position::from_sfen(start).unwrap();
        let mut played: Vec<CoreMove> = Vec::new();
        for _ in 0..120 {
            let mut fresh = Position::from_sfen(&core.to_sfen()).unwrap();
            fresh.import_history(&history_of(&core));
            for ply in [0usize, 1, 4, 8, 64] {
                let want = format!("{:?}", core.repetition_state(ply));
                let sfen = core.to_sfen();
                assert_eq!(
                    format!("{:?}", mb.repetition_state(ply)),
                    want,
                    "手順を共有した配列盤 {ply}: {sfen}"
                );
                assert_eq!(
                    format!("{:?}", fresh.repetition_state(ply)),
                    want,
                    "履歴を写した配列盤 {ply}: {sfen}"
                );
            }
            *seen
                .entry(format!("{:?}", core.repetition_state(64)))
                .or_insert(0u32) += 1;
            let mut legal = CoreList::default();
            generate_legal(&core, true, &mut legal);
            if legal.is_empty() {
                break;
            }
            let mut pick = None;
            if played.len() >= 2 && !rng.next().is_multiple_of(4) {
                let m = played[played.len() - 2];
                if !m.is_drop() && !m.is_promote() {
                    pick = legal.as_slice().iter().copied().find(|x| {
                        !x.is_drop()
                            && !x.is_promote()
                            && x.from_sq() == m.to()
                            && x.to() == m.from_sq()
                    });
                }
            }
            let m = pick
                .unwrap_or_else(|| legal.as_slice()[(rng.next() % legal.len() as u64) as usize]);
            core.do_move(m);
            mb.do_move(to_mb(m));
            played.push(m);
        }
    }
    println!("千日手の判定の内訳（ply=64）: {seen:?}");
    assert!(seen.get("Draw").copied().unwrap_or(0) > 100);
}

#[test]
fn random_playouts_match_core() {
    playouts(60, 300, 0x9E37_79B9_7F4A_7C15);
}

#[test]
#[ignore = "重い。cargo test --release -- --ignored で走らせる"]
fn random_playouts_match_core_long() {
    playouts(3000, 400, 0x2545_F491_4F6C_DD1D);
}

fn perft_vs_core(sfen: &str, depth: u32) {
    let mut core = CorePos::from_sfen(sfen).unwrap();
    let mut mb = Position::from_sfen(sfen).unwrap();
    let a = himawari_core::perft(&mut core, depth);
    let b = perft(&mut mb, depth);
    println!("perft {depth} = {b} (core {a}): {sfen}");
    assert_eq!(a, b, "perftの不一致 depth={depth}: {sfen}");
}

#[test]
fn perft_matches_core_shallow() {
    for sfen in CASES.iter().chain(EXTRA.iter()) {
        perft_vs_core(sfen, 3);
    }
}

#[test]
#[ignore = "重い。cargo test --release -- --ignored で走らせる"]
fn perft_matches_core_deep() {
    let depths = [5, 4, 4, 4, 3, 4];
    for (sfen, d) in CASES.iter().zip(depths) {
        perft_vs_core(sfen, d);
    }
    for sfen in EXTRA {
        perft_vs_core(sfen, 5);
    }
}

/// 打ち歩詰めの規則。詰む歩打ちは出さず、詰まない歩打ちは出す。
#[test]
fn pawn_drop_mate_rule() {
    let mut pinned = Position::from_sfen(EXTRA[7]).unwrap();
    assert!(!mb_seq(&mut pinned, None, true).contains(&"P*1b".to_string()));
    let mut blocked = Position::from_sfen(EXTRA[8]).unwrap();
    assert!(mb_seq(&mut blocked, None, true).contains(&"P*5b".to_string()));
    let mut mate = Position::from_sfen(EXTRA[0]).unwrap();
    let moves = mb_seq(&mut mate, None, true);
    assert!(!moves.contains(&"P*1b".to_string()));
    assert!(moves.contains(&"P*1c".to_string()));
    let mut not_mate = Position::from_sfen(EXTRA[1]).unwrap();
    assert!(mb_seq(&mut not_mate, None, true).contains(&"P*1b".to_string()));
}

#[test]
fn startpos_perft_known_values() {
    let mut mb = Position::from_sfen(SFEN_STARTPOS).unwrap();
    assert_eq!(perft(&mut mb, 4), 719_731);
}

/// 疎な乱数局面を1つ作る。coreの mate_tests.rs の生成器をそのまま写した
/// （玉2枚と数駒、先手番、玉の近くに攻め駒）。1手詰めの照合に使う。
fn random_position(rng: &mut Rng) -> Option<CorePos> {
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

    // 遠くの飛角香も置く。開き王手と合駒の経路を通すため
    let sliders = ['R', 'B', 'L', '+', 'D'];
    for _ in 0..(rng.next() % 3) {
        let sq = (rng.next() % 81) as usize;
        if board[sq].is_some() {
            continue;
        }
        let c = sliders[(rng.next() % sliders.len() as u64) as usize];
        // 先手の香は1段目（段の添字0）に置けない
        if c == 'L' && sq % 9 < 1 {
            continue;
        }
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

    let pos = CorePos::from_sfen(&s).ok()?;
    // 先手番なので先手玉に王手がかかっていてはならない
    if pos.in_check() {
        return None;
    }
    // 後手玉に王手がかかっている局面は非合法（手番側が取れてしまう）
    let occ = pos.occupied();
    let wksq = pos.king(himawari_core::Color::White);
    if !pos
        .attackers_to(himawari_core::Color::Black, wksq, occ)
        .is_empty()
    {
        return None;
    }
    Some(pos)
}

/// 疎な乱数局面で、全部の検査（段階5の関数と1手詰めを含む）をcoreと突き合わせる。
#[test]
fn sparse_positions_match_core() {
    let mut rng = Rng(0x1234_5678_9ABC_DEF0);
    MATE_MOVES.store(0, std::sync::atomic::Ordering::Relaxed);
    let mut positions = 0u32;
    let mut mates = 0u32;
    while positions < 20_000 {
        let Some(mut core) = random_position(&mut rng) else {
            continue;
        };
        positions += 1;
        let mut mb = Position::from_sfen(&core.to_sfen()).unwrap();
        check_node(&mut core, &mut mb);
        mates += u32::from(mate_1ply(&core).is_some());
    }
    println!(
        "疎な乱数局面 {positions}、1手詰めあり {mates}、詰みと判定した合法手 {}",
        MATE_MOVES.load(std::sync::atomic::Ordering::Relaxed)
    );
    assert!(mates > 1000);
}

/// 実戦の棋譜（openings/actdump_positions.txt）を再生し、全手で全部の検査をする。
#[test]
fn game_records_match_core() {
    let text = include_str!("../../../openings/actdump_positions.txt");
    MATE_MOVES.store(0, std::sync::atomic::Ordering::Relaxed);
    let (mut nodes, mut mates) = (0u32, 0u32);
    for line in text.lines().filter(|l| l.starts_with("sfen ")).take(60) {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let sfen = tokens[1..5].join(" ");
        let mut core = CorePos::from_sfen(&sfen).unwrap();
        let mut mb = Position::from_sfen(&sfen).unwrap();
        for usi in &tokens[6..] {
            check_node(&mut core, &mut mb);
            nodes += 1;
            mates += u32::from(!core.in_check() && mate_1ply(&core).is_some());
            let Some(cm) = core.move_from_usi(usi) else {
                break;
            };
            let mm = find(&mut mb, usi);
            core.do_move(cm);
            mb.do_move(mm);
        }
    }
    println!(
        "棋譜の局面 {nodes}、1手詰めあり {mates}、詰みと判定した合法手 {}",
        MATE_MOVES.load(std::sync::atomic::Ordering::Relaxed)
    );
    assert!(nodes > 5000);
}
