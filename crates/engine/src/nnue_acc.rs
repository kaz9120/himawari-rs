//! NNUE accumulator差分計算（ADR-0035）。
//!
//! 第1塔（HalfKP FT）のみが対象。遅延方式: push/popではDirtyPieceを
//! 積むだけで、evaluateが呼ばれた時点で計算済みの祖先まで遡って
//! 差分を適用する。自玉が動いた視点は全計算（refresh）する。
//! 正しさの基準はnnue::evaluate_scalarとの完全一致。

use himawari_core::bonapiece::{FE_END, KING_BUCKETS, View};
use himawari_core::{Color, DirtyPiece, PieceType, Position, Square};

use crate::nnue::{self, CONCAT, FT_OUT, FtWeight, NnueNetwork};
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

/// BonaPieceの集合を持つビットセットの語数。
const BP_WORDS: usize = (FE_END as usize).div_ceil(64);

/// 玉位置ごとのキャッシュの件数（視点色 × 玉バケット）。
/// 左右対称な玉位置は同じバケットを共有する（ADR-0157）。
const FINNY_ENTRIES: usize = 2 * KING_BUCKETS;

/// 玉位置ごとのaccumulatorキャッシュ1件（ADR-0156）。
///
/// `acc` は `bits` が表すBonaPiece集合をバイアスへ足し込んだ状態にある。
/// 玉が動いた視点は差分連鎖を遡れないが、**同じ玉位置の局面をここに
/// 1件だけ残しておけば、全計算の代わりに集合の差分で済む。**
#[repr(C, align(64))]
struct FinnyEntry {
    acc: [i16; FT_OUT],
    /// `acc` に反映済みのBonaPiece集合。
    bits: [u64; BP_WORDS],
    /// falseの間は `acc`・`bits` の中身が未定義で、誰も読まない。
    valid: bool,
}

impl FinnyEntry {
    fn empty() -> FinnyEntry {
        FinnyEntry {
            acc: [0; FT_OUT],
            bits: [0; BP_WORDS],
            valid: false,
        }
    }
}

/// 探索スタックと同期するaccumulatorスタック。
pub struct NnueState {
    /// 起動時に `MAX_ENTRIES` ぶん確保し、以後は長さを変えない。
    /// 有効なのは `0..=top` で、それより上は前回の残骸が残る（ADR-0124）
    entries: Vec<AccEntry>,
    /// スタックトップの添字。
    top: usize,
    /// 玉位置ごとのaccumulatorキャッシュ（ADR-0156）。
    /// 添字は `視点色 * 81 + 玉の升`。
    finny: Vec<FinnyEntry>,
    /// キャッシュとの差分を組み立てる置き場。`scratch` と同じく
    /// 容量を保ったまま使い回す（ADR-0124）
    scratch: Scratch,
}

/// キャッシュ差分で使う作業領域。まとめて `mem::take` できる形に
/// しておくと、`entries`・`finny` と借用が衝突しない。
#[derive(Default)]
struct Scratch {
    /// キャッシュへ足す特徴インデックス。
    adds: Vec<u32>,
    /// キャッシュから引く特徴インデックス。
    subs: Vec<u32>,
}

