//! NNUE accumulator差分計算（ADR-0035）。
//!
//! 第1塔（HalfKP FT）のみが対象。遅延方式: push/popではDirtyPieceを
//! 積むだけで、evaluateが呼ばれた時点で計算済みの祖先まで遡って
//! 差分を適用する。自玉が動いた視点は全計算（refresh）する。
//! 正しさの基準はnnue::evaluate_scalarとの完全一致。

use himawari_core::{Color, DirtyPiece, PieceType, Position, Square, bonapiece};

use crate::nnue::{CONCAT, FT_OUT, FtWeight, NnueNetwork, halfkp_active};
use crate::nnue_simd;
use crate::value::{MAX_PLY, Value};

/// 手駒の増減1件（キャプチャで増、駒打ちで減）。
#[derive(Clone, Copy)]
struct HandDelta {
    owner: Color,
    kind: PieceType,
    /// 1始まりのスロット番号（BonaPieceのi枚目）。
    slot: u32,
    added: bool,
}

/// accを64バイト境界に載せる（ADR-0151群H）。アライン未指定だと要素の
/// ストライドが64の倍数にならず、`Vec` の要素ごとにaccの先頭がずれる。
/// 片視点512バイトが8本でなく9本のキャッシュラインに跨る場合が出る。
/// `repr(C)` は宣言順を固定し、`FT_OUT` の値によらずaccをオフセット0に置く。
#[repr(C, align(64))]
struct AccEntry {
    /// 視点色（絶対色）ごとのFT accumulator。
    /// `computed[c]` がfalseの間は中身が未定義で、誰も読まない。
    /// pushで積み直すときにゼロ埋めしないのはこの不変条件による（ADR-0124）
    acc: [[i16; FT_OUT]; 2],
    computed: [bool; 2],
    /// この状態に至った手の差分。rootでは空。
    dirty: DirtyPiece,
    hand: Option<HandDelta>,
}

// accの先頭がキャッシュライン境界に載ることをコンパイル時に固定する。
const _: () = assert!(std::mem::align_of::<AccEntry>() == 64);
const _: () = assert!(std::mem::offset_of!(AccEntry, acc) == 0);

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

/// 積める段数。rootが0段目で、探索は `ply >= MAX_PLY` で即座に返るため、
/// 実際に積まれるのは最大 `MAX_PLY` 段になる。余白を足して越えを防ぐ。
const MAX_ENTRIES: usize = MAX_PLY + 16;

/// 探索スタックと同期するaccumulatorスタック。
pub struct NnueState {
    /// 起動時に `MAX_ENTRIES` ぶん確保し、以後は長さを変えない。
    /// 有効なのは `0..=top` で、それより上は前回の残骸が残る（ADR-0124）
    entries: Vec<AccEntry>,
    /// スタックトップの添字。
    top: usize,
    /// 全計算で使う特徴の置き場。ノードごとに確保し直さないため、
    /// 容量を保ったまま使い回す（ADR-0124）
    scratch: Vec<u32>,
}

impl NnueState {
    pub fn new() -> NnueState {
        NnueState {
            entries: std::iter::repeat_with(AccEntry::empty)
                .take(MAX_ENTRIES)
                .collect(),
            top: 0,
            // HalfKPで立つ特徴は玉以外の駒の数だけで、上限は38
            scratch: Vec::with_capacity(64),
        }
    }

    /// 探索の巻き戻しに備えてrootだけ残して初期化する。
    pub fn reset(&mut self) {
        self.top = 0;
        // accは書き戻さない。computedがfalseの間は読まれない
        let root = &mut self.entries[0];
        root.computed = [false, false];
        root.dirty = DirtyPiece::default();
        root.hand = None;
    }

    /// do_move直後のposで呼ぶ。差分材料だけ積む（計算はしない）。
    pub fn push(&mut self, pos: &Position) {
        let dirty = pos.state().dirty;
        let hand = hand_delta_of(pos, &dirty);
        self.top += 1;
        debug_assert!(
            self.top < MAX_ENTRIES,
            "accumulatorスタックが溢れた: top={}",
            self.top
        );
        // accは前段の残骸のまま残す。computedをfalseにするので、
        // ensureが全上書きするまで読まれない（ADR-0124）
        let e = &mut self.entries[self.top];
        e.computed = [false, false];
        e.dirty = dirty;
        e.hand = hand;
    }

    pub fn pop(&mut self) {
        debug_assert!(self.top > 0);
        self.top -= 1;
    }

    /// 現局面の評価値。必要な視点だけ遡り差分または全計算で作る。
    pub fn evaluate(&mut self, net: &NnueNetwork, pos: &Position) -> Value {
        for c in [Color::Black, Color::White] {
            self.ensure(net, pos, c);
        }
        let stm = pos.side_to_move();
        let top = &self.entries[self.top];
        let mut concat = [0u8; CONCAT];
        for (half, c) in [(0usize, stm), (1, stm.flip())] {
            debug_assert!(top.computed[c.index()], "未計算のaccを読もうとした");
            let acc = &top.acc[c.index()];
            nnue_simd::clip_to_u8(acc, &mut concat[half * FT_OUT..(half + 1) * FT_OUT]);
        }
        nnue_simd::forward_hidden(net, &concat)
    }

