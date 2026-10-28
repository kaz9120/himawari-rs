//! NNUE accumulator差分計算（ADR-0035）。
//!
//! 第1塔（HalfKP FT）のみが対象。遅延方式: push/popではDirtyPieceを
//! 積むだけで、evaluateが呼ばれた時点で計算済みの祖先まで遡って
//! 差分を適用する。自玉が動いた視点は全計算（refresh）する。
//! 正しさの基準はnnue::evaluate_scalarとの完全一致。
//!
//! 利きの特徴（ADR-0237）を持つ構成では、利きのaccumulatorを別のスタックで
//! 持つ（D2）。評価のときに利き数の段階を集合で作り直し、計算済みの祖先と
//! 比べて、中身か段階が変わった升の行だけを足し引きする。

use himawari_core::{Bitboard, Color, DirtyPiece, Piece, PieceType, Position, Square, bonapiece};

use crate::nnue::{
    CONCAT, EFF_BASE, EFF_GENERIC, EFF_OUT, EffAcc, EffectLevels, FT_OUT, FtWeight, HALF,
    NnueNetwork, SIDE_MINATT_INC, effect_code, effect_code_index, side_f2_index, side_minatt_index,
};
use crate::nnue_simd;
use crate::value::{MAX_PLY, Value};

/// 評価関数が盤から読むもの（ADR-0241の第13回）。
///
/// coreの局面と探索の盤（`SearchBoard`）が実装する。探索の盤は、配列盤の研究のフィーチャ
/// `mailbox` ではcoreの局面を持たずに、配列盤から同じ値を返す。
pub trait EvalBoard {
    fn side_to_move(&self) -> Color;
    fn king(&self, c: Color) -> Square;
    fn hand_count(&self, owner: Color, pt: PieceType) -> u32;
    fn piece_on(&self, sq: Square) -> Piece;
    /// 直前の手の差分の材料。
    fn dirty(&self) -> DirtyPiece;
    /// 視点cのBonaPiece集合（`bonapiece::bona_piece_bits` と同じ値）。
    fn bona_piece_bits(&self, c: Color, out: &mut bonapiece::BonaBits);
    /// 利き数の段階の集合（`EffectLevels::build` と同じ値）。
    fn effect_levels(&self) -> EffectLevels;
    /// 81升の利きの語（`effect_code`）をまとめて書く（ADR-0242の段1を配列盤へ移した形）。
    /// 配列盤は利き数と駒の配列から直接作れるので、段階の集合を経由しない。
    fn effect_codes(&self, out: &mut [u16; 81]);
    /// 81升の、F2と最小の攻め駒の語（`side_feat::f2_minatt_codes` と同じ値、ADR-0248）。
    /// 既定の実装は盤から数え直す正解器で遅い。探索の配列盤は速い版で上書きする。
    fn side_codes(&self, out: &mut [u16; 81]) {
        crate::side_feat::f2_minatt_codes(self, out);
    }
    /// 駒割り（先手から見た値）。駒割りだけの評価関数が使う。
    fn material(&self) -> i32;
    /// 全計算との突き合わせ（テストとデバッグビルド）に使うcoreの局面。無ければ突き合わせない。
    fn core_position(&self) -> Option<&Position>;
}

impl EvalBoard for Position {
    #[inline(always)]
    fn side_to_move(&self) -> Color {
        Position::side_to_move(self)
    }
    #[inline(always)]
    fn king(&self, c: Color) -> Square {
        Position::king(self, c)
    }
    #[inline(always)]
    fn hand_count(&self, owner: Color, pt: PieceType) -> u32 {
        self.hand(owner).count(pt)
    }
    #[inline(always)]
    fn piece_on(&self, sq: Square) -> Piece {
        Position::piece_on(self, sq)
    }
    #[inline(always)]
    fn dirty(&self) -> DirtyPiece {
        self.state().dirty
    }
    #[inline(always)]
    fn bona_piece_bits(&self, c: Color, out: &mut bonapiece::BonaBits) {
        bonapiece::bona_piece_bits(self, c, out);
    }
    #[inline(always)]
    fn effect_levels(&self) -> EffectLevels {
        EffectLevels::build(self)
    }
    #[inline(always)]
    fn effect_codes(&self, out: &mut [u16; 81]) {
        let levels = EffectLevels::build(self);
        for (i, o) in out.iter_mut().enumerate() {
            *o = effect_code(self, &levels, Square::from_index(i as u8));
        }
    }
    #[inline(always)]
    fn material(&self) -> i32 {
        self.state().material
    }
    #[inline(always)]
    fn core_position(&self) -> Option<&Position> {
        Some(self)
    }
}

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

