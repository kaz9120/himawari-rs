//! 探索が手を進め戻す盤面（ADR-0239の手順0）。
//!
//! 探索のワーカーと、盤面操作だけを測る `boardbench` が同じ進め戻しを
//! 呼ぶために1か所へ集める。利きの表のように局面に付いて回る状態を
//! 足すときは、この型に持たせ、`do_move` などへ追従を書く。そうすれば
//! 探索とミクロな計測の両方が同じ費用を払う。
//!
//! いまは `Position` を包むだけである。読み取りは `Deref` で `Position` の
//! メソッドをそのまま呼べる。進め戻しはこの型の同名のメソッドが先に
//! 解決されるので、包んだ側を素通りしない。`DerefMut` は実装しない。
//! 局面を可変で借りる経路を、この型の進め戻しに限るためである。

use std::ops::Deref;

use himawari_core::{Move, Position};

/// 探索が進め戻す盤面。`Position` と、それに追従する状態を持つ。
#[derive(Clone)]
pub struct SearchBoard {
    pos: Position,
}

impl SearchBoard {
    pub fn new(pos: Position) -> SearchBoard {
        SearchBoard { pos }
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
    }

    /// `do_move` を戻す。
    #[inline(always)]
    pub fn undo_move(&mut self, m: Move) {
        self.pos.undo_move(m);
    }

    /// null moveで1手進める。
    #[inline(always)]
    pub fn do_null_move(&mut self) {
        self.pos.do_null_move();
    }

    /// `do_null_move` を戻す。
    #[inline(always)]
    pub fn undo_null_move(&mut self) {
        self.pos.undo_null_move();
    }
}

impl Deref for SearchBoard {
    type Target = Position;

    #[inline(always)]
    fn deref(&self) -> &Position {
        &self.pos
    }
}
