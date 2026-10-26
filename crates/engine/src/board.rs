//! 探索が手を進め戻す盤面（ADR-0239の手順0）。
//!
//! 探索のワーカーと、盤面操作だけを測る `boardbench` が同じ進め戻しを
//! 呼ぶために1か所へ集める。利きの表のように局面に付いて回る状態を
//! 足すときは、この型に持たせ、`do_move` などへ追従を書く。そうすれば
//! 探索とミクロな計測の両方が同じ費用を払う。
//!
//! 探索が盤に問うことは、すべてこの型のメソッドにしてある（ADR-0241の第12回・第13回）。
//! 構成はengineのフィーチャで選び、build.rsが次のcfgへ直す。
//!
//! - 既定（フィーチャ `mailbox`、cfg `mb_search`）: coreの局面を持たず、配列盤
//!   （`himawari_mailbox::Position`）だけで探索する。千日手のために、根の局面を作るときに
//!   coreの過去の局面のキーを写す
//! - `core-search`: coreの `Position` で探索する。比較と照合のために残す。`mailbox` より優先する。
//!   この構成だけ `Deref` で `Position` のほかのメソッドも読めるが、探索と評価関数はそれを使わない
//! - `legal-gen`: `core-search` で、擬似合法手の生成を合法手で濾す統制群
//! - `mailbox-shadow`（cfg `mb_shadow`）: 配列盤に加えてcoreの局面も並べて持ち、同じ手で進め戻す。
//!   デバッグビルドでは、配列盤の答えを毎回coreの答えと比べる
//!
//! `DerefMut` は実装しない。局面を可変で借りる経路を、この型の進め戻しに限るためである。

#[cfg(not(mb_search))]
use std::ops::Deref;

use himawari_core::bonapiece::BonaBits;
use himawari_core::{
    Bitboard, Color, DirtyPiece, GenType, Move, Move16, MoveList, Piece, PieceType, Position,
    Repetition, Square,
};

use crate::nnue::EffectLevels;
use crate::nnue_acc::EvalBoard;

/// 探索が進め戻す盤面。
#[derive(Clone)]
pub struct SearchBoard {
    #[cfg(any(not(mb_search), mb_shadow))]
    pos: Position,
    #[cfg(mb_search)]
    mb: himawari_mailbox::Position,
}

impl SearchBoard {
    pub fn new(pos: Position) -> SearchBoard {
        #[cfg(mb_search)]
        let mb = mbconv::from_core(&pos);
        SearchBoard {
            #[cfg(any(not(mb_search), mb_shadow))]
            pos,
            #[cfg(mb_search)]
            mb,
        }
    }

    /// 包んでいる局面（coreの局面を持つ構成だけ）。
    #[cfg(any(not(mb_search), mb_shadow))]
    #[inline(always)]
    pub fn position(&self) -> &Position {
        &self.pos
    }

    /// いまの局面をcoreの局面として返す。ツールが局面を書き出すときに使う。
    ///
    /// 配列盤の構成ではSFENを経由して作るので、過去の局面（千日手の材料）は持たない。
    pub fn to_position(&self) -> Position {
        #[cfg(any(not(mb_search), mb_shadow))]
        return self.pos.clone();
        #[cfg(all(mb_search, not(mb_shadow)))]
        Position::from_sfen(&self.mb.to_sfen()).expect("配列盤の出したSFENをcoreが読めない")
    }

    /// 1手進める。
    #[inline(always)]
    pub fn do_move(&mut self, m: Move) {
        #[cfg(any(not(mb_search), mb_shadow))]
        self.pos.do_move(m);
        #[cfg(mb_search)]
        self.mb.do_move(mbconv::to_mb(m));
    }

    /// `do_move` を戻す。
    #[inline(always)]
    pub fn undo_move(&mut self, m: Move) {
        #[cfg(any(not(mb_search), mb_shadow))]
        self.pos.undo_move(m);
        #[cfg(mb_search)]
        self.mb.undo_move(mbconv::to_mb(m));
    }

