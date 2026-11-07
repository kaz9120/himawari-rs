"""`hmwr net actdump` が局面ごとに送るgoと間引きを検証する（#653）。

エンジンは起動しない。ビルドと実行を差し替え、標準入力と環境変数だけを見る。
"""

import pytest

from hmwr import cli, paths, proc
from hmwr.commands import build, net


@pytest.fixture
def engine(tmp_path, monkeypatch):
    """エンジンの代わりに、渡された入力を控えてダンプを書くだけの関数を置く。"""
    calls = []
    builds = []
    monkeypatch.setattr(paths, "PROFILE", tmp_path / "profile")
    monkeypatch.setattr(paths, "LOGS", tmp_path / "logs")
    monkeypatch.setattr(build, "cargo_build", lambda *_, **kw: builds.append(kw.get("env") or {}))

    def fake_run(argv, *, env, stdin_text, **_):
        calls.append({"env": env, "stdin": stdin_text})
        with open(env["HIMAWARI_ACT_OUT"], "wb") as fh:
            fh.write(b"dump")
        return proc.OK

    monkeypatch.setattr(proc, "run", fake_run)
    positions = tmp_path / "positions.txt"
    positions.write_text("# コメント\nstartpos\n\nstartpos moves 7g7f\n")
    return calls, positions, builds


def run(positions, *extra):
    argv = ["net", "actdump", "t", "--positions", str(positions), "--eval-file", "e.hmwr"]
    return cli.main(argv + list(extra))


def go_lines(call):
    return [line for line in call["stdin"].splitlines() if line.startswith("go ")]


def test_depth_is_the_default(engine):
    calls, positions, _ = engine
    assert run(positions) == proc.OK
    assert go_lines(calls[0]) == ["go depth 16", "go depth 16"]
    assert calls[0]["env"]["HIMAWARI_ACT_STRIDE"] == str(net.actdump_stride(2, None))


def test_nodes_replaces_depth(engine):
    calls, positions, _ = engine
    assert run(positions, "--nodes", "1000000") == proc.OK
    assert go_lines(calls[0]) == ["go nodes 1000000", "go nodes 1000000"]
    # 間引きはノード数からの見込みで決まる。1局面32万回×2局面÷12,500
    assert calls[0]["env"]["HIMAWARI_ACT_STRIDE"] == "51"


def test_depth_and_nodes_are_exclusive(engine):
    _, positions, _ = engine
    with pytest.raises(SystemExit):
        run(positions, "--depth", "12", "--nodes", "1000")


def test_arch_reaches_the_build(engine):
    """--arch はダンプ用のビルドへ届く（#1025）。

    届かないと既定の構成で target/release を上書きし、既定以外の構成の
    ネットが effect accumulator mismatch で読めない。
    """
    _, positions, builds = engine
    assert run(positions, "--arch", "256x32x32e8") == proc.OK
    assert builds == [{"HIMAWARI_ARCH": "256x32x32e8"}]


def test_no_arch_leaves_the_build_untouched(engine):
    """構成を省いたときは環境変数を足さない。build.rsの既定が効く。"""
    _, positions, builds = engine
    assert run(positions) == proc.OK
    assert builds == [{}]


def test_a_malformed_arch_stops_before_building(engine):
    """構成の書き方が違えば、ビルドへ入る前に終了コード2で止まる。"""
    _, positions, builds = engine
    assert run(positions, "--arch", "256-32") == 2
    assert builds == []