impl NnueState {
    pub fn new() -> NnueState {
        NnueState {
            entries: std::iter::repeat_with(AccEntry::empty)
                .take(MAX_ENTRIES)
                .collect(),
            top: 0,
            finny: std::iter::repeat_with(FinnyEntry::empty)
                .take(FINNY_ENTRIES)
                .collect(),
            // HalfKPで立つ特徴は玉以外の駒の数だけで、上限は38
            scratch: Scratch {
                adds: Vec::with_capacity(64),
                subs: Vec::with_capacity(64),
            },
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
        self.ensure_both(net, pos);
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

    /// 両視点のaccumulatorを最上段に用意する（ADR-0151群N）。
    ///
    /// 視点ごとに遡れる範囲は違う。**両色をまとめて適用できるのは、
    /// 遅いほうの起点から上の区間だけである。** 手前の区間は片色ずつ
    /// 埋め、そこから上を1パスで両色へ書く。玉が動いた視点は遡れないので
    /// 全計算になり、その視点は融合の対象から外れる。
    fn ensure_both(&mut self, net: &NnueNetwork, pos: &Position) {
        let top = self.top;
        let src = [
            self.computed_ancestor(Color::Black),
            self.computed_ancestor(Color::White),
        ];
        match src {
            [Some(b), Some(w)] => {
                let join = b.max(w);
                // 起点がずれている片色だけ、合流点まで先に埋める
                if b < join {
                    self.apply_range(net, pos, Color::Black, b, join);
                }
                if w < join {
                    self.apply_range(net, pos, Color::White, w, join);
                }
                self.apply_range_both(net, pos, join, top);
            }
            [Some(b), None] => {
                self.apply_range(net, pos, Color::Black, b, top);
                self.refresh_top(net, pos, Color::White);
            }
            [None, Some(w)] => {
                self.refresh_top(net, pos, Color::Black);
                self.apply_range(net, pos, Color::White, w, top);
            }
            [None, None] => {
                self.refresh_top(net, pos, Color::Black);
                self.refresh_top(net, pos, Color::White);
            }
        }
    }

    /// 視点cのaccが計算済みで最上段にいちばん近い段。玉移動を跨いだら
    /// 遡れないのでNone（全計算になる）。
    fn computed_ancestor(&self, c: Color) -> Option<usize> {
        let mut i = self.top;
        loop {
            if self.entries[i].computed[c.index()] {
                return Some(i);
            }
            let d = &self.entries[i].dirty;
            if d.king_moved && d.piece_new[0].color() == c {
                return None;
            }
            if i == 0 {
                return None;
            }
            i -= 1;
        }
    }

    /// 段 `from` から `to` まで、視点cの差分を1段ずつ適用する。
    /// `from` は計算済みで、`from+1..=to` を埋める。
    fn apply_range(&mut self, net: &NnueNetwork, pos: &Position, c: Color, from: usize, to: usize) {
        // この区間に視点cの玉移動はないので、視点は現局面のもので通る。
        // 左右の正規化も玉位置から決まるので、同じ理由で区間内は一定である
        let view = View::new(c, pos.king(c));
        for j in from + 1..=to {
            let (before, after) = self.entries.split_at_mut(j);
            let prev = &before[j - 1];
            debug_assert!(prev.computed[c.index()], "未計算のaccを読もうとした");
            let e = &mut after[0];
            // 借用が分かれているので、親のaccを読みながら直接書ける。
            // 複製と足し引きを1パスに融合する（ADR-0151群A）
            let rows = diff_rows(net, [view], &e.dirty, e.hand);
            apply_rows([&mut e.acc[c.index()]], [&prev.acc[c.index()]], &rows);
            e.computed[c.index()] = true;
        }
    }

    /// 同上を両視点まとめて行う（ADR-0151群N）。連鎖の走査とdirtyの
    /// デコードが1回で済み、accへの読み書きが2本のストリームで並ぶ。
    fn apply_range_both(&mut self, net: &NnueNetwork, pos: &Position, from: usize, to: usize) {
        let views = [
            View::new(Color::Black, pos.king(Color::Black)),
            View::new(Color::White, pos.king(Color::White)),
        ];
        for j in from + 1..=to {
            let (before, after) = self.entries.split_at_mut(j);
            let prev = &before[j - 1];
            debug_assert!(prev.computed == [true, true], "未計算のaccを読もうとした");
            let e = &mut after[0];
            let rows = diff_rows(net, views, &e.dirty, e.hand);
            let [d0, d1] = &mut e.acc;
            let [s0, s1] = &prev.acc;
            apply_rows([d0, d1], [s0, s1], &rows);
            e.computed = [true, true];
        }
    }

    /// 最上段の視点cを、同じ玉位置のキャッシュとの差分で作る（ADR-0156）。
    ///
    /// 玉が動いた視点は段の連鎖を遡れない。**遡る代わりに、同じ玉位置で
    /// 最後に作ったaccumulatorを起点にする。** HalfKPの特徴は玉位置を
    /// 固定すればBonaPieceの集合で決まるので、集合の差だけ足し引きすれば
    /// 全計算と同じ値になる。キャッシュは引くと同時に現局面で更新する
    /// ので、書き戻す手順は要らない。
    ///
    /// キャッシュが空のときは、バイアスを置いて全特徴を足す形になり、
    /// 従来の全計算と同じ経路をたどる。
    fn refresh_top(&mut self, net: &NnueNetwork, pos: &Position, c: Color) {
        let view = View::new(c, pos.king(c));
        // 玉バケットを固定した特徴インデックスの起点。BonaPieceを足せば
        // そのまま特徴インデックスになる
        let base = view.base();
        let mut cur = [0u64; BP_WORDS];
        nnue::for_each_bona_piece(pos, view, |bp| {
            cur[bp as usize / 64] |= 1u64 << (bp % 64);
        });

        // scratch・finny・entriesを同時に借りられないので、いったん取り出して
        // 戻す。takeは空Vecとの交換なので確保は起きず、容量も残る
        let mut s = std::mem::take(&mut self.scratch);
        let mut finny = std::mem::take(&mut self.finny);
        let e = &mut finny[c.index() * KING_BUCKETS + (base / u32::from(FE_END)) as usize];
        if !e.valid {
            e.acc.copy_from_slice(&net.ft_b[..FT_OUT]);
            e.bits = [0; BP_WORDS];
            e.valid = true;
        }
        for (w, (&now, &had)) in cur.iter().zip(e.bits.iter()).enumerate() {
            let off = base + (w * 64) as u32;
            let mut added = now & !had;
            while added != 0 {
                s.adds.push(off + added.trailing_zeros());
                added &= added - 1;
            }
            let mut removed = had & !now;
            while removed != 0 {
                s.subs.push(off + removed.trailing_zeros());
                removed &= removed - 1;
            }
        }
        nnue_simd::ft_update(&mut e.acc, &net.ft_w, &s.adds, &s.subs);
        e.bits = cur;

        let top = &mut self.entries[self.top];
        top.acc[c.index()] = e.acc;
        top.computed[c.index()] = true;

        s.adds.clear();
        s.subs.clear();
        self.scratch = s;
        self.finny = finny;
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
///
/// 行を集める条件はdirtyの中身だけで決まり、視点には依らない。だから
/// 本数 `ns`・`na` は視点で共通になり、同じ位置の行を視点 `V` 本ぶん
/// 並べて持てる（ADR-0151群N）。添字は `[スロット][視点]` の順。
struct DiffRows<'a, const V: usize> {
    subs: [[&'a [FtWeight]; V]; 2],
    ns: usize,
    adds: [[&'a [FtWeight]; V]; 2],
    na: usize,
}

/// entry1つぶんの差分から、引く行と足す行を視点ごとに集める。
/// `views` は視点ごとの特徴インデックスの作り方（ADR-0157）である。
fn diff_rows<'a, const V: usize>(
    net: &'a NnueNetwork,
    views: [View; V],
    dirty: &DirtyPiece,
    hand: Option<HandDelta>,
) -> DiffRows<'a, V> {
    let empty: &'a [FtWeight] = &[];
    let mut r = DiffRows {
        subs: [[empty; V]; 2],
        ns: 0,
        adds: [[empty; V]; 2],
        na: 0,
    };
    let row = |k: usize, bp: u16| -> &'a [FtWeight] {
        let idx = (views[k].base() + u32::from(bp)) as usize * FT_OUT;
        &net.ft_w[idx..idx + FT_OUT]
    };
    let push = |r: &mut DiffRows<'a, V>, rows: [&'a [FtWeight]; V], add: bool| {
        // 上限を超えたら添字で落ちる。黙って差分を捨てない
        if add {
            r.adds[r.na] = rows;
            r.na += 1;
        } else {
            r.subs[r.ns] = rows;
            r.ns += 1;
        }
    };
    for j in 0..dirty.count as usize {
        let old = dirty.piece_old[j];
        let from = dirty.from[j];
        if from != Square::NONE && !old.is_empty() && old.piece_type() != PieceType::KING {
            let rows = std::array::from_fn(|k| row(k, views[k].board_bona_piece(old, from)));
            push(&mut r, rows, false);
        }
        let new = dirty.piece_new[j];
        let to = dirty.to[j];
        if to != Square::NONE && !new.is_empty() && new.piece_type() != PieceType::KING {
            let rows = std::array::from_fn(|k| row(k, views[k].board_bona_piece(new, to)));
            push(&mut r, rows, true);
        }
    }
    if let Some(hd) = hand {
        let rows =
            std::array::from_fn(|k| row(k, views[k].hand_bona_piece(hd.owner, hd.kind, hd.slot)));
        push(&mut r, rows, hd.added);
    }
    r
}

/// 親のaccへ差分を適用して自分のaccへ書く（ADR-0151群A・群N）。
/// 行数ごとに融合カーネルを単相化する。実際に現れるのは
/// (0,0)（玉の移動・null move）・(1,1)（普通の手と駒打ち）・
/// (2,2)（取る手）の3通りで、残りは念のため用意する。
/// 視点の本数 `V` も定数なので、1視点版と両視点版が同じ形で書ける。
fn apply_rows<const V: usize>(
    dst: [&mut [i16; FT_OUT]; V],
    src: [&[i16; FT_OUT]; V],
    r: &DiffRows<'_, V>,
) {
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

    /// 玉位置ごとのキャッシュ（ADR-0156）が、別局面を挟んでも全計算と
    /// 一致し続けること。同じ玉位置に違う駒配置で何度も戻るので、
    /// キャッシュの中身と現局面が離れた状態からの差分を必ず踏む。
    #[test]
    fn bucket_cache_matches_full_computation_across_positions() {
        let net = NnueNetwork::random(11);
        let mut st = NnueState::new();
        let mut rng = Rng(0x1234_5678_9ABC_DEF1);
        for _ in 0..200 {
            let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
            for _ in 0..(rng.next() % 60) {
                let mut list = MoveList::default();
                generate_legal(&pos, true, &mut list);
                if list.is_empty() {
                    break;
                }
                let m = list.as_slice()[(rng.next() % list.len() as u64) as usize];
                pos.do_move(m);
            }
            // 探索のたびに積み直す経路（Worker::set_position）と同じ形にする
            st.reset();
            assert_eq!(
                st.evaluate(&net, &pos),
                evaluate_scalar(&net, &pos),
                "キャッシュ差分と全計算が一致しない: {}",
                pos.to_sfen()
            );
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