    /// null moveで1手進める。
    #[inline(always)]
    pub fn do_null_move(&mut self) {
        #[cfg(any(not(mb_search), mb_shadow))]
        self.pos.do_null_move();
        #[cfg(mb_search)]
        self.mb.do_null_move();
    }

    /// `do_null_move` を戻す。
    #[inline(always)]
    pub fn undo_null_move(&mut self) {
        #[cfg(any(not(mb_search), mb_shadow))]
        self.pos.undo_null_move();
        #[cfg(mb_search)]
        self.mb.undo_null_move();
    }
}

/// 盤への問い合わせ。既定ではcoreの同名の関数へそのまま渡す。
#[cfg(not(mb_search))]
impl SearchBoard {
    /// 擬似合法手を生成する。
    ///
    /// フィーチャ `legal-gen` では、coreの列を合法手で濾してから返す。配列盤の生成が
    /// 合法手だけを出すことによる木の変化を、配列盤なしで再現する統制群である（ADR-0241の第12回）。
    #[inline(always)]
    pub fn generate(&self, gt: GenType, all: bool, list: &mut MoveList) {
        if cfg!(feature = "legal-gen") {
            let mut pseudo = MoveList::default();
            himawari_core::generate(&self.pos, gt, all, &mut pseudo);
            for &m in pseudo.as_slice() {
                if self.pos.is_legal(m) {
                    list.push(m);
                }
            }
        } else {
            himawari_core::generate(&self.pos, gt, all, list);
        }
    }

    /// 合法手を生成する。
    #[inline(always)]
    pub fn generate_legal(&self, all: bool, list: &mut MoveList) {
        himawari_core::generate_legal(&self.pos, all, list);
    }

    /// 擬似合法手mが合法か。
    #[inline(always)]
    pub fn is_legal(&self, m: Move) -> bool {
        self.pos.is_legal(m)
    }

    /// 指し手の選び（`MovePicker`）が返した手mが合法か。from_ttは置換表の手か。
    /// coreの生成は擬似合法手を出すので、どの手も判定する。
    #[inline(always)]
    pub fn is_legal_picked(&self, m: Move, _from_tt: bool) -> bool {
        self.pos.is_legal(m)
    }

    /// mがこの局面の擬似合法手か（置換表・キラーの手の検査）。
    #[inline(always)]
    pub fn pseudo_legal(&self, m: Move) -> bool {
        self.pos.pseudo_legal(m)
    }

    /// 合法手mが王手になるか。
    #[inline(always)]
    pub fn gives_check(&self, m: Move) -> bool {
        self.pos.gives_check(m)
    }

    /// 静的交換評価が閾値以上か。
    #[inline(always)]
    pub fn see_ge(&self, m: Move, threshold: i32) -> bool {
        self.pos.see_ge(m, threshold)
    }

    /// 1手詰めの手（王手されていない局面で呼ぶ）。
    #[inline(always)]
    pub fn mate_1ply(&self) -> Option<Move> {
        crate::mate::mate_1ply(&self.pos)
    }

    /// 置換表の `Move16` を完全な手へ戻す。
    #[inline(always)]
    pub fn to_move(&self, m16: Move16) -> Option<Move> {
        self.pos.to_move(m16)
    }

    /// 評価関数の利き数の入力が使う、段階の集合（ADR-0237のD2）。駒ごとに利き先を数えて作る。
    #[inline(always)]
    pub fn effect_levels(&self) -> EffectLevels {
        EffectLevels::build(&self.pos)
    }

    #[inline(always)]
    pub fn piece_on(&self, sq: Square) -> Piece {
        self.pos.piece_on(sq)
    }

    #[inline(always)]
    pub fn side_to_move(&self) -> Color {
        self.pos.side_to_move()
    }

    #[inline(always)]
    pub fn in_check(&self) -> bool {
        self.pos.in_check()
    }

    #[inline(always)]
    pub fn king(&self, c: Color) -> Square {
        self.pos.king(c)
    }

    #[inline(always)]
    pub fn key(&self) -> u64 {
        self.pos.key()
    }

