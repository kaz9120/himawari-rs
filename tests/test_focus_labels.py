"""焦点の熱地図つき局面集の切り出し（ADR-0213）を検証する。

見るのは3つ。盤面の差から手を取れること、熱地図と関与フラグが手順どおりに
付くこと、対局の切れ目をまたがないことである。盤は手で組んだものと、
`cshogi` で短い対局を進めたものの両方で確かめる。cshogiはCIに入れていないので、
そちらの検査は入っている環境でだけ走る。
"""

import json
import struct

import pytest

from hmwr import cli, dataops, paths, proc
from hmwr.tools import focus_labels as fl

# 手で組む盤の駒。値そのものに意味はなく、0でなければよい
BLACK_PAWN = 1
WHITE_PAWN = 17


def square(name: str) -> int:
    """「7g」のような表記をマスの番号にする。cshogiの並びに合わせる。"""
    return (int(name[0]) - 1) * 9 + (ord(name[1]) - ord("a"))


def name_of(sq: int) -> str:
    return f"{sq // 9 + 1}{chr(ord('a') + sq % 9)}"


def board(*pieces: str) -> bytes:
    """マス名を並べて盤を組む。頭にwを付けると後手の駒になる。

    駒の種類はラベルに関わらないので歩だけを置く。取られたマスは、駒の
    持ち主が入れ替わることで盤の差に出る。
    """
    cells = bytearray(fl.SQUARES)
    for name in pieces:
        cells[square(name.lstrip("w"))] = WHITE_PAWN if name.startswith("w") else BLACK_PAWN
    return bytes(cells)


def position(ply: int, turn: int, cells: bytes, tag: int = 0) -> fl.Position:
    record = bytes([tag] * 32) + struct.pack("<hHHbB", 0, 0, ply, 0, 0)
    return fl.Position(record, ply, cells, turn)


# --- 盤面の差から手を取る ----------------------------------------------


def test_move_is_read_from_the_two_changed_squares():
    got = fl.board_move(board("7g"), board("7f"))
    assert got == fl.Move(square("7g"), square("7f"), False)


def test_capture_is_the_destination_that_was_occupied():
    got = fl.board_move(board("8h", "w2b"), board("2b"))
    assert got == fl.Move(square("8h"), square("2b"), True)


def test_a_drop_changes_one_square_and_has_no_origin():
    assert fl.board_move(board(), board("5e")) == fl.Move(None, square("5e"), False)


def test_a_difference_that_is_not_a_move_is_refused():
    # 同じ盤、駒が消える、3マス動くのいずれも手の形にならない
    assert fl.board_move(board("7g"), board("7g")) is None
    assert fl.board_move(board("7g"), board()) is None
    assert fl.board_move(board("7g", "3g"), board("7f", "3g", "5e")) is None


def test_transition_needs_the_ply_to_continue_and_the_turn_to_flip():
    before = position(10, 0, board("7g"))
    assert fl.transition(before, position(11, 1, board("7f"))) is not None
    assert fl.transition(before, position(12, 1, board("7f"))) is None
    assert fl.transition(before, position(11, 0, board("7f"))) is None


# --- ラベル ------------------------------------------------------------


def hot(flags: bytes) -> set[str]:
    """立っているマスを名前の集合にする。"""
    return {name_of(sq) for sq, on in enumerate(flags) if on}


def test_labels_mark_the_origin_and_the_destination():
    moves = [fl.Move(square("7g"), square("7f"), False), fl.Move(square("3c"), square("3d"), False)]
    heat, involved = fl.labels(moves)
    assert hot(heat) == {"7g", "7f", "3c", "3d"}
    assert hot(involved) == {"7g", "3c"}


def test_labels_follow_a_piece_that_moves_twice():
    """2手続けて動いた駒の関与は、先頭の局面のマスへ付く。"""
    moves = [fl.Move(square("7g"), square("7f"), False), fl.Move(square("7f"), square("7e"), False)]
    heat, involved = fl.labels(moves)
    assert hot(heat) == {"7g", "7f", "7e"}
    assert hot(involved) == {"7g"}


