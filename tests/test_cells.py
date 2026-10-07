"""区画の配分とαでの選択（ADR-0227）の純粋な部分を検証する。"""

import numpy as np
import pytest

from hmwr.tools import cells


def test_targets_follow_the_ratios_when_stock_is_enough():
    inv = np.full((3, 3), 1e9)
    t = cells.targets(inv, [0.15, 0.25, 0.60], [0.25, 0.25, 0.50], 100)
    assert t.sum() == pytest.approx(100)
    assert t[0, 0] == pytest.approx(100 * 0.15 * 0.25)


def test_a_short_cell_takes_everything_and_moves_the_rest_to_advantage():
    inv = np.full((3, 3), 1e9)
    inv[0, 0] = 1.0  # 敵陣×互角がほとんど無い
    t = cells.targets(inv, [0.15, 0.25, 0.60], [0.25, 0.25, 0.50], 100)
    assert t[0, 0] == 1.0
    # 不足は同じ玉の帯の優勢へ回り、合計は変わらない
    assert t[0, 1] == pytest.approx(100 * 0.15 * 0.25 + (100 * 0.15 * 0.25 - 1))
    assert t.sum() == pytest.approx(100)


def test_ratios_must_sum_to_one():
    with pytest.raises(ValueError):
        cells.targets(np.ones((3, 3)), [0.5, 0.5, 0.5], [0.25, 0.25, 0.5], 10)


def test_alpha_moves_the_selection_toward_rare_positions_smoothly():
    rng = np.random.default_rng(0)
    logt = rng.normal(0, 1, 100_000)
    w = np.ones_like(logt)
    tails = []
    for a in (0.0, 0.5, 1.0):
        c = cells.solve_scale(logt, w, 10_000, a)
        pi = np.minimum(1, c * np.exp(-a * (logt - logt.max())))
        assert pi.sum() == pytest.approx(10_000, rel=1e-3)
        tails.append(pi[logt < np.quantile(logt, 0.05)].sum() / pi.sum())
    assert tails[0] == pytest.approx(0.05, abs=0.01)
    assert tails[0] < tails[1] < tails[2]


def test_bands():
    assert cells.king_band(np.array([1, 3, 4, 6, 7, 9])).tolist() == [0, 0, 1, 1, 2, 2]
    assert cells.eval_band(np.array([0, 300, 1000, -1000]), 430).tolist() == [0, 1, 2, 2]


def test_decode_reads_the_start_position_from_the_side_to_move():
    cshogi = pytest.importorskip("cshogi")
    b = cshogi.Board()
    rec = np.zeros((2, 40), np.uint8)
    b.to_psfen(rec[0, :32])
    b.push_usi("7g7f")  # 後手番にする
    b.to_psfen(rec[1, :32])
    rank, flat, lens = cells.decode(rec)
    assert rank.tolist() == [9, 9]  # どちらの手番から見ても玉は自陣の最下段
    assert lens.tolist() == [38, 38]  # 玉を除く盤上38枚、持ち駒なし
