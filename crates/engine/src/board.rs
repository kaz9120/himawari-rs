//! 探索が手を進め戻す盤面（ADR-0239の手順0）。
//!
//! 探索のワーカーと、盤面操作だけを測る `boardbench` が同じ進め戻しを
//! 呼ぶために1か所へ集める。利きの表のように局面に付いて回る状態を
//! 足すときは、この型に持たせ、`do_move` などへ追従を書く。そうすれば
//! 探索とミクロな計測の両方が同じ費用を払う。
//!
//! 読み取りは `Deref` で `Position` のメソッドをそのまま呼べる。進め戻しと、
//! 下の「盤への問い合わせ」はこの型の同名のメソッドが先に解決されるので、
//! 包んだ側を素通りしない。`DerefMut` は実装しない。
//! 局面を可変で借りる経路を、この型の進め戻しに限るためである。
//!
//! フィーチャ `mailbox`（ADR-0241の段階6、研究ブランチだけ）では、配列盤
//! （`himawari_mailbox::Position`）を並べて持ち、同じ手で進め戻す。指し手生成・
//! 合法性・擬似合法性・王手判定・SEE・1手詰め・置換表の手の復元・利き数の
//! 段階の集合を配列盤に答えさせる。coreの局面は残りの読み取り（キー・千日手・
//! 駒の配置・評価関数の駒の特徴など）のために持ち続ける。デバッグビルドでは、
//! 配列盤の答えを毎回coreの答えと比べる。

use std::ops::Deref;

use himawari_core::{GenType, Move, Move16, MoveList, Position};

use crate::nnue::EffectLevels;

/// 探索が進め戻す盤面。`Position` と、それに追従する状態を持つ。
#[derive(Clone)]
pub struct SearchBoard {
    pos: Position,
    #[cfg(feature = "mailbox")]
    mb: himawari_mailbox::Position,
}

impl SearchBoard {
    pub fn new(pos: Position) -> SearchBoard {
        #[cfg(feature = "mailbox")]
        let mb = himawari_mailbox::Position::from_sfen(&pos.to_sfen())
            .expect("coreの出したSFENを配列盤が読めない");
        SearchBoard {
            pos,
            #[cfg(feature = "mailbox")]
            mb,
        }
    }

    /// 包んでいる局面。
    #[inline(always)]
    pub fn position(&self) -> &Position {
        &self.pos
    }

    /// 1手進める。
    #[inline(always)]
    pub fn do_move(&mut self, m: Move) {
        self.pos.do_move(m);
        #[cfg(feature = "mailbox")]
        self.mb.do_move(mbconv::to_mb(m));
    }

    /// `do_move` を戻す。
    #[inline(always)]
    pub fn undo_move(&mut self, m: Move) {
        self.pos.undo_move(m);
        #[cfg(feature = "mailbox")]
        self.mb.undo_move(mbconv::to_mb(m));
    }

    /// null moveで1手進める。
    #[inline(always)]
    pub fn do_null_move(&mut self) {
        self.pos.do_null_move();
        #[cfg(feature = "mailbox")]
        self.mb.do_null_move();
    }

    /// `do_null_move` を戻す。
    #[inline(always)]
    pub fn undo_null_move(&mut self) {
        self.pos.undo_null_move();
        #[cfg(feature = "mailbox")]
        self.mb.undo_null_move();
    }
}

/// 盤への問い合わせ。既定ではcoreの同名の関数へそのまま渡す。
#[cfg(not(feature = "mailbox"))]
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
}

/// 盤への問い合わせを配列盤が答える版（ADR-0241の段階6）。
///
/// 返す値はどれもcoreと同じになるように作ってある（等価な版）。
/// デバッグビルドでは毎回coreの答えと比べ、食い違えば局面を添えて止める。
#[cfg(feature = "mailbox")]
impl SearchBoard {
    /// 擬似合法手を生成する。列はcoreの `generate` を合法手で濾した列と同じになる。
    ///
    /// 配列盤の生成は合法手だけを出す（ADR-0241の第2回）。探索はこのあと `is_legal` で
    /// 濾すので、濾した後の列が同じなら探索の木は変わらない。
    #[inline(always)]
    pub fn generate(&self, gt: GenType, all: bool, list: &mut MoveList) {
        let g = match gt {
            GenType::Captures => himawari_mailbox::GenType::Captures,
            GenType::Quiets => himawari_mailbox::GenType::Quiets,
            GenType::Evasions => himawari_mailbox::GenType::Evasions,
            GenType::NonEvasions => himawari_mailbox::GenType::NonEvasions,
        };
        let mut ml = himawari_mailbox::MoveList::default();
        self.mb.generate(g, all, &mut ml);
        for &m in ml.as_slice() {
            list.push(mbconv::to_core(m));
        }
        if cfg!(debug_assertions) {
            let mut want = MoveList::default();
            himawari_core::generate(&self.pos, gt, all, &mut want);
            let want: Vec<Move> = want
                .as_slice()
                .iter()
                .copied()
                .filter(|&m| self.pos.is_legal(m))
                .collect();
            assert_eq!(
                list.as_slice(),
                want.as_slice(),
                "generate {gt:?} all={all}: {}",
                self.pos.to_sfen()
            );
        }
    }

