"""FT出力次元の並べ替えを検証する（ADR-0168）。"""

import pytest

from hmwr.tools import ft_reorder


def write_dump(tmp_path, samples):
    """1サンプル=バイト列のリストからダンプを作る。"""
    path = tmp_path / "act.bin"
    path.write_bytes(b"".join(samples))
    return str(path)


def test_load_masks_reads_bit_per_dimension(tmp_path):
    # FT_OUT=8、CONCAT=16。0b10101010 は次元0,2,4,6がゼロ
    path = write_dump(tmp_path, [b"\xaa\xaa"] * 4)
    z0, z1, n = ft_reorder.load_masks(path, 8)
    assert n == 4
    assert [z.bit_count() for z in z0] == [4, 0, 4, 0, 4, 0, 4, 0]
    assert [z.bit_count() for z in z1] == [4, 0, 4, 0, 4, 0, 4, 0]


def test_greedy_gathers_zeros_into_chunks(tmp_path):
    """1つおきにゼロが並ぶと、そのままでは全ゼロチャンクが作れない。"""
    path = write_dump(tmp_path, [b"\xaa\xaa"] * 4)
    z0, z1, n = ft_reorder.load_masks(path, 8)

    rate, running = ft_reorder.chunk_stats(z0, z1, list(range(8)), n)
    assert rate == 0.0
    assert running == 4

    perm = ft_reorder.greedy_permutation(z0, z1)
    assert sorted(perm) == list(range(8))
    rate, running = ft_reorder.chunk_stats(z0, z1, perm, n)
    assert rate == 0.5
    assert running == 2
    # ゼロの次元だけが1つのチャンクへ集まる
    assert sorted(perm[:4]) == [0, 2, 4, 6]


def test_stats_do_not_change_when_zeros_already_line_up(tmp_path):
    """次元0〜3が常にゼロなら、並べ替えても増えない。"""
    path = write_dump(tmp_path, [b"\xf0\xf0"] * 4)
    z0, z1, n = ft_reorder.load_masks(path, 8)
    before, _ = ft_reorder.chunk_stats(z0, z1, list(range(8)), n)
    perm = ft_reorder.greedy_permutation(z0, z1)
    after, _ = ft_reorder.chunk_stats(z0, z1, perm, n)
    assert before == after == 0.5


def test_load_masks_rejects_a_short_dump(tmp_path):
    path = write_dump(tmp_path, [b"\xaa"])
    with pytest.raises(ValueError, match="1サンプル"):
        ft_reorder.load_masks(path, 8)


def test_read_perm_rejects_a_non_permutation(tmp_path):
    path = tmp_path / "perm.txt"
    path.write_text("0 1 2 2\n")
    with pytest.raises(ValueError, match="順列"):
        ft_reorder.read_perm(str(path), 4)


def test_ft_out_must_be_a_multiple_of_four(tmp_path, monkeypatch, capsys):
    path = write_dump(tmp_path, [b"\xaa\xaa"])
    monkeypatch.setattr("sys.argv", ["ft-reorder.py", path, "6"])
    assert ft_reorder.main() == 2
    assert "4の倍数" in capsys.readouterr().err
