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


def test_main_accepts_argv_list(tmp_path):
    """hmwr net reorderは引数リストでmainを呼ぶ（回帰。ADR-0185で発覚）。

    ダンプ形式の検証は他のテストが持つ。ここでは呼び出し規約だけを見る。
    引数が壊れていればargparseがSystemExit(2)を出し、呼べていれば
    ファイルなしのOSErrorを踏んで3が返る。
    """
    from hmwr.tools import ft_reorder
    missing = str(tmp_path / "no_such_dump.bin")
    assert ft_reorder.main([missing, "512"]) == 3


# --- ネットへ当てる（hmwr net reorder --apply） -------------------------


def _fake_apply(monkeypatch, tables, envs=None):
    """makenet・エンジンのビルド・verifyの3つを差し替える。

    envsを渡すと、呼び出しごとの環境変数をそこへ順に控える（#1025）。
    """
    from hmwr import proc
    from hmwr.commands import build, net

    calls = []

    def record(env):
        if envs is not None:
            envs.append(env or {})

    def fake_run(argv, **kw):
        calls.append(argv)
        record(kw.get("env"))
        return proc.OK

    outputs = iter(tables)

    def fake_capture(argv, **kw):
        record(kw.get("env"))
        return next(outputs)

    monkeypatch.setattr(proc, "run", fake_run)
    monkeypatch.setattr(build, "cargo_build", lambda **kw: record(kw.get("env")))
    monkeypatch.setattr(proc, "capture", fake_capture)
    return net, calls


TABLE = "| 局面 | ノード数 | 評価値 | 最善手 |\n|---|---|---|---|\n| 1 | 100 | cp 5 | 7g7f |\n"


def test_apply_writes_a_reorder_net_and_checks_the_search_is_unchanged(monkeypatch, tmp_path):
    from hmwr import paths, proc

    net, calls = _fake_apply(monkeypatch, [TABLE, TABLE])
    src = paths.NETS / "x.hmwr"
    assert net.apply_reorder(src, paths.PROFILE / "perm_x.txt") == proc.OK
    (makenet,) = calls
    assert makenet[makenet.index("--out") + 1] == paths.rel(paths.NETS / "x_reorder.hmwr")
    # 来歴に残るので、置換は相対パスで渡す
    assert not makenet[makenet.index("--reorder") + 1].startswith("/")


def test_apply_refuses_when_the_search_changes(monkeypatch, tmp_path):
    import pytest

    from hmwr import paths, proc

    net, _ = _fake_apply(monkeypatch, [TABLE, TABLE.replace("100", "101")])
    with pytest.raises(proc.Fail):
        net.apply_reorder(paths.NETS / "x.hmwr", tmp_path / "perm.txt")


def test_apply_passes_the_arch_to_every_cargo_call(monkeypatch):
    """構成はmakenet・ビルド・verifyの全部へ届く（#1025）。

    1つでも欠けると、その呼び出しが既定の構成で target/release を
    置き換え、読み込みが effect accumulator mismatch で落ちる。
    """
    from hmwr import paths, proc

    envs = []
    net, _ = _fake_apply(monkeypatch, [TABLE, TABLE], envs)
    spec = "256x32x32e8"
    code = net.apply_reorder(
        paths.NETS / "x.hmwr", paths.PROFILE / "perm_x.txt", env={"HIMAWARI_ARCH": spec}
    )
    assert code == proc.OK
    # makenet・ビルド・verify（2局面ぶん）の4回
    assert len(envs) == 4
    assert all(env.get("HIMAWARI_ARCH") == spec for env in envs)
    # makenetのRUSTFLAGSは構成を足しても残る
    assert "RUSTFLAGS" in envs[0]


def test_apply_without_an_arch_sets_no_arch_variable(monkeypatch):
    """構成を省いたときは環境変数を足さない。build.rsの既定が効く。"""
    from hmwr import paths, proc

    envs = []
    net, _ = _fake_apply(monkeypatch, [TABLE, TABLE], envs)
    assert net.apply_reorder(paths.NETS / "x.hmwr", paths.PROFILE / "perm_x.txt") == proc.OK
    assert all("HIMAWARI_ARCH" not in env for env in envs)


def test_reordered_name_drops_every_suffix():
    from hmwr import paths
    from hmwr.commands import net

    assert net.reordered_path(paths.NETS / "a_7860M.hmwr.best") == paths.NETS / "a_7860M_reorder.hmwr"