    #[inline(always)]
    pub fn pawn_key(&self) -> u64 {
        self.pos.pawn_key()
    }

    #[inline(always)]
    pub fn minor_piece_key(&self) -> u64 {
        self.pos.minor_piece_key()
    }

    #[inline(always)]
    pub fn non_pawn_key(&self, c: Color) -> u64 {
        self.pos.non_pawn_key(c)
    }

    /// 直前の手で取った駒（無ければ空）。
    #[inline(always)]
    pub fn captured(&self) -> Piece {
        self.pos.state().captured
    }

    /// null moveからの手数。
    #[inline(always)]
    pub fn plies_from_null(&self) -> u16 {
        self.pos.state().plies_from_null
    }

    #[inline(always)]
    pub fn game_ply(&self) -> u16 {
        self.pos.game_ply()
    }

    #[inline(always)]
    pub fn can_declare_win(&self) -> bool {
        self.pos.can_declare_win()
    }

    #[inline(always)]
    pub fn repetition_state(&self, ply: usize) -> Repetition {
        self.pos.repetition_state(ply)
    }

    /// 手番側の駒種ptが置けば相手玉へ直接王手になる升。
    #[inline(always)]
    pub fn check_squares(&self, pt: PieceType) -> Bitboard {
        self.pos.check_squares(pt)
    }

    pub fn to_sfen(&self) -> String {
        self.pos.to_sfen()
    }
}

#[cfg(not(mb_search))]
impl EvalBoard for SearchBoard {
    #[inline(always)]
    fn side_to_move(&self) -> Color {
        self.pos.side_to_move()
    }
    #[inline(always)]
    fn king(&self, c: Color) -> Square {
        self.pos.king(c)
    }
    #[inline(always)]
    fn hand_count(&self, owner: Color, pt: PieceType) -> u32 {
        self.pos.hand(owner).count(pt)
    }
    #[inline(always)]
    fn piece_on(&self, sq: Square) -> Piece {
        self.pos.piece_on(sq)
    }
    #[inline(always)]
    fn dirty(&self) -> DirtyPiece {
        self.pos.state().dirty
    }
    #[inline(always)]
    fn bona_piece_bits(&self, c: Color, out: &mut BonaBits) {
        himawari_core::bonapiece::bona_piece_bits(&self.pos, c, out);
    }
    #[inline(always)]
    fn effect_levels(&self) -> EffectLevels {
        EffectLevels::build(&self.pos)
    }
    #[inline(always)]
    fn effect_codes(&self, out: &mut [u16; 81]) {
        self.pos.effect_codes(out);
    }
    #[inline(always)]
    fn material(&self) -> i32 {
        self.pos.state().material
    }
    #[inline(always)]
    fn core_position(&self) -> Option<&Position> {
        Some(&self.pos)
    }
}

#[cfg(not(mb_search))]
impl Deref for SearchBoard {
    type Target = Position;

    #[inline(always)]
    fn deref(&self) -> &Position {
        &self.pos
    }
}

/// 盤への問い合わせを配列盤が答える版（ADR-0241の段階6）。
///
/// 返す値はどれもcoreと同じになるように作ってある（等価な版）。
/// `mailbox-shadow` のデバッグビルドでは毎回coreの答えと比べ、食い違えば局面を添えて止める。
#[cfg(mb_search)]
impl SearchBoard {
    /// coreの局面と比べる（`mailbox-shadow` のデバッグビルドだけ）。
    #[inline(always)]
    #[allow(unused_variables)]
    fn shadow<T: PartialEq + std::fmt::Debug>(
        &self,
        what: &str,
        got: &T,
        want: impl FnOnce(&Position) -> T,
    ) {
        #[cfg(mb_shadow)]
        if cfg!(debug_assertions) {
            let w = want(&self.pos);
            assert_eq!(*got, w, "{what}: {}", self.pos.to_sfen());
        }
    }