/// 利きの段（ADR-0237のD2）。`EFF_OUT` が0の構成では確保しない。
///
/// 祖先と比べるために、段階の集合と升ごとの語（`effect_code`）を持つ。
/// accの要素は `EffAcc` で、巡回加算で足す。F2だけの構成はi16で持ち、読み込み時に
/// `effect_acc_bound` がi16に収まることを確かめてあるので、i32で足す全計算（D3）と
/// 一致する。F2と最小の攻め駒の組はi32で持ち、どの重みでも溢れない（ADR-0249）。
#[derive(Clone)]
struct EffEntry {
    /// 絶対色ごとの利きのaccumulator。
    acc: [[EffAcc; EFF_OUT]; 2],
    /// 升ごとの中身と段階（`effect_code`）。
    codes: [u16; 81],
    /// falseの間は中身が未定義で、誰も読まない。
    computed: bool,
    /// `acc` と `codes` が、ある1つの局面を表している。pushで `computed` を
    /// 落としても中身は残るので、差分の参照に使える（ADR-0242の段1）。
    coherent: bool,
}

impl EffEntry {
    fn empty() -> EffEntry {
        EffEntry {
            acc: [[0; EFF_OUT]; 2],
            codes: [0; 81],
            computed: false,
            coherent: false,
        }
    }
}

/// 祖先との差分で変わった升がこれを超えたら、81升を全計算する。
/// 1升あたり引く行と足す行の2本を払うので、およそ半分で損益が分かれる。
const EFF_REFRESH_ABOVE: u32 = 40;

/// 積める段数。rootが0段目で、探索は `ply >= MAX_PLY` で即座に返るため、
/// 実際に積まれるのは最大 `MAX_PLY` 段になる。余白を足して越えを防ぐ。
const MAX_ENTRIES: usize = MAX_PLY + 16;

/// BonaPiece集合を保持するブロック数。
const BP_BLOCKS: usize = bonapiece::BP_BLOCKS;

/// 玉位置ごとのキャッシュの件数（視点色 × 玉の81升）。
const FINNY_ENTRIES: usize = 2 * 81;

/// 玉位置ごとのaccumulatorキャッシュ1件（ADR-0156）。
///
/// `acc` は `bits` が表すBonaPiece集合をバイアスへ足し込んだ状態にある。
/// 玉が動いた視点は差分連鎖を遡れないが、**同じ玉位置の局面をここに
/// 1件だけ残しておけば、全計算の代わりに集合の差分で済む。**
#[repr(C, align(64))]
struct FinnyEntry {
    acc: [i16; FT_OUT],
    /// `acc` に反映済みのBonaPiece集合。
    bits: bonapiece::BonaBits,
    /// falseの間は `acc`・`bits` の中身が未定義で、誰も読まない。
    valid: bool,
}

