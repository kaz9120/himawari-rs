"""実験の数値の表を検証する（ADR-0209）。

表は成果物から機械的に作る。読む場所と、欠けているときの表示を固定する。
"""

import json

import pytest

from hmwr import cli, paths, proc, report

RESULT = """name=adr0210-x
decision=指し切り
elo=+12.3
ci_low=-2.6
ci_high=+27.3
games=2000
wdl=+1010 =60 -930
llr=+0.91
elo0=0
elo1=5
"""
LOG = (
    "selfplay: a vs b | tc 10+0.1 | 並列 8 | SPRT elo[0, 5] α=0.05 β=0.05\n"
    "pairs   300 | +310 =20 -270 | [1,2,3,4,5] | Elo +23.2 [-4.1,+50.9] | LLR +0.80 [-2.94,2.94]\n"
)
REGISTRY_HEAD = (
    "timestamp\tname\tdata\tdata_n\tepochs\tbatch\tpeak_lr\tmin_lr\twarmup\tlambda\t"
    "best_step\tbest_valid\tfinal_valid\ttotal_steps\telapsed_s\tnotes\n"
)


@pytest.fixture
def artifacts(tmp_path, monkeypatch):
    for area in ("SPRT", "LOGS", "TRAIN"):
        (tmp_path / area.lower()).mkdir()
        monkeypatch.setattr(paths, area, tmp_path / area.lower())
    monkeypatch.setattr(paths, "REPO", tmp_path)
    (tmp_path / "training" / "runs").mkdir(parents=True)
    return tmp_path


def test_a_finished_match_is_read_from_its_result_file(artifacts):
    (paths.SPRT / "adr0210-x.result").write_text(RESULT, encoding="utf-8")
    row = report.match_row("adr0210-x", "a → b")
    assert row == [
        "adr0210-x", "a → b", "2000", "+1010 =60 -930", "+12.3 [-2.6, +27.3]", "+0.91", "指し切り",
    ]  # fmt: skip


def test_a_running_match_is_read_from_its_log(artifacts):
    (paths.LOGS / "sprt-adr0210-y.log").write_text(LOG, encoding="utf-8")
    row = report.match_row("adr0210-y")
    assert row[2:] == ["600", "+310 =20 -270", "+23.2 [-4.1, +50.9]", "+0.80", "途中"]


def test_a_match_that_never_started_says_so(artifacts):
    assert report.match_row("nope")[-1] == "未着手"


def test_training_is_read_from_the_registry_and_the_last_row_wins(artifacts):
    rows = [
        "t\tn\tdata/train/old.psv\t1000\t1\t16384\t0.001\t1e-06\t100\t0.7\t10\t0.6\t0.6\t20\t36\tmemo",
        "t\tn\tdata/train/m.psv\t360000000\t1\t16384\t0.001\t1e-06\t100\t0.7\t20000\t0.49123\t0.49117\t21973\t7532\tmemo",
    ]
    (artifacts / report.REGISTRY).write_text(REGISTRY_HEAD + "\n".join(rows) + "\n", encoding="utf-8")
    assert report.train_row("n") == [
        "n", "data/train/m.psv", "360,000,000", "21973", "0.49123（step 20000）", "0.49117", "2.1時間",
    ]  # fmt: skip
    assert "台帳に無い" in report.train_row("other")[-1]


def test_data_is_read_from_the_done_mark(artifacts):
    (paths.TRAIN / "m.psv.done").write_text(json.dumps({"bytes": 4000, "seconds": 7}))
    assert report.data_row("m") == ["m.psv", "100", "4,000", "7秒"]
    assert report.data_row("missing") is None


def test_names_are_collected_from_the_steps():
    steps = [
        ["data", "mix", "m", "--in", "a", "--in", "b"],
        ["net", "train", "n", "--data", "m"],
        ["data", "rm", "m"],
        ["match", "run", "x", "--base-net", "ctrl", "--cand-net", "n", "--stop", "pairs:10"],
    ]
    matches, nets, data = report.from_steps(steps)
    assert matches == [("x", "ctrl → n")]
    assert nets == ["n"]
    assert data == ["m"]


def test_report_needs_something_to_report():
    assert cli.main(["exp", "report"]) == proc.USAGE


def test_report_prints_markdown_tables(artifacts, capsys):
    (paths.SPRT / "adr0210-x.result").write_text(RESULT, encoding="utf-8")
    assert cli.main(["exp", "report", "--match", "adr0210-x", "--net", "n"]) == proc.OK
    out = capsys.readouterr().out
    assert "### 対局" in out and "| adr0210-x |" in out
    assert "### 学習" in out
