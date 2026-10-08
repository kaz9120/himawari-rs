"""シャッフル済みの教師の織り交ぜ（ADR-0228）を検証する。"""

import numpy as np

from hmwr.tools import merge


def _rows(tag, n):
    r = np.zeros((n, 40), np.uint8)
    r[:, 0] = tag
    r[:, 1:5] = np.arange(n, dtype=np.uint32).view(np.uint8).reshape(n, 4)
    return r


def test_merge_is_a_permutation_of_the_union_and_consumes_inputs(tmp_path, monkeypatch):
    monkeypatch.setattr(merge, "BLOCK", 1000)  # 末尾から何度も読み直す経路を通す
    srcs = []
    for tag, n in ((1, 7000), (2, 2500), (3, 333)):
        p = tmp_path / f"{tag}.psv"
        _rows(tag, n).tofile(p)
        srcs.append(p)
    out = tmp_path / "out.psv"
    st = merge.merge(srcs, out, seed=1, consume=True)
    got = np.fromfile(out, np.uint8).reshape(-1, 40)
    assert st["kept"] == 9833 and len(got) == 9833
    for tag, n in ((1, 7000), (2, 2500), (3, 333)):
        ids = np.sort(got[got[:, 0] == tag][:, 1:5].copy().view(np.uint32).ravel())
        assert np.array_equal(ids, np.arange(n))
    assert not any(p.exists() for p in srcs)
    # 由来は前半と後半で偏らない（一様に織り交ぜている）
    half = len(got) // 2
    share = [np.mean(got[:half, 0] == 1), np.mean(got[half:, 0] == 1)]
    assert abs(share[0] - share[1]) < 0.05


def test_merge_without_consume_keeps_inputs_and_is_deterministic(tmp_path):
    srcs = []
    for tag, n in ((1, 500), (2, 300)):
        p = tmp_path / f"{tag}.psv"
        _rows(tag, n).tofile(p)
        srcs.append(p)
    a, b = tmp_path / "a.psv", tmp_path / "b.psv"
    merge.merge(srcs, a, seed=3, consume=False)
    merge.merge(srcs, b, seed=3, consume=False)
    assert a.read_bytes() == b.read_bytes()
    assert all(p.exists() for p in srcs)