    /// 擬似合法手を生成する。列はcoreの `generate` を合法手で濾した列と同じになる。
    ///
    /// 配列盤の生成は合法手だけを出す（ADR-0241の第2回）。探索はこのあと `is_legal` で
    /// 濾すので、手の数え方は変わらない。並びが変わる分は統制群 `legal-gen` で再現できる。
    #[inline(always)]
    pub fn generate(&self, gt: GenType, all: bool, list: &mut MoveList) {
        let g = match gt {
            GenType::Captures => himawari_mailbox::GenType::Captures,
            GenType::Quiets => himawari_mailbox::GenType::Quiets,
            GenType::Evasions => himawari_mailbox::GenType::Evasions,
            GenType::NonEvasions => himawari_mailbox::GenType::NonEvasions,
        };
        self.mb.generate(g, all, &mut mbconv::CoreSink::new(list));
        // 照合の引数は、照合するビルドでだけ作る。作るだけで確保が走る
        #[cfg(mb_shadow)]
        self.shadow("generate", &mbconv::order_key(list.as_slice()), |p| {
            let mut want = MoveList::default();
            himawari_core::generate(p, gt, all, &mut want);
            let want: Vec<Move> = want
                .as_slice()
                .iter()
                .copied()
                .filter(|&m| p.is_legal(m))
                .collect();
            mbconv::order_key(&want)
        });
    }

    /// 合法手を生成する。
    #[inline(always)]
    pub fn generate_legal(&self, all: bool, list: &mut MoveList) {
        self.mb
            .generate_legal(all, &mut mbconv::CoreSink::new(list));
        #[cfg(mb_shadow)]
        self.shadow("generate_legal", &mbconv::order_key(list.as_slice()), |p| {
            let mut want = MoveList::default();
            himawari_core::generate_legal(p, all, &mut want);
            mbconv::order_key(want.as_slice())
        });
    }

    /// 擬似合法手mが合法か。
    #[inline(always)]
    pub fn is_legal(&self, m: Move) -> bool {
        let r = self.mb.is_legal(mbconv::to_mb(m));
        self.shadow("is_legal", &r, |p| p.is_legal(m));
        r
    }

    /// 指し手の選び（`MovePicker`）が返した手mが合法か。from_ttは置換表の手か。
    ///
    /// 配列盤の生成は合法手だけを出すので、置換表の手でなければ判定しない（ADR-0241の第15回）。
    /// 置換表の手は擬似合法性しか確かめていないので、判定する。
    #[inline(always)]
    pub fn is_legal_picked(&self, m: Move, from_tt: bool) -> bool {
        if from_tt {
            return self.is_legal(m);
        }
        debug_assert!(
            self.mb.is_legal(mbconv::to_mb(m)),
            "生成した手が合法でない {}: {}",
            m.to_usi(),
            self.to_sfen()
        );
        true
    }

    /// mがこの局面の擬似合法手か（置換表・キラーの手の検査）。
    #[inline(always)]
    pub fn pseudo_legal(&self, m: Move) -> bool {
        // 特殊値と駒の無い手は配列盤の手に直せない。coreはどちらにも偽を返す
        // （駒の無い手は移動元の駒と合わない）
        if m.is_special() || m.piece_after().is_empty() {
            self.shadow("pseudo_legal（駒の無い手）", &false, |p| {
                p.pseudo_legal(m)
            });
            return false;
        }
        let r = self.mb.pseudo_legal(mbconv::to_mb(m));
        self.shadow("pseudo_legal", &r, |p| p.pseudo_legal(m));
        r
    }

    /// 合法手mが王手になるか。
    #[inline(always)]
    pub fn gives_check(&self, m: Move) -> bool {
        let r = self.mb.gives_check(mbconv::to_mb(m));
        self.shadow("gives_check", &r, |p| p.gives_check(m));
        r
    }

    /// 静的交換評価が閾値以上か。
    #[inline(always)]
    pub fn see_ge(&self, m: Move, threshold: i32) -> bool {
        let r = self.mb.see_ge(mbconv::to_mb(m), threshold);
        self.shadow("see_ge", &r, |p| p.see_ge(m, threshold));
        r
    }