def test_labels_mark_the_piece_that_is_taken():
    moves = [fl.Move(square("8h"), square("2b"), True), fl.Move(square("3a"), square("2b"), True)]
    heat, involved = fl.labels(moves)
    assert hot(heat) == {"8h", "2b", "3a"}
    # 2bで取られた駒と、2bへ動いてから取られた8hの駒の両方が関与する
    assert hot(involved) == {"2b", "8h", "3a"}


def test_labels_do_not_credit_a_dropped_piece_to_the_square_it_lands_on():
    moves = [fl.Move(None, square("5e"), False), fl.Move(square("4d"), square("5e"), True)]
    heat, involved = fl.labels(moves)
    assert hot(heat) == {"5e", "4d"}
    # 5eの駒は先頭の局面にいなかったので、関与に数えない
    assert hot(involved) == {"4d"}


# --- 窓と対局の切れ目 --------------------------------------------------


def walk(plies: int, *squares: str) -> list[fl.Position]:
    """1つの歩を順に動かすだけの対局を作る。"""
    out = []
    for i, name in enumerate(squares):
        cells = bytearray(fl.SQUARES)
        cells[square(name)] = BLACK_PAWN
        out.append(position(plies + i, i % 2, bytes(cells)))
    return out


def test_only_positions_with_a_full_continuation_are_kept():
    rows = list(fl.iter_focus(walk(1, "7g", "7f", "7e", "7d", "7c"), plies=2))
    assert len(rows) == 3
    assert len(rows[0]) == fl.RECORD_BYTES
    heat = rows[0][fl.PSV_BYTES : fl.PSV_BYTES + fl.SQUARES]
    assert hot(heat) == {"7g", "7f", "7e"}


def test_the_window_does_not_cross_a_game_boundary():
    first = walk(1, "7g", "7f", "7e")
    second = walk(1, "3c", "3d", "3e")  # 手数が1へ戻る＝別の対局
    rows = list(fl.iter_focus(first + second, plies=2))
    assert len(rows) == 2
    assert [struct.unpack_from("<H", row, fl.PLY_OFFSET)[0] for row in rows] == [1, 1]
    assert hot(rows[1][fl.PSV_BYTES : fl.PSV_BYTES + fl.SQUARES]) == {"3c", "3d", "3e"}


def test_a_game_shorter_than_the_window_yields_nothing():
    assert list(fl.iter_focus(walk(1, "7g", "7f"), plies=8)) == []


# --- 書き出しと完了印 --------------------------------------------------


@pytest.fixture
def fake(tmp_path, monkeypatch):
    """生データの読み取りを、手で組んだ対局に差し替える。"""
    raw = tmp_path / "raw" / "toy"
    raw.mkdir(parents=True)
    (raw / "a.bin").write_bytes(b"")
    train = tmp_path / "train"
    train.mkdir()
    monkeypatch.setattr(paths, "RAW", tmp_path / "raw")
    monkeypatch.setattr(paths, "TRAIN", train)
    monkeypatch.setattr(paths, "LOGS", tmp_path / "logs")
    (tmp_path / "logs").mkdir()
    monkeypatch.setattr(
        fl, "positions", lambda path: walk(1, "7g", "7f", "7e", "7d", "7c")
    )
    return train


def test_focus_writes_fixed_length_records_and_a_done_mark(fake, capsys):
    argv = ["data", "focus", "t", "--raw", "toy", "--count", "3", "--plies", "2"]
    assert cli.main(argv) == proc.OK
    out = fake / "t.focus"
    assert out.stat().st_size == 3 * fl.RECORD_BYTES
    done = json.loads((fake / "t.focus.done").read_text())
    assert done["commands"] == ["focus --raw toy --count 3 --plies 2"]
    assert done["stats"]["rows"] == 3
    assert done["stats"]["heat_mean"] == 3.0
    assert done["stats"]["involved_mean"] == 1.0
    assert "マスごとの熱地図の頻度" in capsys.readouterr().out

    # 同じ条件なら何もしない。条件が違えば止まる
    assert cli.main(argv) == proc.OK
    assert "済み" in capsys.readouterr().out
    assert cli.main(["data", "focus", "t", "--raw", "toy", "--count", "2"]) == proc.RUNTIME


