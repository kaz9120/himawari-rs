"""状態ファイルと `hmwr status`（ADR-0220）を検証する。

状態ファイルは形式と間隔と生死の判定を見る。statusはGitHubを読まずに、状態ファイル・
結果・資源・設定が1つのJSONにまとまることを見る。
"""

import json
import os
import time

import pytest

from hmwr import cli, heartbeat, paths, proc
from hmwr.commands import status


@pytest.fixture
def home(tmp_path, monkeypatch):
    monkeypatch.setattr(paths, "STATUS", tmp_path / "status")
    monkeypatch.setattr(paths, "SPRT", tmp_path / "sprt")
    monkeypatch.setattr(paths, "TRAIN", tmp_path / "train")
    (tmp_path / "train").mkdir()
    return tmp_path


def test_heartbeat_writes_on_start_and_finish_and_throttles_updates(home):
    hb = heartbeat.Heartbeat("relabel", "x", total=100, unit="局面", detail={"scale": 430}, interval=3600)
    path = home / "status" / "relabel-x.json"
    first = json.loads(path.read_text())
    assert first["state"] == "running" and first["pid"] == os.getpid()
    assert first["progress"] == {"done": 0, "total": 100, "unit": "局面"}
    assert first["detail"] == {"scale": 430}
    hb.update(10)
    hb.update(50)
    # 間隔に満たないので、ファイルはまだ開始時のまま
    assert json.loads(path.read_text())["progress"]["done"] == 0
    hb.update(60, force=True)
    mid = json.loads(path.read_text())
    assert mid["progress"]["done"] == 60
    assert mid["rate"] is not None and mid["eta_seconds"] is not None
    hb.finish("done", positions_per_second=123)
    last = json.loads(path.read_text())
    assert last["state"] == "done" and last["progress"]["done"] == 100
    assert last["eta_seconds"] == 0 and last["detail"]["positions_per_second"] == 123
    assert not path.with_name(path.name + ".tmp").exists()


def test_heartbeat_refuses_unknown_state(home):
    hb = heartbeat.Heartbeat("net", "y")
    with pytest.raises(ValueError):
        hb.finish("odd")


def test_read_all_marks_dead_and_stale_runs(home):
    hb = heartbeat.Heartbeat("match", "alive")
    dead = heartbeat.Heartbeat("match", "dead")
    dead.data["pid"] = 999999999
    dead._write()
    old = home / "status" / "match-dead.json"
    past = time.time() - 3600
    os.utime(old, (past, past))
    beats = {b["name"]: b for b in heartbeat.read_all(stale_after=600)}
    assert beats["alive"]["alive"] is True and beats["alive"]["stale"] is False
    assert beats["dead"]["alive"] is False and beats["dead"]["stale"] is True
    hb.finish("done")
    assert "alive" not in {b["name"]: b for b in heartbeat.read_all()}["alive"]


def test_status_json_collects_everything_without_github(home, monkeypatch, capsys):
    (home / "sprt").mkdir()
    (home / "sprt" / "adr0001-x.result").write_text(
        "name=adr0001-x\ndecision=H1\nelo=+10.0\nci_low=+2.0\nci_high=+18.0\ngames=2000\nllr=+3.0\n"
    )
    heartbeat.Heartbeat("relabel", "big", total=10, unit="局面")
    (home / "train" / "old.psv.relabel.json").write_text(
        json.dumps({"labeler": "dlshogi", "scale": 430, "start": 0, "count": 5, "done": 2,
                    "started": "t0", "updated": "t1", "finished": None})
    )
    monkeypatch.setattr(status, "_launchd", lambda: {"loaded": False})
    assert cli.main(["status", "--json", "--no-github"]) == proc.OK
    d = json.loads(capsys.readouterr().out)
    assert set(d) == {"generated", "queue", "heartbeats", "results", "resources", "config"}
    assert d["queue"]["error"]
    names = {(b["kind"], b["name"]) for b in d["heartbeats"]}
    assert ("relabel", "big") in names and ("relabel", "old") in names
    assert d["results"]["matches"][0]["decision"] == "H1"
    assert d["resources"]["disk_free_gb"] > 0
    assert any(k == "評価関数" for k, _ in d["config"])


def test_status_renders_a_readable_table(home, monkeypatch, capsys):
    monkeypatch.setattr(status, "_launchd", lambda: {"loaded": True, "pid": 1})
    heartbeat.Heartbeat("net", "train1", total=1000, unit="step")
    assert cli.main(["status", "--no-github"]) == proc.OK
    out = capsys.readouterr().out
    assert "== キュー" in out and "== 状態ファイル" in out and "net:train1" in out
    assert "キューの常駐: あり（pid 1）" in out


def test_status_config_replaces_env(capsys):
    assert cli.main(["status", "--config"]) == proc.OK
    assert "評価関数" in capsys.readouterr().out
    assert cli.main(["env"]) == proc.OK
    assert "hmwr status --config" in capsys.readouterr().out