    /// 1手詰めの手（王手されていない局面で呼ぶ）。
    #[inline(always)]
    pub fn mate_1ply(&self) -> Option<Move> {
        let r = self.mb.mate_1ply().map(mbconv::to_core);
        self.shadow("mate_1ply", &r, crate::mate::mate_1ply);
        r
    }

    /// 置換表の `Move16` を完全な手へ戻す。
    #[inline(always)]
    pub fn to_move(&self, m16: Move16) -> Option<Move> {
        let r = self.mb.move_from16(m16.0).map(mbconv::to_core);
        self.shadow("to_move", &r, |p| p.to_move(m16));
        r
    }

    /// 評価関数の利き数の入力が使う、段階の集合（ADR-0237のD2）。
    ///
    /// 配列盤は利き数を升ごとに持つので、駒ごとに利き先を数え直さず、81升を読むだけで作れる。
    #[inline(always)]
    pub fn effect_levels(&self) -> EffectLevels {
        let r = EffectLevels::from_bits(self.mb.effect_levels());
        self.shadow("effect_levels", &r, EffectLevels::build);
        r
    }

    #[inline(always)]
    pub fn piece_on(&self, sq: Square) -> Piece {
        let r = Piece(self.mb.piece_on(mbconv::idx(sq)));
        self.shadow("piece_on", &r, |p| p.piece_on(sq));
        r
    }

    #[inline(always)]
    pub fn side_to_move(&self) -> Color {
        mbconv::color(self.mb.side_to_move())
    }

    #[inline(always)]
    pub fn in_check(&self) -> bool {
        let r = self.mb.in_check();
        self.shadow("in_check", &r, |p| p.in_check());
        r
    }

    #[inline(always)]
    pub fn king(&self, c: Color) -> Square {
        mbconv::sq(self.mb.king_square(mbconv::mb_color(c)))
    }

    #[inline(always)]
    pub fn key(&self) -> u64 {
        let r = self.mb.key();
        self.shadow("key", &r, |p| p.key());
        r
    }

    #[inline(always)]
    pub fn pawn_key(&self) -> u64 {
        self.mb.state().pawn_key
    }

    #[inline(always)]
    pub fn minor_piece_key(&self) -> u64 {
        self.mb.state().minor_piece_key
    }

    #[inline(always)]
    pub fn non_pawn_key(&self, c: Color) -> u64 {
        self.mb.state().non_pawn_key[c.index()]
    }

    /// 直前の手で取った駒（無ければ空）。
    #[inline(always)]
    pub fn captured(&self) -> Piece {
        Piece(self.mb.state().captured)
    }

    /// null moveからの手数。
    #[inline(always)]
    pub fn plies_from_null(&self) -> u16 {
        let r = self.mb.state().plies_from_null;
        self.shadow("plies_from_null", &r, |p| p.state().plies_from_null);
        r
    }

    #[inline(always)]
    pub fn game_ply(&self) -> u16 {
        self.mb.game_ply() as u16
    }

    #[inline(always)]
    pub fn can_declare_win(&self) -> bool {
        let r = self.mb.can_declare_win();
        self.shadow("can_declare_win", &r, |p| p.can_declare_win());
        r
    }

    #[inline(always)]
    pub fn repetition_state(&self, ply: usize) -> Repetition {
        use himawari_mailbox::Repetition as R;
        let r = match self.mb.repetition_state(ply) {
            R::None => Repetition::None,
            R::Draw => Repetition::Draw,
            R::Win => Repetition::Win,
            R::Lose => Repetition::Lose,
            R::Superior => Repetition::Superior,
            R::Inferior => Repetition::Inferior,
        };
        self.shadow("repetition_state", &r, |p| p.repetition_state(ply));
        r
    }

    /// 手番側の駒種ptが置けば相手玉へ直接王手になる升。
    #[inline(always)]
    pub fn check_squares(&self, pt: PieceType) -> Bitboard {
        let r = Bitboard::from_bits(self.mb.check_squares(pt.0));
        self.shadow("check_squares", &r, |p| p.check_squares(pt));
        r
    }

    pub fn to_sfen(&self) -> String {
        self.mb.to_sfen()
    }
}