impl FinnyEntry {
    fn empty() -> FinnyEntry {
        FinnyEntry {
            acc: [0; FT_OUT],
            bits: [0; BP_BLOCKS],
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
    /// 利きの段（ADR-0237のD2）。`entries` と同じ添字で並ぶ。
    /// `EFF_OUT` が0の構成では空で、誰も触らない。
    eff: Vec<EffEntry>,
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
            eff: std::iter::repeat_with(EffEntry::empty)
                .take(if EFF_OUT != 0 { MAX_ENTRIES } else { 0 })
                .collect(),
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
        if EFF_OUT != 0 {
            self.eff[0].computed = false;
        }
    }

    /// do_move直後のposで呼ぶ。差分材料だけ積む（計算はしない）。
    pub fn push<B: EvalBoard + ?Sized>(&mut self, pos: &B) {
        let dirty = pos.dirty();
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
        if EFF_OUT != 0 {
            self.eff[self.top].computed = false;
        }
    }

    pub fn pop(&mut self) {
        debug_assert!(self.top > 0);
        self.top -= 1;
    }

    /// 現局面の評価値。必要な視点だけ遡り差分または全計算で作る。
    ///
    /// 盤は `EvalBoard` 越しに読む。coreの局面のほか、探索の盤（`SearchBoard`）も渡せる
    /// （ADR-0241の第13回）。
    pub fn evaluate<B: EvalBoard + ?Sized>(&mut self, net: &NnueNetwork, pos: &B) -> Value {
        self.ensure_both(net, pos);
        let stm = pos.side_to_move();
        let top = &self.entries[self.top];
        let mut concat = [0u8; CONCAT];
        for (half, c) in [(0usize, stm), (1, stm.flip())] {
            debug_assert!(top.computed[c.index()], "未計算のaccを読もうとした");
            let acc = &top.acc[c.index()];
            nnue_simd::clip_to_u8(acc, &mut concat[half * HALF..(half + 1) * HALF]);
        }
        if EFF_GENERIC && !SIDE_MINATT_INC {
            // 表し方を替えた構成は、評価のたびに作り直す（ADR-0245）
            crate::side_feat::fill(net, pos, &mut concat);
        } else if EFF_OUT != 0 {
            // 利きの部分は祖先からの差分で作る（ADR-0237のD2）。F2と最小の攻め駒の組も
            // 同じ升の語の差分で持つ（ADR-0248）
            self.ensure_effect(net, pos);
            let e = &self.eff[self.top];
            for (k, c) in [(0usize, stm), (1, stm.flip())] {
                let at = EFF_BASE + k * EFF_OUT;
                crate::nnue::effect_clip_acc(&e.acc[c.index()], &mut concat[at..at + EFF_OUT]);
            }
            if cfg!(any(test, debug_assertions))
                && let Some(p) = pos.core_position()
            {
                self.check_effect(net, p);
            }
        }
        // 玉の近傍の部分モデル（ADR-0246）。持たない構成では何もしない
        crate::submodel::fill(&net.sub, pos, &mut concat);
        #[cfg(feature = "actdump")]
        crate::actdump::record(&concat);
        nnue_simd::forward_hidden(net, &concat)
    }

    /// 利きのaccumulatorを最上段に用意する（ADR-0237のD2、ADR-0242の段1・段2）。
    ///
    /// 81升の語を盤から受け取り、参照の局面の語と比べて変わった升だけを当てる。
    /// 参照の候補は、計算済みの祖先と、この段に残る前回の中身（兄弟か従兄弟の
    /// 局面）の2つで、変わった升が少ないほうを使う。**途中の手の材料は要らない。**
    /// どちらも40升を超えたら全計算する。利きの特徴は玉位置に依らないので、
    /// 玉が動いても参照にできる。
    fn ensure_effect<B: EvalBoard + ?Sized>(&mut self, net: &NnueNetwork, pos: &B) {
        let top = self.top;
        if self.eff[top].computed {
            return;
        }
        let mut codes = [0u16; 81];
        if SIDE_MINATT_INC {
            pos.side_codes(&mut codes);
        } else {
            pos.effect_codes(&mut codes);
        }
        let here = &self.eff[top];
        let from_here = here.coherent.then(|| codes_diff(&codes, &here.codes));
        let anc = (0..top).rev().find(|&i| self.eff[i].computed);
        let from_anc = anc.map(|a| (a, codes_diff(&codes, &self.eff[a].codes)));
        let far = |c: &Bitboard| c.count() > EFF_REFRESH_ABOVE;
        match (from_here, from_anc) {
            // 同数なら、写さずに済むこの段を使う
            (Some(h), a) if !far(&h) && a.is_none_or(|(_, c)| h.count() <= c.count()) => {
                effect_apply(net, &codes, &mut self.eff[top], h);
            }
            (_, Some((a, c))) if !far(&c) => {
                let (before, after) = self.eff.split_at_mut(top);
                let e = &mut after[0];
                e.acc = before[a].acc;
                e.codes = before[a].codes;
                effect_apply(net, &codes, e, c);
            }
            _ => effect_refresh(net, &codes, &mut self.eff[top]),
        }
        let e = &mut self.eff[top];
        e.codes = codes;
        e.computed = true;
        e.coherent = true;
    }

    /// 最上段の利きの段を、全計算（D3）と突き合わせる。テストとデバッグビルド
    /// だけで、評価のたびに呼ぶ。accの値・升ごとの語・段階の集合を見る。
    fn check_effect(&self, net: &NnueNetwork, pos: &Position) {
        let e = &self.eff[self.top];
        if SIDE_MINATT_INC {
            let mut want = [0u16; 81];
            crate::side_feat::f2_minatt_codes(pos, &mut want);
            assert_eq!(e.codes, want, "升の語が食い違う: {}", pos.to_sfen());
            let sb = crate::side_feat::SideBoard::new(pos);
            let mut feats = Vec::new();
            for c in [Color::Black, Color::White] {
                feats.clear();
                sb.active(c, &mut feats);
                let full = crate::side_feat::accumulate(net, &feats);
                assert_eq!(
                    e.acc[c.index()].map(i32::from),
                    full,
                    "側のaccが作り直しと食い違う: 視点{c:?} {}",
                    pos.to_sfen()
                );
            }
            return;
        }
        let counts = crate::nnue::effect_counts(pos);
        let levels = EffectLevels::build(pos);
        for i in 0..81u8 {
            let sq = Square::from_index(i);
            assert_eq!(
                e.codes[sq.index()],
                effect_code(pos, &levels, sq),
                "升の語が食い違う: sq={} {}",
                sq.to_usi(),
                pos.to_sfen()
            );
        }
        let mut feats = [0u32; 81];
        for c in [Color::Black, Color::White] {
            crate::nnue::effect_active(pos, c, &counts, &mut feats);
            let full = crate::nnue::effect_accumulate(net, &feats);
            assert_eq!(
                e.acc[c.index()].map(i32::from),
                full,
                "利きのaccが全計算と食い違う: 視点{c:?} {}",
                pos.to_sfen()
            );
        }
    }

    /// 両視点のaccumulatorを最上段に用意する（ADR-0151群N）。
    ///
    /// 視点ごとに遡れる範囲は違う。**両色をまとめて適用できるのは、
    /// 遅いほうの起点から上の区間だけである。** 手前の区間は片色ずつ
    /// 埋め、そこから上を1パスで両色へ書く。玉が動いた視点は遡れないので
    /// 全計算になり、その視点は融合の対象から外れる。
    fn ensure_both<B: EvalBoard + ?Sized>(&mut self, net: &NnueNetwork, pos: &B) {
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
    fn apply_range<B: EvalBoard + ?Sized>(
        &mut self,
        net: &NnueNetwork,
        pos: &B,
        c: Color,
        from: usize,
        to: usize,
    ) {
        // この区間に視点cの玉移動はないので、玉位置は現局面のもので通る
        let king = pos.king(c);
        for j in from + 1..=to {
            let (before, after) = self.entries.split_at_mut(j);
            let prev = &before[j - 1];
            debug_assert!(prev.computed[c.index()], "未計算のaccを読もうとした");
            let e = &mut after[0];
            // 借用が分かれているので、親のaccを読みながら直接書ける。
            // 複製と足し引きを1パスに融合する（ADR-0151群A）
            let rows = diff_rows(net, [(c, king)], &e.dirty, e.hand);
            apply_rows([&mut e.acc[c.index()]], [&prev.acc[c.index()]], &rows);
            e.computed[c.index()] = true;
        }
    }

    /// 同上を両視点まとめて行う（ADR-0151群N）。連鎖の走査とdirtyの
    /// デコードが1回で済み、accへの読み書きが2本のストリームで並ぶ。
    fn apply_range_both<B: EvalBoard + ?Sized>(
        &mut self,
        net: &NnueNetwork,
        pos: &B,
        from: usize,
        to: usize,
    ) {
        let views = [
            (Color::Black, pos.king(Color::Black)),
            (Color::White, pos.king(Color::White)),
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
    fn refresh_top<B: EvalBoard + ?Sized>(&mut self, net: &NnueNetwork, pos: &B, c: Color) {
        let king = pos.king(c);
        // 玉位置を固定した特徴インデックスの起点。BonaPieceを足せば
        // そのまま特徴インデックスになる
        let base = bonapiece::halfkp_index(c, king, 0);
        let mut cur = [0u128; BP_BLOCKS];
        pos.bona_piece_bits(c, &mut cur);

        // scratch・finny・entriesを同時に借りられないので、いったん取り出して
        // 戻す。takeは空Vecとの交換なので確保は起きず、容量も残る
        let mut s = std::mem::take(&mut self.scratch);
        let mut finny = std::mem::take(&mut self.finny);
        let e = &mut finny[c.index() * 81 + king.index()];
        if !e.valid {
            e.acc.copy_from_slice(&net.ft_b[..FT_OUT]);
            e.bits = [0; BP_BLOCKS];
            e.valid = true;
        }
        // ブロックの並びはBonaPiece番号の昇順なので、adds・subsは語で分けて
        // いた頃と同じ順に並ぶ。反映済み集合はここで更新し、別のコピーを
        // 挟まない（ADR-0165）
        for (b, (&now, had)) in cur.iter().zip(e.bits.iter_mut()).enumerate() {
            if now == *had {
                continue;
            }
            let off = base + bonapiece::block_base(b);
            let mut added = now & !*had;
            while added != 0 {
                s.adds.push(off + added.trailing_zeros());
                added &= added - 1;
            }
            let mut removed = *had & !now;
            while removed != 0 {
                s.subs.push(off + removed.trailing_zeros());
                removed &= removed - 1;
            }
            *had = now;
        }
        nnue_simd::ft_update(&mut e.acc, &net.ft_w, &s.adds, &s.subs);

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
fn hand_delta_of<B: EvalBoard + ?Sized>(pos: &B, dirty: &DirtyPiece) -> Option<HandDelta> {
    if dirty.count == 2 {
        // キャプチャ: piece_new[1]が手駒に入った駒（枚数は取った後の値）
        let pc = dirty.piece_new[1];
        let owner = pc.color();
        let kind = pc.piece_type();
        Some(HandDelta {
            owner,
            kind,
            slot: pos.hand_count(owner, kind),
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
            slot: pos.hand_count(owner, kind) + 1,
            added: false,
        })
    } else {
        None
    }
}

/// 2つの局面の利きの語が違う升（ADR-0242の段1）。8升ずつベクトルで比べる。
#[inline]
fn codes_diff(a: &[u16; 81], b: &[u16; 81]) -> Bitboard {
    use std::simd::Simd;
    use std::simd::cmp::SimdPartialEq;
    let (ac, at) = a.as_chunks::<8>();
    let (bc, bt) = b.as_chunks::<8>();
    let mut m = 0u128;
    for (k, (x, y)) in ac.iter().zip(bc).enumerate() {
        let ne = Simd::from_array(*x).simd_ne(Simd::from_array(*y));
        m |= u128::from(ne.to_bitmask() as u8) << (k * 8);
    }
    if at[0] != bt[0] {
        m |= 1 << 80;
    }
    Bitboard::from_bits(m)
}

/// 変わった升の行だけを入れ替える（ADR-0237のD2）。`e` は参照の局面の
/// accと升の語を持っている。両視点をまとめて当てる（ADR-0242の段2）。
fn effect_apply(net: &NnueNetwork, codes: &[u16; 81], e: &mut EffEntry, changed: Bitboard) {
    let old_codes = &e.codes;
    let mut sqs = changed.into_iter();
    if SIDE_MINATT_INC {
        // 語が変わった升で、F2の行と最小の攻め駒の行のうち変わったほうだけを入れ替える
        // （ADR-0248）。F2の行は下位9ビット、最小の攻め駒の行は駒と階級（4ビット目から上）で決まる
        let mut pending = None;
        nnue_simd::effect_rows_update(net, &mut e.acc, || {
            if let Some(p) = pending.take() {
                return Some(p);
            }
            loop {
                let sq = sqs.next()?;
                let (old, new) = (old_codes[sq.index()], codes[sq.index()]);
                let ma = ((old ^ new) & !0xF != 0)
                    .then(|| (Some(side_minatt_pair(sq, old)), side_minatt_pair(sq, new)));
                if (old ^ new) & 0x1FF != 0 {
                    pending = ma;
                    return Some((Some(side_f2_pair(sq, old)), side_f2_pair(sq, new)));
                }
                if ma.is_some() {
                    return ma;
                }
            }
        });
        return;
    }
    nnue_simd::effect_rows_update(net, &mut e.acc, || {
        let sq = sqs.next()?;
        let (old, new) = (old_codes[sq.index()], codes[sq.index()]);
        Some((Some(effect_code_pair(sq, old)), effect_code_pair(sq, new)))
    });
}

/// 両視点の特徴の添字。`[Color; 2]::map` の閉包はインライン化されずに残ったので、
/// 並べて書く（ADR-0241の第15回）。
#[inline(always)]
fn effect_code_pair(sq: Square, code: u16) -> [u32; 2] {
    [
        effect_code_index(Color::Black, sq, code),
        effect_code_index(Color::White, sq, code),
    ]
}

/// `side_codes` の語のF2の部分の、両視点の特徴の添字（ADR-0248）。
#[inline(always)]
fn side_f2_pair(sq: Square, code: u16) -> [u32; 2] {
    [
        side_f2_index(Color::Black, sq, code),
        side_f2_index(Color::White, sq, code),
    ]
}

/// `side_codes` の語の最小の攻め駒の部分の、両視点の特徴の添字（ADR-0248）。
#[inline(always)]
fn side_minatt_pair(sq: Square, code: u16) -> [u32; 2] {
    [
        side_minatt_index(Color::Black, sq, code),
        side_minatt_index(Color::White, sq, code),
    ]
}

/// 81升を全計算する（ADR-0237のD3と同じ値になる）。
fn effect_refresh(net: &NnueNetwork, codes: &[u16; 81], e: &mut EffEntry) {
    for c in [Color::Black, Color::White] {
        crate::nnue::effect_acc_bias(net, &mut e.acc[c.index()]);
    }
    if SIDE_MINATT_INC {
        // 1升にF2と最小の攻め駒の2行を足す（ADR-0248）
        let mut i = 0u8;
        nnue_simd::effect_rows_update(net, &mut e.acc, || {
            if i == 162 {
                return None;
            }
            let sq = Square::from_index(i / 2);
            let code = codes[sq.index()];
            i += 1;
            Some(if i % 2 == 1 {
                (None, side_f2_pair(sq, code))
            } else {
                (None, side_minatt_pair(sq, code))
            })
        });
        return;
    }
    let mut i = 0u8;
    nnue_simd::effect_rows_update(net, &mut e.acc, || {
        if i == 81 {
            return None;
        }
        let sq = Square::from_index(i);
        i += 1;
        Some((None, effect_code_pair(sq, codes[sq.index()])))
    });
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
/// `views` は視点色と、その視点の玉位置の組である。
fn diff_rows<'a, const V: usize>(
    net: &'a NnueNetwork,
    views: [(Color, Square); V],
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
        let (c, king) = views[k];
        let idx = bonapiece::halfkp_index(c, king, bp) as usize * FT_OUT;
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
            let rows =
                std::array::from_fn(|k| row(k, bonapiece::board_bona_piece(views[k].0, old, from)));
            push(&mut r, rows, false);
        }
        let new = dirty.piece_new[j];
        let to = dirty.to[j];
        if to != Square::NONE && !new.is_empty() && new.piece_type() != PieceType::KING {
            let rows =
                std::array::from_fn(|k| row(k, bonapiece::board_bona_piece(views[k].0, new, to)));
            push(&mut r, rows, true);
        }
    }
    if let Some(hd) = hand {
        let rows = std::array::from_fn(|k| {
            row(
                k,
                bonapiece::hand_bona_piece(views[k].0, hd.owner, hd.kind, hd.slot),
            )
        });
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

    /// `bona_piece_bits`（ADR-0164・ADR-0165）が、駒を1枚ずつ回して立てた
    /// 集合と一致すること。`for_each_bona_piece` を正解器として残してある。
    ///
    /// 正解器の側は語で区切ったビットセットを作り、ブロック表現へ写して
    /// 比べる。**写像のうえで一致するなら、差分を取り出す順序もBonaPiece
    /// 番号の昇順で変わらない。**
    #[test]
    fn bona_piece_bits_matches_one_by_one() {
        let mut rng = Rng(0x0BAD_C0DE_1234_5678);
        let mut bits = [0u128; BP_BLOCKS];
        for _ in 0..300 {
            let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
            for _ in 0..(rng.next() % 80) {
                let mut list = MoveList::default();
                generate_legal(&pos, true, &mut list);
                if list.is_empty() {
                    break;
                }
                let m = list.as_slice()[(rng.next() % list.len() as u64) as usize];
                pos.do_move(m);
            }
            for c in [Color::Black, Color::White] {
                let mut want = [0u128; BP_BLOCKS];
                crate::nnue::for_each_bona_piece(&pos, c, |bp| {
                    // BonaPiece番号がどのブロックの何ビット目にあたるか
                    let b = match u32::from(bp) {
                        n if n < bonapiece::block_base(1) => 0,
                        n => 1 + (n - bonapiece::block_base(1)) as usize / 81,
                    };
                    want[b] |= 1u128 << (u32::from(bp) - bonapiece::block_base(b));
                });
                bonapiece::bona_piece_bits(&pos, c, &mut bits);
                assert_eq!(bits, want, "集合が一致しない: {} 視点{c:?}", pos.to_sfen());
            }
        }
    }

    /// 評価を飛ばすノードを混ぜた進め戻し。利きの段（ADR-0237のD2）は
    /// 計算済みの祖先が数手上にある状態から差分を当てるので、その経路を踏む。
    /// 評価したノードでは、`evaluate` の中の検査が利きのaccまで突き合わせる。
    fn walk_sparse(
        net: &NnueNetwork,
        pos: &mut Position,
        st: &mut NnueState,
        rng: &mut Rng,
        depth: usize,
    ) -> u64 {
        let mut n = 0;
        if rng.next().is_multiple_of(3) || depth == 0 {
            assert_eq!(
                st.evaluate(net, pos),
                evaluate_scalar(net, pos),
                "差分と全計算が一致しない: {}",
                pos.to_sfen()
            );
            n += 1;
        }
        if depth == 0 {
            return n;
        }
        for _ in 0..2 {
            let mut list = MoveList::default();
            generate_legal(pos, true, &mut list);
            if list.is_empty() {
                return n;
            }
            if !pos.in_check() && rng.next().is_multiple_of(8) {
                pos.do_null_move();
                st.push(pos);
                n += walk_sparse(net, pos, st, rng, depth - 1);
                st.pop();
                pos.undo_null_move();
            } else {
                let m = list.as_slice()[(rng.next() % list.len() as u64) as usize];
                pos.do_move(m);
                st.push(pos);
                n += walk_sparse(net, pos, st, rng, depth - 1);
                st.pop();
                pos.undo_move(m);
            }
        }
        n
    }

    #[test]
    fn incremental_matches_full_computation_with_skipped_evals() {
        let net = NnueNetwork::random(13);
        let mut evaluated = 0;
        for seed in 1..=8u64 {
            let mut rng = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03));
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
            let mut st = NnueState::new();
            evaluated += walk_sparse(&net, &mut pos, &mut st, &mut rng, 7);
        }
        assert!(evaluated > 300, "評価したノードが少なすぎる: {evaluated}");
    }

    /// 探索の盤（既定では配列盤）とcoreの局面を並べて進め戻し、探索の盤で差分に積んだ評価値が
    /// coreの局面の全計算と一致すること（ADR-0248）。配列盤は `core_position` を返さないので、
    /// `evaluate` の中の検査は働かない。ここで外から `check_effect` を呼ぶ。評価を飛ばすノードも混ぜる。
    fn walk_board(
        net: &NnueNetwork,
        pos: &mut Position,
        sb: &mut crate::board::SearchBoard,
        st: &mut NnueState,
        rng: &mut Rng,
        depth: usize,
    ) -> u64 {
        let mut n = 0;
        if rng.next().is_multiple_of(3) || depth == 0 {
            assert_eq!(
                st.evaluate(net, sb),
                evaluate_scalar(net, pos),
                "探索の盤の差分と全計算が一致しない: {}",
                pos.to_sfen()
            );
            // 評価値は活性のクリップで差を吸収しうるので、側のaccと升の語もcoreの局面で突き合わせる
            if EFF_OUT != 0 && (!EFF_GENERIC || SIDE_MINATT_INC) {
                st.check_effect(net, pos);
            }
            n += 1;
        }
        if depth == 0 {
            return n;
        }
        for _ in 0..2 {
            let mut list = MoveList::default();
            generate_legal(pos, true, &mut list);
            if list.is_empty() {
                return n;
            }
            if !pos.in_check() && rng.next().is_multiple_of(8) {
                pos.do_null_move();
                sb.do_null_move();
                st.push(sb);
                n += walk_board(net, pos, sb, st, rng, depth - 1);
                st.pop();
                sb.undo_null_move();
                pos.undo_null_move();
            } else {
                let m = list.as_slice()[(rng.next() % list.len() as u64) as usize];
                pos.do_move(m);
                sb.do_move(m);
                st.push(sb);
                n += walk_board(net, pos, sb, st, rng, depth - 1);
                st.pop();
                sb.undo_move(m);
                pos.undo_move(m);
            }
        }
        n
    }

    #[test]
    fn search_board_incremental_matches_full_computation() {
        let net = NnueNetwork::random(17);
        let mut evaluated = 0;
        for seed in 1..=8u64 {
            let mut rng = Rng(seed.wrapping_mul(0xA076_1D64_78BD_642F));
            let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
            for _ in 0..(rng.next() % 90) {
                let mut list = MoveList::default();
                generate_legal(&pos, true, &mut list);
                if list.is_empty() {
                    break;
                }
                let m = list.as_slice()[(rng.next() % list.len() as u64) as usize];
                pos.do_move(m);
            }
            let mut sb = crate::board::SearchBoard::new(pos.clone());
            let mut st = NnueState::new();
            evaluated += walk_board(&net, &mut pos, &mut sb, &mut st, &mut rng, 7);
        }
        assert!(evaluated > 300, "評価したノードが少なすぎる: {evaluated}");
    }

    /// F2と最小の攻め駒の組で、側のaccがi16を超えるネットでも差分が全計算と一致すること
    /// （ADR-0249）。ランダムなネットの側の重みを3000倍して、見積もりをi16の上限より大きくする。
    #[test]
    fn side_minatt_incremental_matches_beyond_i16() {
        if !SIDE_MINATT_INC {
            return;
        }
        let mut net = NnueNetwork::random(19);
        for w in &mut net.eff_w {
            *w *= 3000;
        }
        assert!(crate::nnue::side_minatt_acc_bound(&net) > 100 * i64::from(i16::MAX));
        let (mut evaluated, mut widest) = (0, 0i64);
        for seed in 1..=8u64 {
            let mut rng = Rng(seed.wrapping_mul(0x8EBC_6AF0_9C88_C6E3));
            let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
            for _ in 0..(rng.next() % 90) {
                let mut list = MoveList::default();
                generate_legal(&pos, true, &mut list);
                if list.is_empty() {
                    break;
                }
                let m = list.as_slice()[(rng.next() % list.len() as u64) as usize];
                pos.do_move(m);
            }
            // coreの局面で進め戻すと、`evaluate` の中の検査がaccを突き合わせる
            let mut st = NnueState::new();
            evaluated += walk_sparse(&net, &mut pos.clone(), &mut st, &mut rng, 6);
            let mut sb = crate::board::SearchBoard::new(pos.clone());
            let mut st = NnueState::new();
            evaluated += walk_board(&net, &mut pos, &mut sb, &mut st, &mut rng, 6);
            let root = st.eff[0].acc.as_flattened().iter();
            widest = widest.max(root.map(|&a| i64::from(a).abs()).max().unwrap());
        }
        assert!(evaluated > 300, "評価したノードが少なすぎる: {evaluated}");
        assert!(
            widest > i64::from(i16::MAX),
            "accがi16を超えていない: {widest}"
        );
    }

    /// 探索の盤が作るF2と最小の攻め駒の語（配列盤では `effect_minatt_codes`）が、
    /// `side_feat` から作る正解器と一致すること（ADR-0248）。構成に依らず走る。
    #[test]
    fn search_board_side_codes_match_side_feat() {
        let mut rng = Rng(0x5DEE_CE66_D1CE_4E5B);
        let mut checked = 0;
        for _ in 0..200 {
            let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
            let mut sb = crate::board::SearchBoard::new(pos.clone());
            for _ in 0..(rng.next() % 160) {
                let mut list = MoveList::default();
                generate_legal(&pos, true, &mut list);
                if list.is_empty() {
                    break;
                }
                let m = list.as_slice()[(rng.next() % list.len() as u64) as usize];
                pos.do_move(m);
                sb.do_move(m);
                let (mut got, mut want) = ([0u16; 81], [0u16; 81]);
                sb.side_codes(&mut got);
                crate::side_feat::f2_minatt_codes(&pos, &mut want);
                assert_eq!(got, want, "升の語が食い違う: {}", pos.to_sfen());
                checked += 1;
            }
        }
        assert!(checked > 5000, "確かめた局面が少なすぎる: {checked}");
    }

    /// 語の最小の攻め駒の部分から戻した添字が、`side_feat` の `KIND_MINATT` と一致すること。
    /// 構成に依らず、添字の式だけを確かめる。
    #[test]
    fn side_minatt_index_matches_side_feat() {
        let base = (81 * crate::nnue::EFF_PIECES * 16) as u32;
        let mut rng = Rng(0x1357_9BDF_2468_ACE0);
        for _ in 0..200 {
            let mut pos = Position::from_sfen(SFEN_STARTPOS).unwrap();
            for _ in 0..(rng.next() % 120) {
                let mut list = MoveList::default();
                generate_legal(&pos, true, &mut list);
                if list.is_empty() {
                    break;
                }
                let m = list.as_slice()[(rng.next() % list.len() as u64) as usize];
                pos.do_move(m);
            }
            let mut codes = [0u16; 81];
            crate::side_feat::f2_minatt_codes(&pos, &mut codes);
            let sb = crate::side_feat::SideBoard::new(&pos);
            for c in [Color::Black, Color::White] {
                let mut want = Vec::new();
                sb.kind(crate::side_feat::KIND_MINATT, c, base, &mut want);
                let mut got: Vec<u32> = (0..81u8)
                    .map(|i| {
                        let sq = Square::from_index(i);
                        side_minatt_index(c, sq, codes[sq.index()])
                    })
                    .collect();
                let mut f2_want = Vec::new();
                sb.kind(crate::side_feat::KIND_F2, c, 0, &mut f2_want);
                let mut f2_got: Vec<u32> = (0..81u8)
                    .map(|i| {
                        let sq = Square::from_index(i);
                        side_f2_index(c, sq, codes[sq.index()])
                    })
                    .collect();
                want.sort_unstable();
                got.sort_unstable();
                f2_want.sort_unstable();
                f2_got.sort_unstable();
                assert_eq!(got, want, "最小の攻め駒の添字: {}", pos.to_sfen());
                assert_eq!(f2_got, f2_want, "F2の添字: {}", pos.to_sfen());
            }
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
