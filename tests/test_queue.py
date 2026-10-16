"""実験の待ち行列を検証する（ADR-0209）。

GitHubにも開発機にも触らない。`gh` の呼び出しと実験の実行を差し替え、
どのIssueを取り、ラベルをどう付け替えるかを見る。
"""

import plistlib
import re

import pytest

from hmwr import cli, paths, proc
from hmwr.commands import queue


def issue(number, name, author="owner", created="2026-09-18T00:00:00Z"):
    return {
        "number": number,
        "title": f"実験: {name}",
        "body": f"### spec\n\nexperiments/{name}.toml\n\n### ADR\n\ndocs/adr/0209-x.md",
        "author": {"login": author},
        "createdAt": created,
    }


@pytest.fixture
def world(tmp_path, monkeypatch):
    """Issueの一覧と、実験の終了コードを持つ小さな世界。"""
    state = {"queued": [], "running": [], "exit": proc.OK, "edits": [], "comments": [], "ran": []}
    monkeypatch.setattr(paths, "QUEUE", tmp_path / "queue")
    monkeypatch.setattr(paths, "LOGS", tmp_path / "logs")
    monkeypatch.setattr(queue, "issues", lambda label: list(state.get(label, [])))
    monkeypatch.setattr(queue, "owner", lambda: "owner")
    monkeypatch.setattr(
        queue, "relabel", lambda n, add, remove: state["edits"].append((n, add, remove))
    )
    monkeypatch.setattr(queue, "comment", lambda n, body: state["comments"].append((n, body)))
    monkeypatch.setattr(queue, "_progress", lambda name: f"{name}: 進み具合")
    monkeypatch.setattr(queue, "_build_if_moved", lambda: None)
    monkeypatch.setattr(queue, "_record", lambda name, number: state["recorded"].append(name))
    state["recorded"] = []

    class Result:
        def __init__(self, code):
            self.returncode = code

    def fake_subprocess(argv, **_):
        state["ran"].append(argv[-1])
        return Result(state["exit"])

    monkeypatch.setattr(queue.subprocess, "run", fake_subprocess)
    return state


def test_the_oldest_queued_experiment_runs_first(world):
    world["queued"] = [
        issue(12, "adr0212-b", created="2026-09-19T00:00:00Z"),
        issue(11, "adr0211-a", created="2026-09-18T00:00:00Z"),
    ]
    # issues() は古い順に返す約束。ここでは並べ替え済みのものを渡す
    world["queued"].sort(key=lambda i: i["createdAt"])
    assert cli.main(["queue", "tick"]) == proc.OK
    assert world["ran"] == ["adr0211-a"]
    assert world["edits"] == [(11, "running", "queued,failed"), (11, "done", "running")]
    assert "完了" in world["comments"][-1][1]
    assert world["recorded"] == ["adr0211-a"]


def test_an_interrupted_experiment_is_resumed_before_new_ones(world):
    world["running"] = [issue(10, "adr0210-x")]
    world["queued"] = [issue(11, "adr0211-a")]
    assert cli.main(["queue", "tick"]) == proc.OK
    assert world["ran"] == ["adr0210-x"]
    # 続きからなので、開始の付け替えとコメントはしない
    assert world["edits"] == [(10, "done", "running")]


def test_a_failure_is_written_back_and_the_queue_moves_on(world):
    world["queued"] = [issue(11, "adr0211-a")]
    world["exit"] = proc.RUNTIME
    assert cli.main(["queue", "tick"]) == proc.RUNTIME
    assert world["recorded"] == []
    assert world["edits"][-1] == (11, "failed", "running")
    assert "queued" in world["comments"][-1][1]


def test_only_the_owner_can_enqueue(world):
    """Issueフォームは誰が出してもラベルが付く。公開リポジトリなので作者を見る。"""
    world["queued"] = [issue(11, "adr0211-a", author="stranger")]
    assert cli.main(["queue", "tick"]) == proc.RUNTIME
    assert world["ran"] == []
    assert world["edits"] == [(11, "failed", "queued")]


def test_an_issue_without_a_spec_path_fails(world):
    broken = issue(11, "x")
    broken["body"] = "specを書き忘れた"
    world["queued"] = [broken]
    assert cli.main(["queue", "tick"]) == proc.RUNTIME
    assert world["ran"] == []


def test_pause_stops_the_queue_from_taking_work(world):
    world["queued"] = [issue(11, "adr0211-a")]
    assert cli.main(["queue", "pause"]) == proc.OK
    assert cli.main(["queue", "tick"]) == proc.OK
    assert world["ran"] == []
    assert cli.main(["queue", "resume"]) == proc.OK
    assert cli.main(["queue", "tick"]) == proc.OK
    assert world["ran"] == ["adr0211-a"]


def test_an_experiment_stopped_by_pause_stays_running(world, monkeypatch):
    world["queued"] = [issue(11, "adr0211-a")]
    world["exit"] = proc.JUDGE
    (paths.QUEUE).mkdir(parents=True)

    def pause_midway(argv, **_):
        queue.exp.pause_file().touch()
        world["ran"].append(argv[-1])

        class Result:
            returncode = proc.JUDGE

        return Result()

    monkeypatch.setattr(queue.subprocess, "run", pause_midway)
    assert cli.main(["queue", "tick"]) == proc.OK
    assert world["edits"] == [(11, "running", "queued,failed")]