#[cfg(mb_search)]
impl EvalBoard for SearchBoard {
    #[inline(always)]
    fn side_to_move(&self) -> Color {
        SearchBoard::side_to_move(self)
    }
    #[inline(always)]
    fn king(&self, c: Color) -> Square {
        SearchBoard::king(self, c)
    }
    #[inline(always)]
    fn hand_count(&self, owner: Color, pt: PieceType) -> u32 {
        self.mb.hand_count(mbconv::mb_color(owner), pt.0)
    }
    #[inline(always)]
    fn piece_on(&self, sq: Square) -> Piece {
        SearchBoard::piece_on(self, sq)
    }
    #[inline(always)]
    fn dirty(&self) -> DirtyPiece {
        let r = mbconv::dirty(&self.mb.state().dirty);
        // coreの `DirtyPiece` は比較を持たないので、中身を組にして比べる
        let parts = |d: &DirtyPiece| {
            (
                d.count,
                d.king_moved,
                d.piece_old,
                d.piece_new,
                d.from,
                d.to,
            )
        };
        self.shadow("dirty", &parts(&r), |p| parts(&p.state().dirty));
        r
    }
    #[inline(always)]
    fn bona_piece_bits(&self, c: Color, out: &mut BonaBits) {
        let sets = self.mb.piece_sets();
        himawari_core::bonapiece::bona_piece_bits_with(
            c,
            |owner, pt| self.mb.hand_count(mbconv::mb_color(owner), pt.0),
            |owner, pt| {
                let s = &sets[owner.index()];
                if pt == PieceType::GOLD {
                    // 金の動きをする5駒種（金と成駒4種）の和は、配列盤が添字1に持つ
                    s[1]
                } else {
                    s[pt.0 as usize]
                }
            },
            out,
        );
        self.shadow("bona_piece_bits", &*out, |p| {
            let mut want = [0u128; himawari_core::bonapiece::BP_BLOCKS];
            himawari_core::bonapiece::bona_piece_bits(p, c, &mut want);
            want
        });
    }
    #[inline(always)]
    fn effect_levels(&self) -> EffectLevels {
        SearchBoard::effect_levels(self)
    }
    #[inline(always)]
    fn effect_codes(&self, out: &mut [u16; 81]) {
        self.mb.effect_codes(out);
        self.shadow("effect_codes", &*out, |p| {
            let mut want = [0u16; 81];
            EvalBoard::effect_codes(p, &mut want);
            want
        });
    }
    #[inline(always)]
    fn side_codes(&self, out: &mut [u16; 81]) {
        self.mb.effect_minatt_codes(out);
        self.shadow("side_codes", &*out, |p| {
            let mut want = [0u16; 81];
            EvalBoard::side_codes(p, &mut want);
            want
        });
    }
    #[inline(always)]
    fn material(&self) -> i32 {
        self.mb.state().material
    }
    #[inline(always)]
    fn core_position(&self) -> Option<&Position> {
        #[cfg(mb_shadow)]
        return Some(&self.pos);
        #[cfg(not(mb_shadow))]
        None
    }
}

/// coreの型と配列盤の型の変換。升を81升の通し番号と配列盤の添字の間で直す。
/// 駒の符号と駒種は両者で同じ（OSL配列）。
#[cfg(mb_search)]
mod mbconv {
    use himawari_core::{Color, DirtyPiece, Move, Piece, PieceType, Position, Square};
    use himawari_mailbox::board::{IDX81, SQ81};

