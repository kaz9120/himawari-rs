"""実験のspecと実行器を検証する（ADR-0209）。

ステップの中身は走らせない。`proc.run` を差し替え、どのステップがどの順で
呼ばれ、完了印がどう裁くかを見る。
"""

import pytest

from hmwr import cli, paths, proc, spec
from hmwr.commands import exp

GOOD = '''
adr = "0209"  # 事前登録のADR

[[step]]
id = "mix"
run = "hmwr data mix m --in a --in b"

[[step]]
id = "train"
run = "hmwr net train n --data m --notes \\"混合 20%\\""

[[step]]
id = "match"
run = "hmwr match run x --cand-net n --stop pairs:10 --foreground"
'''


def with_step(run: str, sid: str = "s") -> str:
    return f'adr = "0209"\n[[step]]\nid = "{sid}"\nrun = "{run}"\n'


# --- specの検査 --------------------------------------------------------


def test_a_spec_is_a_list_of_hmwr_commands():
    s = spec.parse(GOOD, "x")
    assert s.adr == "0209"
    assert [step.id for step in s.steps] == ["mix", "train", "match"]
    assert s.steps[1].argv == ["net", "train", "n", "--data", "m", "--notes", "混合 20%"]


@pytest.mark.parametrize(
    "text",
    [
        with_step("python3 training/train.py --data x"),
        with_step("rm -rf data"),
        with_step("hmwr --dry-run data mix m --in a --in b"),
        with_step("hmwr match run x --stop pairs:10"),
        with_step("hmwr sprt run x"),
        with_step("hmwr data mix m", sid="bad id"),
        'adr = "210"\n[[step]]\nid = "s"\nrun = "hmwr env"\n',
        'adr = "0209"\n',
        with_step("hmwr env") + '[[step]]\nid = "s"\nrun = "hmwr env"\n',
    ],
    ids=[
        "hmwr以外", "シェルのコマンド", "dry-run", "切り離す対局", "切り離すSPRT",
        "idの文字", "adrの桁", "ステップなし", "idの重複",
    ],
)  # fmt: skip
def test_specs_that_break_the_rules_are_refused(text):
    with pytest.raises(spec.Invalid):
        spec.parse(text, "x")


def test_steps_never_go_through_a_shell():
    """連結の記号は引数として渡るだけで、解釈されない。"""
    s = spec.parse(with_step("hmwr data stats a ; rm -rf data"), "x")
    assert s.steps[0].argv == ["data", "stats", "a", ";", "rm", "-rf", "data"]


@pytest.mark.skipif(spec.tomllib is None, reason="tomllibが無いPython")
def test_the_fallback_parser_agrees_with_tomllib():
    assert spec.parse_subset(GOOD) == spec.tomllib.loads(GOOD)


def test_the_fallback_parser_refuses_what_it_cannot_read():
    with pytest.raises(spec.Invalid):
        spec.parse_subset('adr = "0209"\nsteps = ["a", "b"]\n')


def test_only_merged_specs_are_loaded(monkeypatch):
    asked = []

    def fake(argv, **_):
        asked.append(argv)
        return GOOD, ""

    monkeypatch.setattr(proc, "capture_both", fake)
    assert spec.load("adr0209-x").name == "adr0209-x"
    assert asked == [["git", "show", "origin/main:experiments/adr0209-x.toml"]]

    monkeypatch.setattr(proc, "capture_both", lambda argv, **_: ("", "fatal: not in origin/main"))
    with pytest.raises(proc.Fail):
        spec.load("adr0209-x")


# --- 実行器 ------------------------------------------------------------


@pytest.fixture
def runner(tmp_path, monkeypatch):
    calls, failing = [], set()
    monkeypatch.setattr(paths, "QUEUE", tmp_path / "queue")
    monkeypatch.setattr(paths, "LOGS", tmp_path / "logs")
    # 予行演習が手元の成果物を拾わないようにする
    for area in ("SPRT", "TRAIN", "NETS", "CHECKPOINTS", "BIN"):
        monkeypatch.setattr(paths, area, tmp_path / area.lower())
    monkeypatch.setattr(spec, "load", lambda name, ref="origin/main": spec.parse(GOOD, name))
    monkeypatch.setattr(proc, "succeeds", lambda argv, **_: True)

    def fake_run(argv, **_):
        step = argv[2:4]
        calls.append(step)
        if tuple(step) in failing:
            raise proc.Fail("落ちた", proc.RUNTIME)
        return proc.OK

    monkeypatch.setattr(proc, "run", fake_run)
    return calls, failing


def test_steps_run_in_order_and_are_not_repeated(runner, capsys):
    calls, _ = runner
    assert cli.main(["exp", "run", "x"]) == proc.OK
    assert calls == [["data", "mix"], ["net", "train"], ["match", "run"]]
    assert cli.main(["exp", "run", "x"]) == proc.OK
    assert len(calls) == 3
    assert capsys.readouterr().out.count("済み") == 3


def test_a_failed_step_stops_the_run_and_is_retried_next_time(runner):
    calls, failing = runner
    failing.add(("net", "train"))
    assert cli.main(["exp", "run", "x"]) == proc.RUNTIME
    assert calls == [["data", "mix"], ["net", "train"]]
    assert cli.main(["exp", "show", "x"]) == proc.JUDGE

    failing.clear()
    assert cli.main(["exp", "run", "x"]) == proc.OK
    assert calls[2:] == [["net", "train"], ["match", "run"]]
    assert cli.main(["exp", "show", "x"]) == proc.OK


def test_a_spec_changed_after_running_is_refused(runner, monkeypatch):
    assert cli.main(["exp", "run", "x"]) == proc.OK
    changed = GOOD.replace("--in a --in b", "--in a --in c")
    monkeypatch.setattr(spec, "load", lambda name, ref="origin/main": spec.parse(changed, name))
    assert cli.main(["exp", "run", "x"]) == proc.RUNTIME


def test_dry_run_leaves_no_marks(runner):
    assert cli.main(["--dry-run", "exp", "run", "x"]) == proc.OK
    assert not (paths.QUEUE / "x").exists()


def test_every_spec_in_the_repository_passes_the_check():
    """リポジトリのspecは、全ステップが予行演習を通る。CIでの検査を兼ねる。"""
    assert exp.check(cli.build_parser().parse_args(["exp", "check"])) == proc.OK