def test_empty_queue_does_nothing(world):
    assert cli.main(["queue", "tick"]) == proc.OK
    assert world["ran"] == [] and world["edits"] == []


def test_a_quiet_tick_writes_nothing_when_idle(world, capsys):
    # launchdは5分おきに呼ぶ。何もしなかった回まで書くと、ログが9万行に膨らんだ
    assert cli.main(["queue", "tick", "--quiet"]) == proc.OK
    assert cli.main(["queue", "pause"]) == proc.OK
    capsys.readouterr()
    assert cli.main(["queue", "tick", "--quiet"]) == proc.OK
    assert capsys.readouterr().out == ""


def test_taking_an_experiment_is_logged_with_a_time(world, capsys):
    world["queued"] = [issue(11, "adr0211-a")]
    assert cli.main(["queue", "tick", "--quiet"]) == proc.OK
    first = capsys.readouterr().out.splitlines()[0]
    assert re.match(r"^\d{4}-\d\d-\d\d \d\d:\d\d:\d\d #11 ", first)


def test_a_large_launchd_log_is_set_aside_not_deleted(tmp_path, monkeypatch):
    monkeypatch.setattr(paths, "LOGS", tmp_path)
    monkeypatch.setattr(queue, "ROTATE_BYTES", 10)
    log = tmp_path / queue.LAUNCHD_LOG
    log.write_text("小さい")
    queue._rotate_log()
    assert log.read_text() == "小さい"
    log.write_text("x" * 11)
    queue._rotate_log()
    assert not log.exists()
    [kept] = tmp_path.glob("queue-launchd-*.log")
    assert kept.read_text() == "x" * 11


def test_spec_name_is_read_from_the_issue_form():
    assert queue.spec_name(issue(1, "adr0210-teacher-mix-control")["body"]) == (
        "adr0210-teacher-mix-control"
    )
    assert queue.spec_name("") is None


def test_run_refuses_a_working_tree_on_a_branch(monkeypatch):
    monkeypatch.setattr(queue, "_detached", lambda: False)
    assert cli.main(["queue", "run"]) == proc.RUNTIME


def test_the_agent_keeps_the_machine_awake_and_unthrottled(tmp_path):
    spec = queue.plist(tmp_path)
    assert spec["ProgramArguments"][:2] == ["/usr/bin/caffeinate", "-i"]
    assert spec["ProgramArguments"][-2:] == ["queue", "run"]
    assert spec["ProcessType"] == "Standard"
    assert spec["StartInterval"] == queue.INTERVAL
    plistlib.dumps(spec)


# --- 計測と対局の前の検査（issue #764） ---------------------------------


@pytest.fixture
def machine(tmp_path, monkeypatch):
    """常駐が入った開発機。data/queue があり、一時停止していない。"""
    import fcntl

    monkeypatch.setattr(paths, "QUEUE", tmp_path / "queue")
    paths.QUEUE.mkdir()
    plist = tmp_path / "agent.plist"
    plist.touch()
    monkeypatch.setattr(queue, "plist_path", lambda: plist)
    monkeypatch.delenv(queue.STEP_ENV, raising=False)

    class Machine:
        def pause(self):
            (paths.QUEUE / "PAUSE").touch()

        def hold_lock(self):
            fh = open(paths.QUEUE / "LOCK", "w")
            fcntl.flock(fh, fcntl.LOCK_EX | fcntl.LOCK_NB)
            return fh

    return Machine()


def test_measuring_needs_the_queue_paused(machine):
    with pytest.raises(proc.Fail) as e:
        queue.guard("SPRT", dry_run=False)
    assert e.value.code == proc.RUNTIME and "hmwr queue pause" in str(e.value)
    machine.pause()
    queue.guard("SPRT", dry_run=False)


def test_a_running_step_blocks_even_when_paused(machine):
    # 一時停止は走っているステップを止めない。ロックが離れるまで待たせる
    machine.pause()
    fh = machine.hold_lock()
    with pytest.raises(proc.Fail, match="実行中"):
        queue.guard("NPSの計測", dry_run=False)
    fh.close()
    queue.guard("NPSの計測", dry_run=False)


def test_the_queue_own_steps_and_dry_runs_pass(machine, monkeypatch, capsys):
    queue.guard("SPRT", dry_run=True)
    assert "本番なら止まる" in capsys.readouterr().out
    monkeypatch.setenv(queue.STEP_ENV, "1")
    queue.guard("SPRT", dry_run=False)


def test_a_machine_without_the_agent_is_not_held(machine, monkeypatch, tmp_path):
    # CIやクラウドには常駐が無い。一時停止を求めない
    monkeypatch.setattr(queue, "plist_path", lambda: tmp_path / "none.plist")
    queue.guard("SPRT", dry_run=False)


def test_bench_and_sprt_stop_before_doing_anything(machine, monkeypatch):
    from hmwr.commands import build

    ran = []
    monkeypatch.setattr(proc, "run", lambda argv, **kw: ran.append(argv) or proc.OK)
    monkeypatch.setattr(build, "make_pair", lambda *a, **kw: ran.append("build") or proc.OK)
    assert cli.main(["bench", "a", "b"]) == proc.RUNTIME
    assert cli.main(["sprt", "run", "adr0001-x"]) == proc.RUNTIME
    assert ran == []
