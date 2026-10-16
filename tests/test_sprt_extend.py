"""`hmwr sprt extend` を検証する（ADR-0217の追記、issue #782）。

対局は起こさない。見送りの結果と条件の記録を置き、延ばすかどうかの裁きと、
組み立て直した対局のコマンドを見る。
"""

import pytest

from hmwr import cli, paths, proc
from hmwr.commands import match, queue

NAME = "adr9999-x"


def cond_line(tmp_path, *, extra=""):
    """`sprt run` が控える形の条件。パスは一時ディレクトリの絶対パスにする。"""
    return (
        f"target/release/selfplay --baseline {paths.BIN}/base-{NAME} "
        f"--candidate {paths.BIN}/cand-{NAME} --option EvalFile={tmp_path}/a.hmwr{extra} "
        f"--openings {tmp_path}/open.txt --tc 10+0.1 --concurrency 8 --hash 64 --max-moves 320 "
        "--adjudicate 2000,8 --elo0 0 --elo1 5 --alpha 0.05 --beta 0.05"
    )


def put(tmp_path, *, decision="見送り", elo="+5.1", elo0="0", elo1="5", games=20000, line=None):
    """見送りの結果・条件の記録・棋譜・対局者のファイルを置く。"""
    paths.SPRT.mkdir(parents=True, exist_ok=True)
    paths.BIN.mkdir(parents=True, exist_ok=True)
    for path in (paths.BIN / f"base-{NAME}", paths.BIN / f"cand-{NAME}", tmp_path / "a.hmwr", tmp_path / "open.txt"):
        path.write_bytes(b"x")
    # 対局の起動と同じく、proc.show を通した形で控える
    shown = proc.show((line or cond_line(tmp_path)).split())
    match.check_conditions(paths.SPRT / f"{NAME}.cond", shown, 0, adopt=False, dry_run=False)
    (paths.SPRT / f"{NAME}.jsonl").write_text("{}\n" * games, encoding="utf-8")
    (paths.SPRT / f"{NAME}.result").write_text(
        f"name={NAME}\ndecision={decision}\nelo={elo}\nci_low=+0.4\nci_high=+9.7\n"
        f"games={games}\nllr=+2.30\nelo0={elo0}\nelo1={elo1}\n",
        encoding="utf-8",
    )


@pytest.fixture
def started(monkeypatch):
    """前面で走らせる処理を差し替え、渡された条件を控える。"""
    specs = []
    monkeypatch.setattr(queue, "guard", lambda *a, **k: None)
    monkeypatch.setattr(match, "until_decision", lambda spec, **_: specs.append(spec) or proc.OK)
    return specs


def test_a_capped_run_above_the_midpoint_is_archived_and_resumed(tmp_path, started):
    put(tmp_path)
    assert cli.main(["sprt", "extend", NAME, "--foreground"]) == proc.OK
    assert (paths.SPRT / f"{NAME}.result.capped-10000").is_file()
    assert not (paths.SPRT / f"{NAME}.result").exists()
    [spec] = started
    assert spec.env["SPRT_MAX_PAIRS"] == "20000"
    assert spec.base.build == str(paths.BIN / f"base-{NAME}")


def test_the_second_extension_keeps_the_first_archive(tmp_path, started):
    put(tmp_path)
    (paths.SPRT / f"{NAME}.result.capped-10000").write_text("前回\n", encoding="utf-8")
    (paths.SPRT / f"{NAME}.result").unlink()
    put(tmp_path, games=40000)
    assert cli.main(["sprt", "extend", NAME, "--foreground"]) == proc.OK
    assert (paths.SPRT / f"{NAME}.result.capped-10000").read_text(encoding="utf-8") == "前回\n"
    assert (paths.SPRT / f"{NAME}.result.capped-20000").is_file()
    assert started[0].env["SPRT_MAX_PAIRS"] == "30000"


