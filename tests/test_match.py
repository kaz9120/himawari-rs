"""`hmwr match` を検証する（ADR-0208）。

対局は起こさない。`--dry-run` が組み立てるコマンドと、条件の記録の裁き方、
指し切りの結果の書き方を固定する。
"""

import pytest

from hmwr import cli, paths, proc, sprt_log
from hmwr.commands import match


def dry(capsys, argv):
    code = cli.main(["--dry-run", *argv])
    out = capsys.readouterr().out
    return code, [line for line in out.splitlines() if line.startswith("[dry-run]")]


def play_line(lines):
    return [x for x in lines if "selfplay" in x][0]


@pytest.fixture(autouse=True)
def empty_dirs(tmp_path, monkeypatch):
    """手元の棋譜やビルドの有無でコマンドが変わらないようにする。"""
    monkeypatch.setattr(paths, "SPRT", tmp_path / "sprt")
    monkeypatch.setattr(paths, "BIN", tmp_path / "bin")
    monkeypatch.setattr(paths, "LOGS", tmp_path / "logs")
    monkeypatch.delenv("EVAL_FILE", raising=False)
    (tmp_path / "bin").mkdir()
    return tmp_path


# --- コマンドの組み立て ------------------------------------------------


def test_fixed_pairs_matches_the_adr0206_chain(capsys):
    """ADR-0206のチェーンが selfplay を直接叩いた条件を、名前だけで書ける。"""
    _, lines = dry(
        capsys,
        [
            "match", "run", "adr0210-x",
            "--build", "adr0200-himawari",
            "--base-net", "pairrank_300M_q1",
            "--cand-net", "mixhao20_300M",
            "--stop", "pairs:1000",
            "--max-moves", "400",
        ],
    )  # fmt: skip
    play = play_line(lines)
    assert "--baseline" in play and "bin/adr0200-himawari --candidate" in play
    assert "--bopt EvalFile=" in play and "data/nets/pairrank_300M_q1.hmwr" in play
    assert "--copt EvalFile=" in play and "data/nets/mixhao20_300M.hmwr" in play
    assert "--option EvalFile=" not in play
    # 並列数はマシンのコア数で決まるので、ここでは見ない
    assert "--tc 10+0.1 --concurrency " in play
    assert "--hash 64 --max-moves 400 --adjudicate 2000,8" in play
    assert "--no-stop" in play
    assert "--max-pairs 1000" in play


def test_detached_worker_reuses_the_same_arguments(capsys):
    _, lines = dry(capsys, ["match", "run", "x", "--stop", "pairs:10", "--cand-odds", "0.5"])
    worker = [x for x in lines if "--worker" in x][0]
    assert "bin/hmwr match run x --stop pairs:10 --cand-odds 0.5 --worker" in worker
    assert "--dry-run" not in worker


def test_defaults_are_passed_explicitly(capsys):
    """既定はselfplay任せにせず、1か所の値を明示して渡す。"""
    _, lines = dry(capsys, ["match", "run", "x"])
    play = play_line(lines)
    assert "--hash 64 --max-moves 320" in play
    assert "--option EvalFile=" in play
    assert "--no-stop" not in play
    assert "--max-pairs 10000" in play


def test_per_side_options_and_odds(capsys):
    _, lines = dry(
        capsys,
        [
            "match", "run", "x",
            "--base-build", "/opt/ref/engine",
            "--opt", "USI_Hash=256",
            "--cand-opt", "Threads=8",
            "--base-odds", "2",
            "--openings", "start_sfens_ply32",
        ],
    )  # fmt: skip
    play = play_line(lines)
    assert "--baseline /opt/ref/engine" in play
    assert "--option USI_Hash=256" in play
    assert "--copt Threads=8" in play
    assert "--bodds 2" in play
    assert "--openings openings/start_sfens_ply32.txt" in play


def test_builds_default_to_the_pair(capsys, empty_dirs):
    """ビルドを省くと build pair の出力を使う。sprt run と同じ対局になる。"""
    for side in ("base", "cand"):
        (empty_dirs / "bin" / f"{side}-adr0180-x").write_text("")
    _, new = dry(capsys, ["match", "run", "adr0180-x", "--foreground"])
    _, old = dry(capsys, ["sprt", "run", "adr0180-x", "--foreground", "--no-verify"])
    assert play_line(new) == play_line(old)
    assert "bin/base-adr0180-x --candidate" in play_line(new)


@pytest.mark.parametrize("stop", ["pairs:0", "pairs:x", "games:10", "pairs"])
def test_stop_rule_is_validated(stop):
    assert cli.main(["--dry-run", "match", "run", "x", "--stop", stop]) == proc.USAGE


def test_sprt_only_flags_are_refused_with_fixed_pairs():
    argv = ["--dry-run", "match", "run", "x", "--stop", "pairs:10", "--noninferiority"]
    assert cli.main(argv) == proc.USAGE


def test_concurrency_flag_matches_the_environment_variable(capsys, monkeypatch):
    """`--concurrency N` は SPRT_CONCURRENCY=N と同じ条件を組み立てる。"""
    _, by_flag = dry(capsys, ["match", "run", "x", "--concurrency", "2"])
    monkeypatch.setenv("SPRT_CONCURRENCY", "2")
    _, by_env = dry(capsys, ["match", "run", "x"])
    assert "--concurrency 2 " in play_line(by_flag)
    assert play_line(by_flag) == play_line(by_env)


