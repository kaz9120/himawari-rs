"""対局の再現の、CSAからUSIへの変換を検証する。エンジンは走らせない。"""

from hmwr.tools import replay

CSA = """N+Aoba
N-Himawari
+7776FU
T3
-3334FU
T5
+8822UM
T7
-3122GI
T2
+0055KA
T9
"""


def test_csa_moves_become_usi_with_promotions_and_drops():
    moves, seconds, white = replay.csa_to_usi(CSA)
    assert moves == ["7g7f", "3c3d", "8h2b+", "3a2b", "B*5e"]
    assert seconds == [3, 5, 7, 2, 9]
    assert white == "Himawari"


def test_a_move_of_an_already_promoted_piece_is_not_a_promotion():
    text = CSA + "-2233UM\nT1\n"
    moves, _, _ = replay.csa_to_usi(text)
    assert moves[-1] == "2b3c"