def test_dry_run_shows_the_resume_and_renames_nothing(tmp_path, capsys):
    put(tmp_path)
    assert cli.main(["--dry-run", "sprt", "extend", NAME]) == proc.OK
    out = capsys.readouterr().out
    assert f"退避 data/sprt/{NAME}.result → data/sprt/{NAME}.result.capped-10000" in out
    play = next(x for x in out.splitlines() if "selfplay --baseline" in x)
    assert "--max-pairs 20000" in play and "--resume" in play
    worker = next(x for x in out.splitlines() if "--worker" in x)
    assert f"sprt extend {NAME} --max-pairs 20000 --worker" in worker
    assert (paths.SPRT / f"{NAME}.result").is_file()


def test_at_or_below_the_midpoint_it_is_settled_as_capped(tmp_path, started):
    put(tmp_path, elo="+2.5")
    assert cli.main(["sprt", "extend", NAME, "--foreground"]) == proc.JUDGE
    assert (paths.SPRT / f"{NAME}.result").is_file()
    assert started == []


def test_noninferiority_uses_its_own_midpoint(tmp_path, started):
    """非劣性の中点は−2.5である。点推定−2.0は延ばす。"""
    put(tmp_path, elo="-2.0", elo0="-5", elo1="0", line=cond_line(tmp_path).replace(
        "--elo0 0 --elo1 5", "--elo0 -5 --elo1 0"
    ))
    assert cli.main(["sprt", "extend", NAME, "--foreground"]) == proc.OK
    assert started[0].env["SPRT_ELO0"] == "-5"


@pytest.mark.parametrize("decision", ["H1", "H0", "指し切り"])
def test_only_capped_runs_are_extended(tmp_path, started, decision):
    put(tmp_path, decision=decision)
    assert cli.main(["sprt", "extend", NAME, "--foreground"]) == proc.RUNTIME
    assert (paths.SPRT / f"{NAME}.result").is_file()


def test_beyond_the_owner_line_needs_approval(tmp_path, started):
    put(tmp_path, games=60000)
    assert cli.main(["sprt", "extend", NAME, "--foreground"]) == proc.USAGE
    assert (paths.SPRT / f"{NAME}.result").is_file()
    assert cli.main(["sprt", "extend", NAME, "--foreground", "--owner-approved"]) == proc.OK
    assert started[0].env["SPRT_MAX_PAIRS"] == "40000"


def test_a_cap_below_the_current_pairs_is_refused(tmp_path, started):
    put(tmp_path)
    assert cli.main(["sprt", "extend", NAME, "--foreground", "--max-pairs", "5000"]) == proc.USAGE


def test_conditions_that_cannot_be_rebuilt_are_refused(tmp_path, started):
    """組み立て直した対局が控えと食い違うなら、棋譜へ積まない。"""
    put(tmp_path, line=cond_line(tmp_path) + " --unknown 1")
    assert cli.main(["sprt", "extend", NAME, "--foreground"]) == proc.RUNTIME
    assert (paths.SPRT / f"{NAME}.result").is_file()


def test_per_side_nets_are_rebuilt(tmp_path, started):
    """`sprt net` の条件（片側ずつの評価関数）も組み立て直せる。"""
    line = cond_line(tmp_path).replace(
        f"--option EvalFile={tmp_path}/a.hmwr",
        f"--bopt EvalFile={tmp_path}/a.hmwr --copt EvalFile={tmp_path}/a.hmwr --option USI_Hash=1",
    )
    put(tmp_path, line=line)
    assert cli.main(["sprt", "extend", NAME, "--foreground"]) == proc.OK
    spec = started[0]
    assert spec.base.net == spec.cand.net == str(tmp_path / "a.hmwr")
    assert spec.opts == ("USI_Hash=1",)


def test_fixed_pairs_are_not_extended(tmp_path, started):
    put(tmp_path, line=cond_line(tmp_path) + " --no-stop")
    assert cli.main(["sprt", "extend", NAME, "--foreground"]) == proc.RUNTIME


def test_missing_players_stop_before_archiving(tmp_path, started):
    put(tmp_path)
    (paths.BIN / f"base-{NAME}").unlink()
    assert cli.main(["sprt", "extend", NAME, "--foreground"]) == proc.RUNTIME
    assert (paths.SPRT / f"{NAME}.result").is_file()
