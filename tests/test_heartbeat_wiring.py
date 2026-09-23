"""長く走るコマンドへの心拍の配線（ADR-0220の2段目）を検証する。

心拍の形式そのものは test_status.py が見る。ここでは、各コマンドの出力や
進み具合が心拍へ写ることと、抜け方で終わりの状態が決まることを見る。
学習と対局は起こさず、出力の行を食わせる。
"""

import json
import sys

import pytest

from hmwr import cli, heartbeat, paths, proc, spec
from hmwr.commands import exp, match, net


def beat_of(kind: str, name: str) -> dict:
    return json.loads(heartbeat.path_of(kind, name).read_text(encoding="utf-8"))


# --- 抜け方と終わりの状態 ----------------------------------------------


def test_leaving_quietly_writes_done():
    with heartbeat.running("train", "a", total=10, unit="step"):
        pass
    assert beat_of("train", "a")["state"] == "done"


def test_a_failure_writes_failed_with_the_first_line():
    with pytest.raises(proc.Fail):
        with heartbeat.running("train", "b"):
            raise proc.Fail("学習データがない\n詳細")
    d = beat_of("train", "b")
    assert d["state"] == "failed" and d["detail"]["error"] == "学習データがない"


def test_an_explicit_finish_is_kept():
    with heartbeat.running("spsa", "c") as beat:
        beat.finish("stopped")
    assert beat_of("spsa", "c")["state"] == "stopped"


def test_dry_run_writes_no_heartbeat():
    with heartbeat.running("train", "d", dry_run=True) as beat:
        beat.update(5, force=True)
    assert not heartbeat.path_of("train", "d").exists()


def test_steps_do_not_estimate_the_remaining_time():
    beat = heartbeat.Heartbeat("exp", "e", total=5, estimate=False)
    beat.update(1)
    beat.update(2, force=True)
    assert beat_of("exp", "e")["eta_seconds"] is None


# --- 子プロセスの出力を行で受ける --------------------------------------


def test_run_hands_each_line_to_the_callback(tmp_path):
    lines = []
    code = proc.run(
        [sys.executable, "-c", "print('step 1'); print('valid loss 0.5')"],
        log=tmp_path / "x.log",
        on_line=lines.append,
    )
    assert code == proc.OK
    assert lines == ["step 1", "valid loss 0.5"]


# --- 学習 --------------------------------------------------------------

# training/train.py の実際の出力（data/logs/train-t20_7860M.log から）
TRAIN_LINES = [
    "学習データ: 2000000000局面 × 1エポック, batch=16384, peak_lr=0.001, warmup=100, "
    "total_steps=122071, λ=0.7",
    "step 2000 samples 32768000 loss 0.51672 lr 0.000999 rank 0.00738 fire 23.2% (48129 samples/s)",
    "  valid loss 0.51321",
    "  best checkpoint: data/nets/t20_7860M.hmwr.best (step 2000, valid 0.51321)",
    "step 4000 samples 65536000 loss 0.51000 lr 0.000999 rank 0.00700 fire 22.0% (48000 samples/s)",
    "  valid loss 0.51400",
]


def test_training_output_moves_the_heartbeat():
    with heartbeat.running("train", "t", unit="step") as beat:
        on_line = net.train_progress(beat)
        for line in TRAIN_LINES:
            on_line(line)
        beat.update(force=True)
        d = beat_of("train", "t")
    assert d["progress"] == {"done": 4000, "total": 122071, "unit": "step"}
    assert d["detail"]["loss"] == 0.51
    # 最新のvalidと、これまでの最良を分けて持つ
    assert d["detail"]["valid"] == 0.514 and d["detail"]["best_valid"] == 0.51321


# --- 対局 --------------------------------------------------------------

