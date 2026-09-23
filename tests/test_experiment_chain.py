"""実験のチェーンが `hmwr` のコマンドだけで書けることを固定する（ADR-0208）。

ADR-0206のチェーンは、68回の呼び出しのうち `hmwr` を通ったのが9回だった。
同じ実験を `hmwr` の列へ書き直し、全行がdry-runを通ることを検査する。
引数の誤りと、廃止されたコマンドはここで落ちる。
"""

import shlex

import pytest

from hmwr import cli, paths, proc

MATCH = (
    "--build adr0200-himawari --base-net pairrank_300M_q1 "
    "--stop pairs:500 --max-moves 400 --foreground"
)
TRAIN = "--valid valid_385M_q1 --rank rank_300M_100M --seed 0"
EK = "--openings entering_king_ply40"

ADR0206 = f"""
hmwr data shuffle ek_s --raw entering_king
hmwr data split ek_60M --in ek_s --count 60000000
hmwr data quiet ek_60M_q1 --in ek_60M
hmwr data split ek_18M_q1 --in ek_60M_q1 --count 18000000
hmwr data openings entering_king_ply40 --in ek_s --skip 100000000 --min-ply 40 --count 2000
hmwr data mix ekmix6_300M --in train_300M_q1 --in ek_18M_q1
hmwr net train ekmix6_300M --data ekmix6_300M {TRAIN}
hmwr data rm ekmix6_300M
hmwr data mix ekmix20_300M --in train_300M_q1 --in ek_60M_q1
hmwr net train ekmix20_300M --data ekmix20_300M {TRAIN}
hmwr data rm ekmix20_300M
hmwr match run adr0206-ekmix6_300M-normal --cand-net ekmix6_300M {MATCH}
hmwr match run adr0206-ekmix6_300M-ek --cand-net ekmix6_300M {EK} {MATCH}
hmwr match run adr0206-ekmix20_300M-normal --cand-net ekmix20_300M {MATCH}
hmwr match run adr0206-ekmix20_300M-ek --cand-net ekmix20_300M {EK} {MATCH}
"""

STEPS = ADR0206.strip().splitlines()


@pytest.fixture(autouse=True)
def empty_dirs(tmp_path, monkeypatch):
    """手元の成果物の有無で結果が変わらないようにする。"""
    monkeypatch.setattr(paths, "SPRT", tmp_path / "sprt")
    monkeypatch.setattr(paths, "TRAIN", tmp_path / "train")
    monkeypatch.setattr(paths, "LOGS", tmp_path / "logs")
    monkeypatch.setattr(paths, "CHECKPOINTS", tmp_path / "checkpoints")
    monkeypatch.setattr(paths, "NETS", tmp_path / "nets")


@pytest.mark.parametrize("step", STEPS)
def test_every_step_is_an_hmwr_command_that_dry_runs(step, capsys):
    argv = shlex.split(step)
    assert argv[0] == "hmwr"
    assert cli.main(["--dry-run", *argv[1:]]) == proc.OK