    /// 配列盤の生成に、coreの手の列へ直接書かせる受け手。升は81升の通し番号のまま受け取れるので、
    /// 表を引かずにcoreの手の符号になる（ADR-0241の第15回）。
    ///
    /// 作るときに、手の列の残りの容量が1回の生成の上限（配列盤の `MAX_MOVES`）以上あることを確かめる。
    /// 生成は合法手だけを出し、1回で `MAX_MOVES` 手を超えないので、積むときは範囲を検査しない。
    pub struct CoreSink<'a>(&'a mut himawari_core::MoveList);

    impl<'a> CoreSink<'a> {
        #[inline(always)]
        pub fn new(list: &'a mut himawari_core::MoveList) -> Self {
            assert!(
                list.len() + himawari_mailbox::movegen::MAX_MOVES
                    <= himawari_core::MoveList::CAPACITY,
                "手の列の残りの容量が1回の生成の上限より小さい"
            );
            CoreSink(list)
        }
    }

    impl himawari_mailbox::MoveSink for CoreSink<'_> {
        #[inline(always)]
        fn push_normal(&mut self, from81: u8, to81: u8, promote: bool, piece_after: u8) {
            let m = Move::new_move(
                Square::from_index(from81),
                Square::from_index(to81),
                promote,
                Piece(piece_after),
            );
            // SAFETY: `new` が残りの容量を確かめた。1回の生成で積むのはMAX_MOVES手以内である
            unsafe { self.0.push_unchecked(m) };
        }

        #[inline(always)]
        fn push_drop(&mut self, pt: u8, to81: u8, piece: u8) {
            let c = if piece & 16 != 0 {
                Color::White
            } else {
                Color::Black
            };
            let m = Move::new_drop(PieceType(pt), Square::from_index(to81), c);
            // SAFETY: `push_normal` と同じ
            unsafe { self.0.push_unchecked(m) };
        }
    }

    /// 生成の列を照合するための形。並びがcoreと同じ構成では列のまま、並びの変わる構成
    /// （`HIMAWARI_MB_ORDER=type`）では符号の順に並べて手の集合として比べる（ADR-0241の第16回）。
    #[cfg_attr(not(mb_shadow), allow(dead_code))]
    pub fn order_key(list: &[Move]) -> Vec<u32> {
        let mut v: Vec<u32> = list.iter().map(|m| m.bits()).collect();
        if !himawari_mailbox::ORDER_MATCHES_CORE {
            v.sort_unstable();
        }
        v
    }

    /// coreの局面から配列盤を作る。千日手の判定のために、過去の局面のキーも写す。
    pub fn from_core(pos: &Position) -> himawari_mailbox::Position {
        let mut mb = himawari_mailbox::Position::from_sfen(&pos.to_sfen())
            .expect("coreの出したSFENを配列盤が読めない");
        let hist: Vec<himawari_mailbox::HistoryKey> = pos
            .state_history()
            .iter()
            .map(|s| himawari_mailbox::HistoryKey {
                board_key: s.board_key,
                hand_key: s.hand_key,
                continuous_check: s.continuous_check,
                plies_from_null: s.plies_from_null,
            })
            .collect();
        mb.import_history(&hist);
        mb
    }

    #[inline(always)]
    pub fn idx(sq: Square) -> u8 {
        IDX81[sq.index()]
    }

    #[inline(always)]
    pub fn sq(i: u8) -> Square {
        Square::from_index(SQ81[i as usize])
    }

    #[inline(always)]
    pub fn color(c: himawari_mailbox::Color) -> Color {
        match c {
            himawari_mailbox::Color::Black => Color::Black,
            himawari_mailbox::Color::White => Color::White,
        }
    }

    #[inline(always)]
    pub fn mb_color(c: Color) -> himawari_mailbox::Color {
        match c {
            Color::Black => himawari_mailbox::Color::Black,
            Color::White => himawari_mailbox::Color::White,
        }
    }

    /// 81升の通し番号（81は升なし）をcoreの升へ。
    #[inline(always)]
    fn sq81(v: u8) -> Square {
        if v >= 81 {
            Square::NONE
        } else {
            Square::from_index(v)
        }
    }

    #[inline(always)]
    pub fn dirty(d: &himawari_mailbox::DirtyPiece) -> DirtyPiece {
        DirtyPiece {
            count: d.count,
            king_moved: d.king_moved,
            piece_old: [Piece(d.piece_old[0]), Piece(d.piece_old[1])],
            piece_new: [Piece(d.piece_new[0]), Piece(d.piece_new[1])],
            from: [sq81(d.from[0]), sq81(d.from[1])],
            to: [sq81(d.to[0]), sq81(d.to[1])],
        }
    }

    /// 81升の通し番号から配列盤の添字。7bitの欄をそのまま引けるように128升ぶん持つ（81以上は0）。
    const IDX128: [u8; 128] = {
        let mut t = [0u8; 128];
        let mut i = 0;
        while i < 81 {
            t[i] = IDX81[i];
            i += 1;
        }
        t
    };

    /// coreの手を配列盤の手へ。表を引いて欄を詰め替えるだけで、分岐しない（ADR-0241の第15回）。
    ///
    /// coreの符号は下位から行き先7bit・移動元7bit・成り・駒打ち・移動後の駒（bit16〜）、
    /// 配列盤の符号は行き先8bit・移動元8bit・成り・駒打ち・移動後の駒（bit24〜）である。
    /// 駒打ちの移動元の欄は打つ駒種で、両者で同じ値なので引かずに写す。
    #[inline(always)]
    pub fn to_mb(m: Move) -> himawari_mailbox::Move {
        let b = m.bits();
        let to = u32::from(IDX128[(b & 0x7F) as usize]);
        let f = (b >> 7) & 0x7F;
        let from = if b & 0x8000 != 0 {
            f
        } else {
            u32::from(IDX128[f as usize])
        };
        himawari_mailbox::Move::from_raw(
            to | (from << 8) | ((b & 0xC000) << 2) | (((b >> 16) & 0x1F) << 24),
        )
    }

    /// 配列盤の手をcoreの手へ。`to_mb` の逆。
    #[inline(always)]
    pub fn to_core(m: himawari_mailbox::Move) -> Move {
        let v = m.raw();
        let to = u32::from(SQ81[(v & 0xFF) as usize]);
        let f = (v >> 8) & 0xFF;
        let from = if v & (1 << 17) != 0 {
            f
        } else {
            u32::from(SQ81[f as usize])
        };
        Move::from_bits(to | (from << 7) | ((v >> 2) & 0xC000) | (((v >> 24) & 0x1F) << 16))
    }
}