@pytest.mark.parametrize("value", ["0", "-1"])
def test_concurrency_is_validated(value):
    assert cli.main(["--dry-run", "match", "run", "x", "--concurrency", value]) == proc.USAGE


def test_sprt_hands_the_concurrency_to_the_detached_worker(capsys):
    """sprtは畳んだ条件を --set で子へ渡す。鍵を許していないと子が落ちる。"""
    _, lines = dry(capsys, ["sprt", "run", "x", "--concurrency", "3", "--no-verify"])
    worker = [x for x in lines if "--worker" in x][0]
    assert "--set SPRT_CONCURRENCY=3" in worker
    assert cli.main(["--dry-run", "match", "run", "x", "--set", "SPRT_CONCURRENCY=3"]) == proc.OK


def test_missing_files_fail_before_starting():
    assert cli.main(["match", "run", "x", "--cand-net", "no_such_net"]) == proc.RUNTIME


# --- 条件の記録 --------------------------------------------------------


def test_conditions_are_recorded_on_a_fresh_start(tmp_path):
    cond = tmp_path / "x.cond"
    match.check_conditions(cond, "selfplay a", 0, adopt=False, dry_run=False)
    assert "selfplay a" in cond.read_text()
    match.check_conditions(cond, "selfplay a", 10, adopt=False, dry_run=False)


def test_other_conditions_are_refused_on_resume(tmp_path):
    cond = tmp_path / "x.cond"
    match.check_conditions(cond, "selfplay a", 0, adopt=False, dry_run=False)
    with pytest.raises(proc.Fail):
        match.check_conditions(cond, "selfplay b", 10, adopt=False, dry_run=False)


def test_a_record_left_by_a_deleted_kifu_is_overwritten(tmp_path):
    cond = tmp_path / "x.cond"
    match.check_conditions(cond, "selfplay a", 0, adopt=False, dry_run=False)
    match.check_conditions(cond, "selfplay b", 0, adopt=False, dry_run=False)
    assert "selfplay b" in cond.read_text()


def test_kifu_without_a_record_needs_adopt(tmp_path):
    cond = tmp_path / "x.cond"
    with pytest.raises(proc.Fail):
        match.check_conditions(cond, "selfplay a", 10, adopt=False, dry_run=False)
    match.check_conditions(cond, "selfplay a", 10, adopt=True, dry_run=False)
    assert cond.is_file()


def test_dry_run_writes_nothing(tmp_path):
    cond = tmp_path / "x.cond"
    match.check_conditions(cond, "selfplay a", 0, adopt=False, dry_run=True)
    assert not cond.exists()


# --- 指し切りの結果 ----------------------------------------------------

START = "selfplay: cand vs base | tc 10+0.1 | 並列 8 | SPRT elo[0, 5] α=0.05 β=0.05"
END = "判定に至らず | pairs {p} games {g} | +537 =33 -430 | Elo +37.3 [+16.1,+58.8] | LLR +1.50"


def test_playing_out_the_fixed_pairs_counts_as_done(tmp_path):
    log, result = tmp_path / "x.log", tmp_path / "x.result"
    log.write_text(START + "\n" + END.format(p=500, g=1000) + "\n", encoding="utf-8")
    text, verdict = sprt_log.report(log, "x", result=result, fixed_pairs=500)
    assert verdict == "指し切り"
    assert "1000局を指し切り" in text
    assert "decision=指し切り" in result.read_text(encoding="utf-8")
    assert sprt_log.EXIT_BY_VERDICT[verdict] == proc.OK


def test_stopping_short_of_the_fixed_pairs_is_not_done(tmp_path):
    log, result = tmp_path / "x.log", tmp_path / "x.result"
    log.write_text(START + "\n" + END.format(p=300, g=600) + "\n", encoding="utf-8")
    _, verdict = sprt_log.report(log, "x", result=result, fixed_pairs=500)
    assert verdict == "打ち切り"
    assert not result.exists()


def test_sprt_runs_are_unchanged_by_the_fixed_pairs_rule(tmp_path):
    log, result = tmp_path / "x.log", tmp_path / "x.result"
    log.write_text(START + "\n" + END.format(p=500, g=1000) + "\n", encoding="utf-8")
    _, verdict = sprt_log.report(log, "x", result=result)
    assert verdict == "打ち切り"
    assert not result.exists()


def test_conditions_do_not_depend_on_where_the_repository_lives(tmp_path):
    """開発用の作業ツリーと専用worktreeで、同じ条件が食い違わない。"""
    cond = tmp_path / "x.cond"
    here = f"selfplay --option EvalFile={paths.REPO}/data/nets/a.hmwr"
    there = f"selfplay --option EvalFile={paths.REPO.parent}/{paths.REPO.name}-runner/data/nets/a.hmwr"
    other = "selfplay --option EvalFile=/elsewhere/data/nets/a.hmwr"
    match.check_conditions(cond, here, 0, adopt=False, dry_run=False)
    match.check_conditions(cond, "selfplay --option EvalFile=data/nets/a.hmwr", 10, adopt=False, dry_run=False)
    match.check_conditions(cond, there, 10, adopt=False, dry_run=False)
    with pytest.raises(proc.Fail):
        match.check_conditions(cond, other, 10, adopt=False, dry_run=False)
