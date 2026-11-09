//! 学習が使う指し手のラベル。推論は使わない。

#[allow(clippy::wildcard_imports)]
use super::*;

/// ラベルが取れなかったときの値。PyTorchの `ignore_index` に渡す。
pub const MOVE_NONE: i64 = -1;

/// `move16` を手番視点の (from, to) ラベルへ分解する。
///
/// HalfKPの特徴は手番視点で作る（`bonapiece::board_bona_piece` が後手番で
/// `sq.inv()` する）ので、指し手のラベルも同じ向きに揃える。揃えないと、
/// 同じ形の局面が先後で違うラベルになり、学習が割れる。
///
/// `from` は盤上81マスに打つ駒7種を続けた88クラス、`to` は81クラス。
/// **成りは落ちる。** どちらのヘッドにも現れない。
pub fn move_labels(m: u16, stm: Color) -> (i64, i64) {
    if m == 0 {
        return (MOVE_NONE, MOVE_NONE);
    }
    let flip = |sq: usize| -> usize { if stm == Color::Black { sq } else { 80 - sq } };
    let to_raw = (m & 0x7F) as usize;
    if to_raw >= 81 {
        return (MOVE_NONE, MOVE_NONE);
    }
    let to = flip(to_raw) as i64;

    let from_raw = ((m >> 7) & 0x7F) as usize;
    // bit15が立っていれば駒打ち。fromフィールドには駒種が入る
    let from = if m & (1 << 15) != 0 {
        match PieceType::HAND_KINDS
            .iter()
            .position(|pt| pt.0 as usize == from_raw)
        {
            Some(i) => (81 + i) as i64,
            None => return (MOVE_NONE, MOVE_NONE),
        }
    } else if from_raw < 81 {
        flip(from_raw) as i64
    } else {
        return (MOVE_NONE, MOVE_NONE);
    };
    (from, to)
}
