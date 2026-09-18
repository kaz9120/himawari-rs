"""影の利き属性の反転数の数え方を検証する（ADR-0213）。

期待値は初期局面から手で数える。盤はcshogiの並び（マス番号は
`(筋 - 1) * 9 + 段`、段はaを0とする）で持ち、その並びがcshogiと
一致することも別のテストで確かめる。
"""

import pytest

from hmwr.tools import shadow_flips as sf

# 初期局面。9つの筋を、筋1から筋9の順に、各筋は段aから段iの順で並べる
INITIAL = (
    [18, 0, 17, 0, 0, 0, 1, 0, 2]  # 1筋
    + [19, 21, 17, 0, 0, 0, 1, 6, 3]  # 2筋。2bに後手角、2hに先手飛
    + [20, 0, 17, 0, 0, 0, 1, 0, 4]  # 3筋
    + [23, 0, 17, 0, 0, 0, 1, 0, 7]  # 4筋
    + [24, 0, 17, 0, 0, 0, 1, 0, 8]  # 5筋。玉
    + [23, 0, 17, 0, 0, 0, 1, 0, 7]  # 6筋
    + [20, 0, 17, 0, 0, 0, 1, 0, 4]  # 7筋
    + [19, 22, 17, 0, 0, 0, 1, 5, 3]  # 8筋。8bに後手飛、8hに先手角
    + [18, 0, 17, 0, 0, 0, 1, 0, 2]  # 9筋
)

RANK_INDEX = {c: i for i, c in enumerate("abcdefghi")}


def square(name):
    """「7g」のような表記をマス番号にする。"""
    return (int(name[0]) - 1) * 9 + RANK_INDEX[name[1]]


def move(pieces, frm, to, code=None):
    """盤を1手動かした盤を返す。成りは code で渡す。"""
    after = list(pieces)
    after[square(to)] = code if code is not None else after[square(frm)]
    after[square(frm)] = 0
    return after


def count(before, after):
    """粒度ごとの反転数を返す。"""
    return sf.flips(before, sf.attributes(before), after, sf.attributes(after))


def test_pawn_push_flips_nothing():
    """7g7fは、歩の短い利きの先が前後とも空マスなので反転が起きない。

    角道は開くが、影の利きは遮りを無視するので数は動かない。
    """
    after = move(INITIAL, "7g", "7f")
    assert count(INITIAL, after) == [0, 0, 0]


def test_gold_move_flips_hand_counted_pieces():
    """6i7hで属性が変わる駒を手で数える。

    金の利きは6iで7i（銀）と5i（玉）、7hで7g・8g・6g（歩）と8h（角）と
    7i（銀）を差す。7iは前後とも1つ差されたままなので数が動かない。

    - 5i玉: 自分の短い利きが2から1へ。3粒度とも変わる
    - 8g歩・6g歩: 自分の利きが0から1へ。3粒度とも変わる
    - 7g歩: 短い利きが1から2へ。合計は2から3で丸めると2のまま
    - 8h角: 短い利きが1から2へ。合計は2から3で丸めると2のまま

    よって both9 は3、split は5、own3 は3になる。
    """
    after = move(INITIAL, "6i", "7h")
    assert count(INITIAL, after) == [3, 5, 3]


def test_shadow_counts_ignore_blockers():
    """初期局面の1aは、遮られていても先手の角と香が影の利きで差す。"""
    b_short, b_shadow, w_short, w_shadow = sf.counts(INITIAL)
    # 8h角の斜めと、1i香の縦。どちらも間の駒を無視して1aへ届く
    assert b_shadow[square("1a")] == 2
    # 後手の飛車は8筋を8iまで差す
    assert w_shadow[square("8i")] == 1
    # 桂の利きは厳密なので、8i桂は7gと9gだけを差す
    assert b_short[square("7g")] == 1
    assert b_short[square("8g")] == 0


def test_successor_requires_next_ply_and_small_board_diff():
    after = move(INITIAL, "7g", "7f")
    assert sf.is_successor(INITIAL, 1, after, 2)
    # 手数が飛んでいる
    assert not sf.is_successor(INITIAL, 1, after, 3)
    # 盤が3マス以上違う（別の対局）
    other = move(after, "3c", "3d")
    assert not sf.is_successor(INITIAL, 1, other, 2)
    # 同じ盤（指し手がない）
    assert not sf.is_successor(INITIAL, 1, list(INITIAL), 2)


def test_board_layout_matches_cshogi():
    """盤の並びと駒コードがcshogiと一致することを確かめる。"""
    cshogi = pytest.importorskip("cshogi")
    board = cshogi.Board()
    assert list(board.pieces) == INITIAL
    assert (cshogi.BPAWN, cshogi.BKING, cshogi.WPROM_ROOK) == (
        sf.PAWN,
        sf.KING,
        sf.DRAGON + sf.WHITE,
    )
    board.push_usi("6i7h")
    assert list(board.pieces) == move(INITIAL, "6i", "7h")