    /// 合法手を生成する。
    #[inline(always)]
    pub fn generate_legal(&self, all: bool, list: &mut MoveList) {
        let mut ml = himawari_mailbox::MoveList::default();
        self.mb.generate_legal(all, &mut ml);
        for &m in ml.as_slice() {
            list.push(mbconv::to_core(m));
        }
        if cfg!(debug_assertions) {
            let mut want = MoveList::default();
            himawari_core::generate_legal(&self.pos, all, &mut want);
            assert_eq!(
                list.as_slice(),
                want.as_slice(),
                "generate_legal: {}",
                self.pos.to_sfen()
            );
        }
    }

    /// 擬似合法手mが合法か。
    #[inline(always)]
    pub fn is_legal(&self, m: Move) -> bool {
        let r = self.mb.is_legal(mbconv::to_mb(m));
        debug_assert_eq!(
            r,
            self.pos.is_legal(m),
            "is_legal {}: {}",
            m.to_usi(),
            self.pos.to_sfen()
        );
        r
    }

    /// mがこの局面の擬似合法手か（置換表・キラーの手の検査）。
    #[inline(always)]
    pub fn pseudo_legal(&self, m: Move) -> bool {
        // 特殊値と駒の無い手は配列盤の手に直せない。coreも前者には偽を返す
        if m.is_special() || m.piece_after().is_empty() {
            return self.pos.pseudo_legal(m);
        }
        let r = self.mb.pseudo_legal(mbconv::to_mb(m));
        debug_assert_eq!(
            r,
            self.pos.pseudo_legal(m),
            "pseudo_legal {}: {}",
            m.to_usi(),
            self.pos.to_sfen()
        );
        r
    }

    /// 合法手mが王手になるか。
    #[inline(always)]
    pub fn gives_check(&self, m: Move) -> bool {
        let r = self.mb.gives_check(mbconv::to_mb(m));
        debug_assert_eq!(
            r,
            self.pos.gives_check(m),
            "gives_check {}: {}",
            m.to_usi(),
            self.pos.to_sfen()
        );
        r
    }

    /// 静的交換評価が閾値以上か。
    #[inline(always)]
    pub fn see_ge(&self, m: Move, threshold: i32) -> bool {
        let r = self.mb.see_ge(mbconv::to_mb(m), threshold);
        debug_assert_eq!(
            r,
            self.pos.see_ge(m, threshold),
            "see_ge {} {threshold}: {}",
            m.to_usi(),
            self.pos.to_sfen()
        );
        r
    }

    /// 1手詰めの手（王手されていない局面で呼ぶ）。
    #[inline(always)]
    pub fn mate_1ply(&self) -> Option<Move> {
        let r = self.mb.mate_1ply().map(mbconv::to_core);
        debug_assert_eq!(
            r,
            crate::mate::mate_1ply(&self.pos),
            "mate_1ply: {}",
            self.pos.to_sfen()
        );
        r
    }

    /// 置換表の `Move16` を完全な手へ戻す。
    #[inline(always)]
    pub fn to_move(&self, m16: Move16) -> Option<Move> {
        let r = self.mb.move_from16(m16.0).map(mbconv::to_core);
        debug_assert_eq!(
            r,
            self.pos.to_move(m16),
            "to_move {:#06x}: {}",
            m16.0,
            self.pos.to_sfen()
        );
        r
    }

    /// 評価関数の利き数の入力が使う、段階の集合（ADR-0237のD2）。
    ///
    /// 配列盤は利き数を升ごとに持つので、駒ごとに利き先を数え直さず、81升を読むだけで作れる。
    #[inline(always)]
    pub fn effect_levels(&self) -> EffectLevels {
        let r = EffectLevels::from_bits(self.mb.effect_levels());
        debug_assert_eq!(
            r,
            EffectLevels::build(&self.pos),
            "effect_levels: {}",
            self.pos.to_sfen()
        );
        r
    }
}

impl Deref for SearchBoard {
    type Target = Position;

    #[inline(always)]
    fn deref(&self) -> &Position {
        &self.pos
    }
}

/// coreの手と配列盤の手の変換。升を81升の通し番号と配列盤の添字の間で直す。
/// 駒の符号は両者で同じ（OSL配列）。
#[cfg(feature = "mailbox")]
mod mbconv {
    use himawari_core::{Color, Move, Piece, PieceType, Square};
    use himawari_mailbox::board::{IDX81, SQ81};

    #[inline(always)]
    pub fn to_mb(m: Move) -> himawari_mailbox::Move {
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

    #[inline(always)]
    pub fn to_core(m: himawari_mailbox::Move) -> Move {
        let to = Square::from_index(SQ81[m.to() as usize]);
        if m.is_drop() {
            let c = if m.piece_after() & 16 != 0 {
                Color::White
            } else {
                Color::Black
            };
            Move::new_drop(PieceType(m.drop_type()), to, c)
        } else {
            Move::new_move(
                Square::from_index(SQ81[m.from() as usize]),
                to,
                m.is_promote(),
                Piece(m.piece_after()),
            )
        }
    }
}
