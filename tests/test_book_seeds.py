"""定跡の種の数え方。"""

import pytest

pytest.importorskip("cshogi")

from hmwr.tools import book_seeds  # noqa: E402

# 空き升は " * " の3文字で、行末の空白も要る（cshogiは欠けると落ちる）。
# エディタに消されないよう、行末を文字列の中に閉じ込める
EMPTY = " * " * 9
BOARD = "\n".join([
    "P1-KY-KE-GI-KI-OU-KI-GI-KE-KY",
    "P2 * -HI" + " * " * 5 + "-KA * ",
    "P3" + "-FU" * 9,
    "P4" + EMPTY,
    "P5" + EMPTY,
    "P6" + EMPTY,
    "P7" + "+FU" * 9,
    "P8 * +KA" + " * " * 5 + "+HI * ",
    "P9+KY+KE+GI+KI+OU+KI+GI+KE+KY",
    "+",
    "",
])


def write_game(path, black_rate, white_rate, moves):
    body = "V2\nN+a\nN-b\n" + BOARD + "".join(f"{m}\nT1\n" for m in moves)
    body += f"'black_rate:a+0:{black_rate}\n'white_rate:b+0:{white_rate}\n%TORYO\n"
    path.write_text(body, encoding="utf-8")


def test_game_keys_counts_each_position_once_up_to_max_ply(tmp_path):
    f = tmp_path / "g.csa"
    write_game(f, 3600, 3700, ["+2726FU", "-8384FU", "+2625FU", "-8485FU"])
    keys = book_seeds.game_keys((str(f), 2, 3500, 4))
    assert [ply for _, ply in keys] == [0, 1, 2]
    assert keys[0][0] == "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b -"
    assert keys[1][0].endswith(" w -")


def test_game_keys_skips_weak_or_short_games(tmp_path):
    f = tmp_path / "g.csa"
    write_game(f, 3400, 3700, ["+2726FU", "-8384FU"])
    assert book_seeds.game_keys((str(f), 40, 3500, 2)) is None, "片方のレートが下限未満"
    write_game(f, 3600, 3700, ["+2726FU", "-8384FU"])
    assert book_seeds.game_keys((str(f), 40, 3500, 60)) is None, "手数が足りない"