PAIRS = "pairs   525 | +602 =46 -402 | [73,22,236,20,174] | Elo +67.0 [+45.1,+89.3] | LLR +2.05 [-2.94,2.94]"
START = "selfplay: cand vs base | tc 10+0.1 | 並列 8 | SPRT elo[0, 5] α=0.05 β=0.05"
VERDICT = "H1採択（候補は有意に強い） | pairs 600 games 1200 | +650 =50 -500 | Elo +43.6 [+20.0,+67.0] | LLR +2.95"


def test_the_progress_line_of_selfplay_moves_the_heartbeat():
    with heartbeat.running("match", "m", unit="ペア") as beat:
        match.match_progress(beat)("ignored line")
        match.match_progress(beat)(PAIRS)
        beat.update(force=True)
        d = beat_of("match", "m")
    assert d["progress"]["done"] == 525
    assert d["detail"]["elo"] == 67.0 and d["detail"]["llr"] == 2.05
    assert (d["detail"]["ci_low"], d["detail"]["ci_high"]) == (45.1, 89.3)


@pytest.fixture
def match_dirs(tmp_path, monkeypatch):
    for area in ("SPRT", "LOGS", "BIN"):
        monkeypatch.setattr(paths, area, tmp_path / area.lower())
    return tmp_path


def _spec(stop_pairs=0):
    player = match.Player(build="/bin/true")
    return match.Spec(name="adr0001-x", base=player, cand=player, stop_pairs=stop_pairs)


def test_a_verdict_is_written_as_done_with_the_decision(match_dirs, monkeypatch):
    def fake_selfplay(spec, *, dry_run, attempt, on_line=None):
        f = match.files(spec.name)
        f["log"].write_text(f"{START}\n{PAIRS}\n{VERDICT}\n", encoding="utf-8")
        on_line(PAIRS)
        return 0

    monkeypatch.setattr(match, "_selfplay", fake_selfplay)
    assert match.until_decision(_spec(), dry_run=False) == proc.OK
    d = beat_of("match", "adr0001-x")
    assert d["state"] == "done" and d["detail"]["decision"] == "H1"
    assert d["detail"]["stop"] == "sprt" and d["progress"]["total"] is None


def test_stopping_short_of_fixed_pairs_is_stopped(match_dirs, monkeypatch):
    monkeypatch.setattr(match, "_selfplay", lambda spec, **_: 2)
    assert match.until_decision(_spec(stop_pairs=500), dry_run=False) == 2
    d = beat_of("match", "adr0001-x")
    assert d["state"] == "stopped" and d["progress"]["total"] == 500


# --- 実験 --------------------------------------------------------------

SPEC = '''
adr = "0220"

[[step]]
id = "mix"
run = "hmwr data mix m --in a --in b"

[[step]]
id = "train"
run = "hmwr net train n --data m"
'''


@pytest.fixture
def exp_runner(tmp_path, monkeypatch):
    monkeypatch.setattr(paths, "QUEUE", tmp_path / "queue")
    monkeypatch.setattr(paths, "LOGS", tmp_path / "logs")
    monkeypatch.setattr(spec, "load", lambda name, ref="origin/main": spec.parse(SPEC, name))
    monkeypatch.setattr(proc, "succeeds", lambda argv, **_: True)
    seen = []

    def fake_run(argv, **_):
        # ステップを始める前に、心拍が今のステップを指している
        seen.append(beat_of("exp", "x")["detail"]["step"])
        return proc.OK

    monkeypatch.setattr(proc, "run", fake_run)
    return seen


def test_an_experiment_reports_the_current_step(exp_runner):
    assert cli.main(["exp", "run", "x"]) == proc.OK
    assert exp_runner == ["mix", "train"]
    d = beat_of("exp", "x")
    assert d["state"] == "done" and d["progress"]["done"] == 2 and d["progress"]["total"] == 2


def test_a_paused_experiment_is_stopped(exp_runner):
    exp.pause_file().parent.mkdir(parents=True, exist_ok=True)
    exp.pause_file().touch()
    assert cli.main(["exp", "run", "x"]) == proc.JUDGE
    d = beat_of("exp", "x")
    assert d["state"] == "stopped" and d["detail"]["reason"] == "paused"
