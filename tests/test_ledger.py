"""点数を付けた局面の台帳（ADR-0228）を検証する。局面の中身は読まないので、
乱数のバイト列を局面に見立てる。"""

import json

import numpy as np
import pytest

from hmwr import paths, proc
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


def test_the_hash_is_pinned_to_its_scheme_name():
    # board_hash を変えると既存の台帳の区分が黙って無効になる。値が変わったら
    # ledger.HASH の名前も変え、台帳を作り直す
    r = np.arange(80, dtype=np.uint8).reshape(2, 40)
    assert [int(x) for x in ledger.board_hash(r)] == [0x5524C51FC9ED42C2, 0xB6EF7295EFF6ED87]
    assert ledger.HASH == "splitmix64-packed32"


def test_a_new_ledger_gets_its_mark(tmp_path):
    root = tmp_path / "ledger"
    ledger.add(root, [_psv(tmp_path / "a.psv", np.zeros((3, 40), np.uint8))])
    meta = json.loads((root / ledger.META).read_text())
    assert meta == {"schema": 1, "hash": ledger.HASH, "shards": ledger.SHARDS}


def test_a_ledger_without_a_mark_is_read_as_version_1_and_left_alone(tmp_path):
    root = tmp_path / "ledger"
    rng = np.random.default_rng(3)
    src = _psv(tmp_path / "a.psv", rng.integers(0, 256, (10, 40), dtype=np.uint8))
    ledger.add(root, [src])
    (root / ledger.META).unlink()  # 印を入れる前に作った台帳
    assert ledger.add(root, [src])["new"] == 0
    assert ledger.filter_new(root, src, tmp_path / "out.psv")["known"] == 10
    assert not (root / ledger.META).exists()


def test_a_ledger_with_another_hash_is_refused(tmp_path):
    root = tmp_path / "ledger"
    root.mkdir()
    (root / ledger.META).write_text(json.dumps({"schema": 1, "hash": "other", "shards": 256}))
    src = _psv(tmp_path / "a.psv", np.zeros((1, 40), np.uint8))
    with pytest.raises(proc.Fail, match="台帳の印"):
        ledger.filter_new(root, src, tmp_path / "out.psv")
    with pytest.raises(proc.Fail, match="台帳の印"):
        ledger.add(root, [src])


def test_the_log_keeps_sources_relative_to_the_repository():
    src = paths.TRAIN / "a.psv"
    src.parent.mkdir(parents=True)
    ledger.add(paths.LEDGER, [_psv(src, np.zeros((1, 40), np.uint8))])
    line = json.loads((paths.LEDGER / "added.jsonl").read_text().splitlines()[-1])
    assert line["sources"] == ["data/train/a.psv"]
