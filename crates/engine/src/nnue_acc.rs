//! NNUE accumulator差分計算（ADR-0035）。
//!
//! 第1塔（HalfKP FT）のみが対象。遅延方式: push/popではDirtyPieceを
//! 積むだけで、evaluateが呼ばれた時点で計算済みの祖先まで遡って
//! 差分を適用する。自玉が動いた視点は全計算（refresh）する。
//! 正しさの基準はnnue::evaluate_scalarとの完全一致。

use himawari_core::{Color, DirtyPiece, PieceType, Position, Square, bonapiece};

use crate::nnue::{CONCAT, FT_OUT, NnueNetwork, halfkp_active};
use crate::nnue_simd;
use crate::value::Value;

/// 手駒の増減1件（キャプチャで増、駒打ちで減）。
#[derive(Clone, Copy)]
struct HandDelta {
    owner: Color,
    kind: PieceType,
    /// 1始まりのスロット番号（BonaPieceのi枚目）。
    slot: u32,
    added: bool,
}

struct AccEntry {
    /// 視点色（絶対色）ごとのFT accumulator。
    acc: [[i16; FT_OUT]; 2],
    computed: [bool; 2],
    /// この状態に至った手の差分。rootでは空。
    dirty: DirtyPiece,
    hand: Option<HandDelta>,
}

impl AccEntry {
    fn empty() -> AccEntry {
        AccEntry {
            acc: [[0; FT_OUT]; 2],
            computed: [false, false],
            dirty: DirtyPiece::default(),
            hand: None,
        }
    }
}

/// 探索スタックと同期するaccumulatorスタック。
pub struct NnueState {
    entries: Vec<AccEntry>,
}

impl NnueState {
    pub fn new() -> NnueState {
        NnueState {
            entries: vec![AccEntry::empty()],
        }
    }

    /// 探索の巻き戻しに備えてrootだけ残して初期化する。
    pub fn reset(&mut self) {
        self.entries.truncate(1);
        self.entries[0] = AccEntry::empty();
    }

    /// do_move直後のposで呼ぶ。差分材料だけ積む（計算はしない）。
    pub fn push(&mut self, pos: &Position) {
        let dirty = pos.state().dirty;
        let hand = hand_delta_of(pos, &dirty);
        self.entries.push(AccEntry {
            acc: [[0; FT_OUT]; 2],
            computed: [false, false],
            dirty,
            hand,
        });
    }

    pub fn pop(&mut self) {
        debug_assert!(self.entries.len() > 1);
        self.entries.pop();
    }

    /// 現局面の評価値。必要な視点だけ遡り差分または全計算で作る。
    pub fn evaluate(&mut self, net: &NnueNetwork, pos: &Position) -> Value {
        for c in [Color::Black, Color::White] {
            self.ensure(net, pos, c);
        }
        let stm = pos.side_to_move();
        let top = self.entries.last().expect("stack not empty");
        let mut concat = [0u8; CONCAT];
        for (half, c) in [(0usize, stm), (1, stm.flip())] {
            let acc = &top.acc[c.index()];
            nnue_simd::clip_to_u8(acc, &mut concat[half * FT_OUT..(half + 1) * FT_OUT]);
        }
        nnue_simd::forward_hidden(net, &concat)
    }

    /// 視点cのaccumulatorを最上段に用意する。
    fn ensure(&mut self, net: &NnueNetwork, pos: &Position, c: Color) {
        let top = self.entries.len() - 1;
        if self.entries[top].computed[c.index()] {
            return;
        }
        // 計算済みの祖先を探す。視点cの玉移動を跨いだら遡れない
        let mut src = None;
        let mut i = top;
        loop {
            if self.entries[i].computed[c.index()] {
                src = Some(i);
                break;
            }
            let d = &self.entries[i].dirty;
            if d.king_moved && d.piece_new[0].color() == c {
                break;
            }
            if i == 0 {
                break;
            }
            i -= 1;
        }
        match src {
            Some(s) => {
                let king = pos.king(c);
                for j in s + 1..=top {
                    let (before, after) = self.entries.split_at_mut(j);
                    let prev_acc = before[j - 1].acc[c.index()];
                    let e = &mut after[0];
                    e.acc[c.index()] = prev_acc;
                    apply_dirty(net, c, king, e);
                    e.computed[c.index()] = true;
                }
            }
            None => self.refresh_top(net, pos, c),
        }
    }

    /// 最上段を現局面から全計算する。
    fn refresh_top(&mut self, net: &NnueNetwork, pos: &Position, c: Color) {
        let mut features = Vec::with_capacity(64);
        halfkp_active(pos, c, &mut features);
        let top = self.entries.last_mut().expect("stack not empty");
        let acc = &mut top.acc[c.index()];
        for (o, a) in acc.iter_mut().enumerate() {
            *a = net.ft_b[o];
        }
        for &f in &features {
            let base = f as usize * FT_OUT;
            nnue_simd::ft_add(acc, &net.ft_w[base..base + FT_OUT]);
        }
        top.computed[c.index()] = true;
    }
}