    /// 視点cのaccumulatorを最上段に用意する。
    fn ensure(&mut self, net: &NnueNetwork, pos: &Position, c: Color) {
        let top = self.top;
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
                    let prev = &before[j - 1];
                    debug_assert!(prev.computed[c.index()], "未計算のaccを読もうとした");
                    let e = &mut after[0];
                    // 借用が分かれているので、親のaccを読みながら直接書ける。
                    // 複製と足し引きを1パスに融合する（ADR-0151群A）
                    let rows = diff_rows(net, c, king, &e.dirty, e.hand);
                    apply_rows(&mut e.acc[c.index()], &prev.acc[c.index()], &rows);
                    e.computed[c.index()] = true;
                }
            }
            None => self.refresh_top(net, pos, c),
        }
    }

    /// 最上段を現局面から全計算する。
    fn refresh_top(&mut self, net: &NnueNetwork, pos: &Position, c: Color) {
        // scratchとentriesを同時に借りられないので、いったん取り出して戻す。
        // takeは空Vecとの交換なので確保は起きず、戻すときに容量が残る
        let mut features = std::mem::take(&mut self.scratch);
        features.clear();
        halfkp_active(pos, c, &mut features);
        {
            let top = &mut self.entries[self.top];
            // バイアスの複製と全特徴の加算を1パスにまとめる（ADR-0151群A）
            nnue_simd::ft_refresh(
                &mut top.acc[c.index()],
                &net.ft_b[..FT_OUT],
                &net.ft_w,
                &features,
            );
            top.computed[c.index()] = true;
        }
        self.scratch = features;
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

/// 1手ぶんの差分の重み行。玉は特徴に含まれないので、引く行も足す行も
/// 2本を超えない（取る手が最大で、移動元＋取られた駒と移動先＋手駒）。
struct DiffRows<'a> {
    subs: [&'a [FtWeight]; 2],
    ns: usize,
    adds: [&'a [FtWeight]; 2],
    na: usize,
}

/// entry1つぶんの差分から、引く行と足す行を集める。
fn diff_rows<'a>(
    net: &'a NnueNetwork,
    c: Color,
    king: Square,
    dirty: &DirtyPiece,
    hand: Option<HandDelta>,
) -> DiffRows<'a> {
    let mut r = DiffRows {
        subs: [&[], &[]],
        ns: 0,
        adds: [&[], &[]],
        na: 0,
    };
    let push = |r: &mut DiffRows<'a>, bp: u16, add: bool| {
        let idx = bonapiece::halfkp_index(c, king, bp) as usize * FT_OUT;
        let w = &net.ft_w[idx..idx + FT_OUT];
        // 上限を超えたら添字で落ちる。黙って差分を捨てない
        if add {
            r.adds[r.na] = w;
            r.na += 1;
        } else {
            r.subs[r.ns] = w;
            r.ns += 1;
        }
    };
    for j in 0..dirty.count as usize {
        let old = dirty.piece_old[j];
        let from = dirty.from[j];
        if from != Square::NONE && !old.is_empty() && old.piece_type() != PieceType::KING {
            push(&mut r, bonapiece::board_bona_piece(c, old, from), false);
        }
        let new = dirty.piece_new[j];
        let to = dirty.to[j];
        if to != Square::NONE && !new.is_empty() && new.piece_type() != PieceType::KING {
            push(&mut r, bonapiece::board_bona_piece(c, new, to), true);
        }
    }
    if let Some(hd) = hand {
        let bp = bonapiece::hand_bona_piece(c, hd.owner, hd.kind, hd.slot);
        push(&mut r, bp, hd.added);
    }
    r
}

/// 親のaccへ差分を適用して自分のaccへ書く（ADR-0151群A）。
/// 行数ごとに融合カーネルを単相化する。実際に現れるのは
/// (0,0)（玉の移動・null move）・(1,1)（普通の手と駒打ち）・
/// (2,2)（取る手）の3通りで、残りは念のため用意する。
fn apply_rows(dst: &mut [i16; FT_OUT], src: &[i16; FT_OUT], r: &DiffRows<'_>) {
    let (s, a) = (r.subs, r.adds);
    match (r.ns, r.na) {
        (0, 0) => nnue_simd::ft_apply(dst, src, [], []),
        (1, 1) => nnue_simd::ft_apply(dst, src, [s[0]], [a[0]]),
        (2, 2) => nnue_simd::ft_apply(dst, src, [s[0], s[1]], [a[0], a[1]]),
        (0, 1) => nnue_simd::ft_apply(dst, src, [], [a[0]]),
        (0, 2) => nnue_simd::ft_apply(dst, src, [], [a[0], a[1]]),
        (1, 0) => nnue_simd::ft_apply(dst, src, [s[0]], []),
        (1, 2) => nnue_simd::ft_apply(dst, src, [s[0]], [a[0], a[1]]),
        (2, 0) => nnue_simd::ft_apply(dst, src, [s[0], s[1]], []),
        (2, 1) => nnue_simd::ft_apply(dst, src, [s[0], s[1]], [a[0]]),
        _ => unreachable!("差分の行数が上限を超えた: ns={}, na={}", r.ns, r.na),
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
