//! 評価関数インターフェース（ADR-0023）。
//!
//! enumディスパッチ。P2は駒割＋tempoのMaterialのみ。
//! push/popはdo_move/undo_moveと対で呼ぶ契約（NNUEの布石）。

use himawari_core::{Color, Position};

use crate::value::Value;

const TEMPO: Value = 20;

pub enum Evaluator {
    Material(MaterialEval),
}

#[derive(Default)]
pub struct MaterialEval {
    depth: i32,
}

impl Evaluator {
    pub fn material() -> Evaluator {
        Evaluator::Material(MaterialEval::default())
    }

    pub fn new_search(&mut self, _pos: &Position) {
        match self {
            Evaluator::Material(m) => m.depth = 0,
        }
    }

    pub fn push(&mut self, _pos: &Position) {
        match self {
            Evaluator::Material(m) => m.depth += 1,
        }
    }

    pub fn pop(&mut self) {
        match self {
            Evaluator::Material(m) => {
                m.depth -= 1;
                debug_assert!(m.depth >= 0, "push/popの対応が壊れている");
            }
        }
    }

    /// 手番視点の評価値。
    pub fn evaluate(&mut self, pos: &Position) -> Value {
        match self {
            Evaluator::Material(_) => {
                let m = pos.state().material;
                let v = if pos.side_to_move() == Color::Black {
                    m
                } else {
                    -m
                };
                v + TEMPO
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use himawari_core::SFEN_STARTPOS;

    #[test]
    fn material_is_symmetric_at_start() {
        let pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
        let mut ev = Evaluator::material();
        ev.new_search(&pos);
        assert_eq!(ev.evaluate(&pos), TEMPO);
    }

    #[test]
    fn capture_changes_material() {
        // 先手が歩得している局面
        let pos = Position::from_sfen("4k4/9/9/9/9/9/9/9/4K4 b P 1").unwrap();
        let mut ev = Evaluator::material();
        ev.new_search(&pos);
        assert!(ev.evaluate(&pos) > TEMPO);
    }
}
