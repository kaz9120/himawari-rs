"""取り込む系のsubprocess呼び出しを検証する（issue #712）。

`proc.capture_code` が唯一の実装で、`capture`・`capture_both`・`succeeds` と
3つの呼び出し元（`config.physical_cores`・`status._launchd`・`queue.gh`）が
これを細めたものになっていることを見る。外部コマンドは1つだけ実際に走らせ、
残りは `capture_code` を差し替えて組み立てだけを見る。
"""

import sys

import pytest

from hmwr import config, paths, proc
from hmwr.commands import queue, status


def test_capture_code_returns_the_code_and_both_streams():
    code, out, err = proc.capture_code(
        [sys.executable, "-c", "import sys; sys.stdout.write('o'); sys.stderr.write('e'); sys.exit(7)"]
    )
    assert (code, out, err) == (7, "o", "e")


def test_capture_code_runs_in_the_repository_by_default():
    out = proc.capture_code([sys.executable, "-c", "import os; print(os.getcwd())"])[1]
    assert out.strip() == str(paths.REPO)


def test_capture_code_takes_a_missing_command_as_a_runtime_failure():
    code, out, err = proc.capture_code(["himawari-does-not-exist"])
    assert (code, out) == (proc.RUNTIME, "")
    assert err  # 原因を捨てない


def test_the_narrower_helpers_follow_capture_code(monkeypatch):
    monkeypatch.setattr(proc, "capture_code", lambda argv, cwd=None: (3, "o", "e"))
    assert proc.capture(["x"]) == "o"
    assert proc.capture_both(["x"]) == ("o", "e")
    assert proc.succeeds(["x"]) is False
    monkeypatch.setattr(proc, "capture_code", lambda argv, cwd=None: (0, "o", ""))
    assert proc.succeeds(["x"]) is True


@pytest.fixture
def cores(monkeypatch):
    """物理コア数の問い合わせを差し替える。lru_cacheは毎回捨てる。"""
    calls: list[list[str]] = []

    def fake(reply):
        def capture_code(argv, cwd=None):
            calls.append(argv)
            return reply
        config.physical_cores.cache_clear()
        monkeypatch.setattr(proc, "capture_code", capture_code)
        return calls

    yield fake
    config.physical_cores.cache_clear()


def test_physical_cores_reads_lscpu_on_linux(monkeypatch, cores):
    monkeypatch.setattr(config.platform, "system", lambda: "Linux")
    calls = cores((0, "# 注記\n0,0\n0,0\n1,0\n", ""))
    assert config.physical_cores() == 2
    assert calls == [["lscpu", "-p=Core,Socket"]]


def test_physical_cores_reads_sysctl_on_macos(monkeypatch, cores):
    monkeypatch.setattr(config.platform, "system", lambda: "Darwin")
    calls = cores((0, " 10 \n", ""))
    assert config.physical_cores() == 10
    assert calls == [["sysctl", "-n", "hw.perflevel0.physicalcpu"]]


def test_physical_cores_falls_back_when_the_command_is_missing(monkeypatch, cores):
    monkeypatch.setattr(config.platform, "system", lambda: "Linux")
    monkeypatch.setattr(config.os, "cpu_count", lambda: 6)
    cores((proc.RUNTIME, "", "そんなコマンドは無い"))
    assert config.physical_cores() == 6


def test_launchd_reads_the_pid_when_the_agent_is_loaded(monkeypatch):
    monkeypatch.setattr(proc, "capture_code", lambda argv, cwd=None: (0, '\t"PID" = 4321;\n', ""))
    assert status._launchd() == {"loaded": True, "pid": 4321}


@pytest.mark.parametrize("code", [1, proc.RUNTIME])
def test_launchd_reports_not_loaded_on_any_failure(monkeypatch, code):
    monkeypatch.setattr(proc, "capture_code", lambda argv, cwd=None: (code, "", "だめ"))
    assert status._launchd() == {"loaded": False}


def test_gh_returns_stdout_and_raises_with_stderr(monkeypatch):
    monkeypatch.setattr(proc, "capture_code", lambda argv, cwd=None: (0, "[]\n", ""))
    assert queue.gh("issue", "list") == "[]\n"
    monkeypatch.setattr(proc, "capture_code", lambda argv, cwd=None: (1, "", "届かない\n"))
    with pytest.raises(proc.Fail) as e:
        queue.gh("issue", "list")
    assert "gh issue list" in str(e.value) and "届かない" in str(e.value)


def test_capture_code_does_not_throw_on_broken_bytes():
    """出力の文字が壊れていても投げない。`succeeds` も壊れた出力で落ちない。"""
    argv = [sys.executable, "-c", "import sys; sys.stdout.buffer.write(b'\\xff')"]
    code, out, _ = proc.capture_code(argv)
    assert code == 0 and out == "�"
    assert proc.succeeds(argv) is True