#[cfg(all(test, mb_search))]
mod tests {
    use super::mbconv::{to_core, to_mb};
    use himawari_core::{MoveList, Position, generate_legal};

    /// 手の変換が、配列盤の手を組み立てる関数と同じ符号を作り、往復でcoreの手に戻る。
    /// 盤上の手・成り・駒打ちを含む局面で、合法手の全部を確かめる。
    #[test]
    fn move_conversion_round_trips() {
        for sfen in [
            himawari_core::SFEN_STARTPOS,
            "lr7/2g3k2/p2Ppp2B/4s1pPp/2Pnn4/PP1+b1P1p1/1S4P1N/6S2/L3KG2L w RGSNL3Pg2p 104",
            "l6nl/5+P1gk/2np1S3/p1p4Pp/3P2Sp1/1PPb2P1P/P5GS1/R8/LN4bKL w GR5pnsg 1",
        ] {
            let pos = Position::from_sfen(sfen).unwrap();
            let mb = himawari_mailbox::Position::from_sfen(sfen).unwrap();
            let mut list = MoveList::default();
            generate_legal(&pos, true, &mut list);
            let mut ml = himawari_mailbox::MoveList::default();
            mb.generate_legal(true, &mut ml);
            assert_eq!(list.len(), ml.len());
            // 並びの変わる構成でも組を作れるよう、配列盤の手をcoreの手の順に並べ直す
            let mut ml: Vec<himawari_mailbox::Move> = ml.as_slice().to_vec();
            if !himawari_mailbox::ORDER_MATCHES_CORE {
                let order: Vec<u32> = list.as_slice().iter().map(|m| m.bits()).collect();
                ml.sort_by_key(|w| order.iter().position(|&b| b == to_core(*w).bits()));
            }
            for (&m, &w) in list.as_slice().iter().zip(ml.iter()) {
                assert_eq!(to_mb(m), w, "to_mb {}", m.to_usi());
                assert_eq!(to_core(w), m, "to_core {}", m.to_usi());
            }
        }
    }
}
