//! スカラーの全計算の評価。差分計算とSIMDの正解器になる。

#[allow(clippy::wildcard_imports)]
use super::*;

/// accumulatorの前半と後半を要素ごとに掛けて活性にする。
///
/// `(clip(a) * clip(b) + 64) >> 7` の最大は 126 で、127には届かない。
/// **学習側が同じ127/128を掛けているので、これで縮尺が合う。**掛けずに
/// 127で割ると量子化の係数を後段まで直す羽目になる。
///
/// 64を足すのは切り捨てでなく四捨五入にするためである。足さないと活性が
/// 平均0.48ぶん小さく出る。1024次元すべてに同じ向きの偏りが乗るので、
/// バイアスでは吸収されない（足すと平均誤差が0.49から0.19へ下がる）。
#[inline]
pub(crate) fn pair_activation(a: i32, b: i32) -> u8 {
    ((i32::from(clip(a)) * i32::from(clip(b)) + 64) >> 7) as u8
}

/// スカラー全計算の評価（手番視点、歩=90スケール）。
/// 差分計算・SIMDの正解基準。
pub fn evaluate_scalar(net: &NnueNetwork, pos: &Position) -> Value {
    let stm = pos.side_to_move();
    let mut features = Vec::with_capacity(64);
    let mut concat = [0u8; CONCAT];

    // FT: 手番視点 → [0..HALF)、非手番視点 → [HALF..CONCAT)
    for (half, c) in [(0usize, stm), (1, stm.flip())] {
        halfkp_active(pos, c, &mut features);
        let mut acc = [0i32; FT_OUT];
        for (o, a) in acc.iter_mut().enumerate() {
            *a = i32::from(net.ft_b[o]);
        }
        for &f in &features {
            let base = f as usize * FT_OUT;
            for (o, a) in acc.iter_mut().enumerate() {
                *a += i32::from(net.ft_w[base + o]);
            }
        }
        // accの前半と後半を対にして掛ける
        for o in 0..HALF {
            concat[half * HALF + o] = pair_activation(acc[o], acc[o + HALF]);
        }
    }
    fill_effect(net, pos, &mut concat);

    forward_hidden(net, &concat)
}

/// 連結ベクトルから評価値まで（隠れ層はi8×u8の積和）。
pub(crate) fn forward_hidden(net: &NnueNetwork, concat: &[u8; CONCAT]) -> Value {
    // 隠れ層2の入力はL1_PAD幅で、L1_OUT以降はゼロのまま渡す
    let mut h2 = [0u8; L1_PAD];
    for (o, h) in h2[..L1_OUT].iter_mut().enumerate() {
        let mut sum = net.b2[o];
        for (i, &x) in concat.iter().enumerate() {
            sum += i32::from(net.w2[o * CONCAT + i]) * i32::from(x);
        }
        // 学習時のスケール（2^6）で割るのが標準
        *h = clip(sum >> 6);
    }
    // 隠れ層は書いたぶんだけ挟む。次元は定数なので使わない分岐は消える
    let mut h3 = [0u8; L2_OUT];
    for (o, h) in h3.iter_mut().enumerate() {
        let mut sum = net.b3[o];
        for (i, &x) in h2.iter().enumerate() {
            sum += i32::from(net.w3[o * L1_PAD + i]) * i32::from(x);
        }
        *h = clip(sum >> 6);
    }
    let last: &[u8] = if L2_OUT != 0 { &h3 } else { &h2[..L1_OUT] };

    let mut out = net.b_out;
    for (i, &x) in last.iter().enumerate() {
        out += i32::from(net.w_out[i]) * i32::from(x);
    }
    out / FV_SCALE
}
