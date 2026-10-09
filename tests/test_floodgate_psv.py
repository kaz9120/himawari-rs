"""floodgateの棋譜からの局面の取り出し（ADR-0228）を検証する。"""

import numpy as np
import pytest

from hmwr.tools import floodgate_psv as fp

cshogi = pytest.importorskip("cshogi")


def test_move16_round_trips_between_cshogi_and_yaneuraou():
    b = cshogi.Board()
    kinds = {}
    # 普通の手、成って取る手、取り返す手、持ち駒の角を打つ手
    for usi, kind in (("7g7f", "move"), ("3c3d", "move"), ("8h2b+", "prom"), ("3a2b", "move"), ("B*4e", "drop")):
        mv = b.move_from_usi(usi)
        assert b.is_legal(mv), usi
        c16 = cshogi.move16(mv)
        y = fp.to_yo_move16(c16)
        assert fp.from_yo_move16(y) == c16
        kinds[kind] = y
        b.push(mv)
    assert kinds["prom"] & (1 << 15) and not kinds["prom"] & (1 << 14)  # やねうら王の成り
    assert kinds["drop"] & (1 << 14) and not kinds["drop"] & (1 << 15)  # やねうら王の打つ印
    assert (kinds["drop"] >> 7) & 0x7F == cshogi.BISHOP  # 移動元の欄は駒種


def test_a_game_becomes_one_row_per_ply_with_the_side_to_move_result(tmp_path):
    csa = tmp_path / "g.csa"
    moves = ["+7776FU", "-3334FU", "+8822UM", "-3122GI", "+0055KA"]
    csa.write_text("V2.2\nN+a\nN-b\nPI\n+\n" + "\n".join(moves) + "\n%TORYO\n", encoding="utf-8")
    rows = np.frombuffer(fp.game_records(str(csa), min_plies=1), np.uint8).reshape(-1, 40)
    assert len(rows) == 5
    ply = rows[:, 36].astype(int) | (rows[:, 37].astype(int) << 8)
    assert ply.tolist() == [1, 2, 3, 4, 5]
    # 投了したのは後手番（6手目）なので先手の勝ち。手番側から見た勝敗が交互に並ぶ
    assert [int(np.int8(x)) for x in rows[:, 38]] == [1, -1, 1, -1, 1]
    assert not rows[:, 32:34].any()
    assert fp.game_records(str(csa), min_plies=30) == b""
