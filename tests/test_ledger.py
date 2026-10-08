"""点数を付けた局面の台帳（ADR-0228）を検証する。局面の中身は読まないので、
乱数のバイト列を局面に見立てる。"""

import numpy as np

from hmwr.tools import ledger


def _psv(path, rows):
    np.asarray(rows, np.uint8).tofile(path)
    return path


def test_add_is_idempotent_and_filter_keeps_only_new_first_occurrences(tmp_path):
    rng = np.random.default_rng(0)
    a = rng.integers(0, 256, (1000, 40), dtype=np.uint8)
    b = rng.integers(0, 256, (500, 40), dtype=np.uint8)
    root = tmp_path / "ledger"
    src_a = _psv(tmp_path / "a.psv", a)
    st = ledger.add(root, [src_a])
    assert st == {"read": 1000, "new": 1000, "size": 1000}
    assert ledger.add(root, [src_a])["new"] == 0  # 二度足しても増えない

    # 入力: 新しい局面b、台帳にあるa[:100]、b[:50]の重複（盤面だけ同じで評価値が違う）
    dup = b[:50].copy()
    dup[:, 32:40] = 7
    mixed = np.concatenate([b, a[:100], dup])
    src = _psv(tmp_path / "in.psv", mixed)
    out = tmp_path / "out.psv"
    st = ledger.filter_new(root, src, out)
    got = np.fromfile(out, np.uint8).reshape(-1, 40)
    assert np.array_equal(got, b)  # 初出だけが、入力の順に残る
    assert st["known"] == 100 and st["dup_in_input"] == 50


def test_hash_depends_only_on_the_board_bytes():
    rng = np.random.default_rng(1)
    r = rng.integers(0, 256, (2, 40), dtype=np.uint8)
    r[1, :32] = r[0, :32]
    h = ledger.board_hash(r)
    assert h[0] == h[1]
    r[1, 5] ^= 1
    assert ledger.board_hash(r)[0] != ledger.board_hash(r)[1]


def test_add_works_under_a_low_open_file_limit(tmp_path):
    import resource

    soft, hard = resource.getrlimit(resource.RLIMIT_NOFILE)
    resource.setrlimit(resource.RLIMIT_NOFILE, (64, hard))  # 区分の数（256）より少ない
    try:
        rng = np.random.default_rng(2)
        src = _psv(tmp_path / "a.psv", rng.integers(0, 256, (3000, 40), dtype=np.uint8))
        assert ledger.add(tmp_path / "ledger", [src])["new"] == 3000
    finally:
        resource.setrlimit(resource.RLIMIT_NOFILE, (soft, hard))