impl Default for NnueState {
    fn default() -> Self {
        NnueState::new()
    }
}

/// dirtyから手駒の増減を復元する。posはdo_move直後の局面。
fn hand_delta_of(pos: &Position, dirty: &DirtyPiece) -> Option<HandDelta> {
    if dirty.count == 2 {
        // キャプチャ: piece_new[1]が手駒に入った駒（枚数は取った後の値）
        let pc = dirty.piece_new[1];
        let owner = pc.color();
        let kind = pc.piece_type();
        Some(HandDelta {
            owner,
            kind,
            slot: pos.hand(owner).count(kind),
            added: true,
        })
    } else if dirty.count == 1 && dirty.from[0] == Square::NONE {
        // 駒打ち: 打った後の枚数+1のスロットが消えた
        let pc = dirty.piece_new[0];
        let owner = pc.color();
        let kind = pc.piece_type().unpromote();
        Some(HandDelta {
            owner,
            kind,
            slot: pos.hand(owner).count(kind) + 1,
            added: false,
        })
    } else {
        None
    }
}

/// entry1つぶんの差分をacc[c]へ適用する。玉は特徴に含まれない。
fn apply_dirty(net: &NnueNetwork, c: Color, king: Square, e: &mut AccEntry) {
    let acc = &mut e.acc[c.index()];
    let mut sub_add = |bp: u16, add: bool| {
        let idx = bonapiece::halfkp_index(c, king, bp) as usize * FT_OUT;
        let w = &net.ft_w[idx..idx + FT_OUT];
        if add {
            nnue_simd::ft_add(acc, w);
        } else {
            nnue_simd::ft_sub(acc, w);
        }
    };
    for j in 0..e.dirty.count as usize {
        let old = e.dirty.piece_old[j];
        let from = e.dirty.from[j];
        if from != Square::NONE && !old.is_empty() && old.piece_type() != PieceType::KING {
            sub_add(bonapiece::board_bona_piece(c, old, from), false);
        }
        let new = e.dirty.piece_new[j];
        let to = e.dirty.to[j];
        if to != Square::NONE && !new.is_empty() && new.piece_type() != PieceType::KING {
            sub_add(bonapiece::board_bona_piece(c, new, to), true);
        }
    }
    if let Some(hd) = e.hand {
        let bp = bonapiece::hand_bona_piece(c, hd.owner, hd.kind, hd.slot);
        sub_add(bp, hd.added);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nnue::evaluate_scalar;
    use himawari_core::{MoveList, SFEN_STARTPOS, generate_legal};

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

    /// 差分計算 = 全計算の完全一致（P4出口条件）。
    /// 実際の探索と同型の再帰do/undoウォーク（null move含む）で照合する。
    fn walk(
        net: &NnueNetwork,
        pos: &mut Position,
        st: &mut NnueState,
        rng: &mut Rng,
        depth: usize,
    ) {
        assert_eq!(
            st.evaluate(net, pos),
            evaluate_scalar(net, pos),
            "差分と全計算が一致しない: {}",
            pos.to_sfen()
        );
        if depth == 0 {
            return;
        }
        for _ in 0..2 {
            let mut list = MoveList::default();
            generate_legal(pos, true, &mut list);
            if list.is_empty() {
                return;
            }
            if !pos.in_check() && rng.next().is_multiple_of(8) {
                pos.do_null_move();
                st.push(pos);
                walk(net, pos, st, rng, depth - 1);
                st.pop();
                pos.undo_null_move();
            } else {
                let m = list.as_slice()[(rng.next() % list.len() as u64) as usize];
                pos.do_move(m);
                st.push(pos);
                walk(net, pos, st, rng, depth - 1);
                st.pop();
                pos.undo_move(m);
            }
            // undo直後の再評価も一致すること
            assert_eq!(st.evaluate(net, pos), evaluate_scalar(net, pos));
        }
    }

    #[test]
    fn incremental_matches_full_computation() {
        let net = NnueNetwork::random(7);
        for seed in 1..=5u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
            // ランダムに進めた中盤局面をrootにする
            for _ in 0..(rng.next() % 30) {
                let mut list = MoveList::default();
                generate_legal(&pos, true, &mut list);
                if list.is_empty() {
                    break;
                }
                let m = list.as_slice()[(rng.next() % list.len() as u64) as usize];
                pos.do_move(m);
            }
            let mut st = NnueState::new();
            walk(&net, &mut pos, &mut st, &mut rng, 5);
        }
    }
}
