//! 利き数の特徴（升×乗る駒×自分の利き数×相手の利き数。F2と呼ぶ）の全計算。
//!
//! 局面から利き数を数え、特徴の添字を作ってaccumulatorを積む。
//! 差分計算（`nnue_acc`）の正解器になる。

#[allow(clippy::wildcard_imports)]
use super::*;

/// 升ごとの利き数を色別に数える。添字は `[色][升]` で、升は盤の
/// 向きのまま。玉の利きを含め、ピンは無視する。その升の駒自身は数えない
/// （駒は自分の升へ利かない）。値は打ち切らない。
///
/// 盤上の駒を1周して利き先へ足す。`attackers_to` との一致はテストで確かめる。
pub fn effect_counts(pos: &Position) -> [[u8; 81]; 2] {
    let occ = pos.occupied();
    let mut n = [[0u8; 81]; 2];
    for sq in occ {
        let pc = pos.piece_on(sq);
        let side = &mut n[pc.color().index()];
        for to in attacks(pc, sq, occ) {
            side[to.index()] += 1;
        }
    }
    n
}

/// 利きの特徴のindex（F2）。`sq` は視点から見た升、`pc` は
/// 0（空き）・1〜14（自分の駒）・15〜28（相手の駒）、`own`・`opp` は
/// 自分と相手の利き数（打ち切り前）。
#[inline]
pub fn effect_index(sq: usize, pc: usize, own: u8, opp: u8) -> u32 {
    let cap = |n: u8| usize::from(n).min(EFF_LEVELS - 1);
    (((sq * EFF_PIECES + pc) * EFF_LEVELS + cap(own)) * EFF_LEVELS + cap(opp)) as u32
}

/// 視点cの利きの特徴を81個書く（F2）。升ごとに1つ立つ。
///
/// 視点ごとに盤を反転し、自他は視点から見る。HalfKPと同じ扱いなので、
/// 180度回して先後を入れ替えた局面は同じ特徴になる。`counts` は
/// `effect_counts` の結果を渡す。両視点で使い回すために外へ出してある。
pub fn effect_active(pos: &Position, c: Color, counts: &[[u8; 81]; 2], out: &mut [u32; 81]) {
    let (own, opp) = (&counts[c.index()], &counts[c.flip().index()]);
    for i in 0..81u8 {
        let sq = Square::from_index(i);
        let pc = pos.piece_on(sq);
        let code = if pc.is_empty() {
            0
        } else {
            // 駒種は2〜15の14種。自分を1〜14、相手を15〜28へ写す
            let kind = usize::from(pc.piece_type().0) - 1;
            if pc.color() == c { kind } else { kind + 14 }
        };
        let view = if c == Color::Black { sq } else { sq.inv() };
        out[view.index()] = effect_index(view.index(), code, own[sq.index()], opp[sq.index()]);
    }
}

/// 利きのaccumulatorを特徴から全計算する。i32で足すので、
/// 81行を足しても桁あふれしない。差分の経路はこの値との一致を求める。
pub fn effect_accumulate(net: &NnueNetwork, features: &[u32; 81]) -> [i32; EFF_OUT] {
    let mut acc = [0i32; EFF_OUT];
    for (a, &b) in acc.iter_mut().zip(&net.eff_b) {
        *a = i32::from(b);
    }
    for &f in features {
        effect_row_add(&mut acc, effect_row(net, f));
    }
    acc
}

/// 利きのaccumulatorが取りうる値の上限（絶対値）を、重みから見積もる。
///
/// 出力ごとに、バイアスと升ごとの重みの最大の絶対値を足す。1升に立つ特徴は
/// 1つなので、どの局面でもaccの絶対値はこれを超えない。i16に収まれば、
/// 差分の経路をi16の巡回加算で持っても、i32で足す全計算と一致する。
pub fn effect_acc_bound(net: &NnueNetwork) -> i64 {
    let per = EFF_PIECES * EFF_LEVELS * EFF_LEVELS;
    // 利きを持たない構成では重みが空で、下のループは回らない。chunksに0を
    // 渡せないので、そのときだけ幅を1と置く
    const WIDTH: usize = if EFF_OUT == 0 { 1 } else { EFF_OUT };
    let width = WIDTH;
    let mut sum: Vec<i64> = net.eff_b.iter().map(|&b| i64::from(b).abs()).collect();
    for rows in net.eff_w.chunks(per * width) {
        let mut most = vec![0i64; EFF_OUT];
        for row in rows.chunks(width) {
            for (m, &w) in most.iter_mut().zip(row) {
                *m = (*m).max(i64::from(w).abs());
            }
        }
        for (s, m) in sum.iter_mut().zip(&most) {
            *s += m;
        }
    }
    sum.into_iter().max().unwrap_or(0)
}

/// 利きの特徴 `f` の重みの行。
#[inline]
pub fn effect_row(net: &NnueNetwork, f: u32) -> &[i16] {
    let at = f as usize * EFF_OUT;
    &net.eff_w[at..at + EFF_OUT]
}

#[inline]
pub fn effect_row_add(acc: &mut [i32; EFF_OUT], row: &[i16]) {
    for (a, &w) in acc.iter_mut().zip(row) {
        *a += i32::from(w);
    }
}

/// 利きのaccumulatorを0〜127へクリップして書く。
#[inline]
pub fn effect_clip(acc: &[i32; EFF_OUT], out: &mut [u8]) {
    debug_assert_eq!(out.len(), EFF_OUT);
    for (o, &a) in out.iter_mut().zip(acc) {
        *o = clip(a);
    }
}

/// 差分で持つ側のaccを0〜127へクリップして書く。
#[inline]
pub fn effect_clip_acc(acc: &[i16; EFF_OUT], out: &mut [u8]) {
    debug_assert_eq!(out.len(), EFF_OUT);
    for (o, &a) in out.iter_mut().zip(acc) {
        *o = a.clamp(0, 127) as u8;
    }
}

/// 差分で持つ側のaccへバイアスを写す。
#[inline]
pub fn effect_acc_bias(net: &NnueNetwork, acc: &mut [i16; EFF_OUT]) {
    acc.copy_from_slice(&net.eff_b[..EFF_OUT]);
}

/// 利きのaccumulatorを特徴から作り直し、0〜127へクリップして書く。
///
/// 評価のたびに81行を足す。差分は持たない。対の積は掛けない。
pub fn effect_transform(net: &NnueNetwork, features: &[u32; 81], out: &mut [u8]) {
    effect_clip(&effect_accumulate(net, features), out);
}

/// 連結ベクトルの利きの部分（両視点）を埋める。手番視点が先に並ぶ。
/// `EFF_OUT` が0の構成では何もしない。
#[inline]
pub fn fill_effect(net: &NnueNetwork, pos: &Position, concat: &mut [u8; CONCAT]) {
    if EFF_OUT == 0 {
        return;
    }
    if EFF_GENERIC {
        crate::side_feat::fill(net, pos, concat);
        return;
    }
    let counts = effect_counts(pos);
    let stm = pos.side_to_move();
    let mut feats = [0u32; 81];
    for (k, c) in [(0usize, stm), (1, stm.flip())] {
        effect_active(pos, c, &counts, &mut feats);
        let at = EFF_BASE + k * EFF_OUT;
        effect_transform(net, &feats, &mut concat[at..at + EFF_OUT]);
    }
}