def test_focus_fails_when_the_count_is_short(fake):
    assert cli.main(["data", "focus", "t", "--raw", "toy", "--count", "9", "--plies", "2"]) == proc.RUNTIME
    assert not (fake / "t.focus").exists()


def test_focus_needs_raw_data(fake):
    assert cli.main(["data", "focus", "t", "--raw", "nope", "--count", "1"]) == proc.RUNTIME


def test_focus_dry_run_only_shows_the_input_and_the_count(capsys):
    assert cli.main(["--dry-run", "data", "focus", "t", "--raw", "toy", "--count", "5"]) == proc.OK
    lines = [x for x in capsys.readouterr().out.splitlines() if x.startswith("[dry-run]")]
    assert lines[0] == "[dry-run] focus --raw toy --count 5 --plies 8 → data/train/t.focus.part"
    assert lines[1].startswith("[dry-run] 入力: data/raw/toy/*.bin")


def test_rm_removes_a_focus_output(fake):
    assert cli.main(["data", "focus", "t", "--raw", "toy", "--count", "3", "--plies", "2"]) == proc.OK
    assert cli.main(["data", "rm", "t"]) == proc.OK
    assert not (fake / "t.focus").exists() and not (fake / "t.focus.done").exists()


# --- cshogiで進めた対局 ------------------------------------------------


def toy_game(moves):
    """初期局面から手順を進め、psvのレコードを並べた生データを作る。"""
    cshogi = pytest.importorskip("cshogi")
    np = pytest.importorskip("numpy")

    board = cshogi.Board()
    out = bytearray()
    buffer = np.empty(fl.SFEN_BYTES, dtype=np.uint8)
    for ply, usi in enumerate([*moves, None], start=1):
        board.to_psfen(buffer)
        out += buffer.tobytes() + struct.pack("<hHHbB", 0, 0, ply, 0, 0)
        if usi is not None:
            board.push_usi(usi)
    return bytes(out)


def test_a_real_game_gives_the_expected_heat_map(tmp_path):
    """7g7f 3c3d 8h2b+ 3a2bの4手を、初期局面から見たラベルにする。"""
    raw = tmp_path / "a.bin"
    raw.write_bytes(toy_game(["7g7f", "3c3d", "8h2b+", "3a2b"]))

    rows = list(fl.iter_focus(fl.positions(raw), plies=4))
    assert len(rows) == 1
    heat = rows[0][fl.PSV_BYTES : fl.PSV_BYTES + fl.SQUARES]
    involved = rows[0][fl.PSV_BYTES + fl.SQUARES :]
    assert hot(heat) == {"7g", "7f", "3c", "3d", "8h", "2b", "3a"}
    # 2bの角は取られ、8hの角は動いた先で取られる
    assert hot(involved) == {"7g", "3c", "8h", "2b", "3a"}


def test_a_real_game_keeps_the_original_psv_record(tmp_path):
    raw = tmp_path / "a.bin"
    raw.write_bytes(toy_game(["7g7f", "3c3d", "2g2f", "8c8d"]))
    rows = list(fl.iter_focus(fl.positions(raw), plies=2))

    assert len(rows) == 3
    assert rows[0][: fl.PSV_BYTES] == raw.read_bytes()[: fl.PSV_BYTES]
    assert hot(rows[0][fl.PSV_BYTES : fl.PSV_BYTES + fl.SQUARES]) == {"7g", "7f", "3c", "3d"}
    assert hot(rows[2][fl.PSV_BYTES : fl.PSV_BYTES + fl.SQUARES]) == {"2g", "2f", "8c", "8d"}
