//! HalfKPの活性特徴の列挙。

#[allow(clippy::wildcard_imports)]
use super::*;

/// 視点cのHalfKP活性特徴（玉以外の盤上駒＋両者の持ち駒）のBonaPieceを
/// 順に渡す。玉位置のオフセットを掛ける前の形で、これを起点に
/// 特徴インデックスにもキャッシュの鍵にもできる。
/// 盤上は駒のある升だけを走る。81升を舐めると空升の判定が3分の2を
/// 占め、全計算のほうで列挙が支配的になる。
#[inline]
pub fn for_each_bona_piece(pos: &Position, c: Color, mut f: impl FnMut(u16)) {
    let kings = Bitboard::from_square(pos.king(Color::Black))
        | Bitboard::from_square(pos.king(Color::White));
    for sq in pos.occupied() ^ kings {
        f(bonapiece::board_bona_piece(c, pos.piece_on(sq), sq));
    }
    for owner in [Color::Black, Color::White] {
        let hand = pos.hand(owner);
        for pt in PieceType::HAND_KINDS {
            for i in 1..=hand.count(pt) {
                f(bonapiece::hand_bona_piece(c, owner, pt, i));
            }
        }
    }
}

/// 視点cのHalfKP活性特徴（玉以外の盤上駒＋両者の持ち駒）を列挙する。
pub fn halfkp_active(pos: &Position, c: Color, out: &mut Vec<u32>) {
    out.clear();
    let king = pos.king(c);
    for_each_bona_piece(pos, c, |bp| {
        out.push(bonapiece::halfkp_index(c, king, bp));
    });
}
