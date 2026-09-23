"""`hmwr net train` の完了マーカーと再開を検証する（ADR-0208・ADR-0209）。

学習は回さない。学習器の起動を差し替え、渡された引数だけを見る。
"""

import pytest

from hmwr import cli, paths, proc
from hmwr.commands import net


@pytest.fixture
def trainer(tmp_path, monkeypatch):
    """学習器の代わりに、引数を控えて最終ネットを書くだけの関数を置く。"""
    calls = []
    train_dir = tmp_path / "train"
    train_dir.mkdir()
    for name in ("d.psv", "d2.psv", "v.psv"):
        (train_dir / name).write_bytes(b"x")
    monkeypatch.setattr(paths, "NETS", tmp_path / "nets")
    monkeypatch.setattr(paths, "CHECKPOINTS", tmp_path / "checkpoints")
    monkeypatch.setattr(paths, "LOGS", tmp_path / "logs")
    monkeypatch.setattr(net, "_ensure_extension", lambda **_: None)

    def fake_run(argv, **_):
        calls.append(argv)
        out = argv[argv.index("--out") + 1]
        with open(out, "wb") as fh:
            fh.write(b"net")
        return proc.OK

    monkeypatch.setattr(proc, "run", fake_run)
    return calls, train_dir


def argv_for(train_dir, data="d.psv", *extra):
    return [
        "net", "train", "x",
        "--data", str(train_dir / data),
        "--valid", str(train_dir / "v.psv"),
        *extra,
    ]  # fmt: skip


def test_a_finished_training_is_not_repeated(trainer, capsys):
    calls, train_dir = trainer
    assert cli.main(argv_for(train_dir)) == proc.OK
    assert cli.main(argv_for(train_dir)) == proc.OK
    assert len(calls) == 1
    assert "済み" in capsys.readouterr().out


def test_notes_are_not_a_condition(trainer):
    calls, train_dir = trainer
    assert cli.main(argv_for(train_dir, "d.psv", "--notes", "一度目")) == proc.OK
    assert cli.main(argv_for(train_dir, "d.psv", "--notes", "二度目")) == proc.OK
    assert len(calls) == 1


def test_an_existing_net_is_not_overwritten_by_other_conditions(trainer):
    calls, train_dir = trainer
    assert cli.main(argv_for(train_dir)) == proc.OK
    assert cli.main(argv_for(train_dir, "d2.psv")) == proc.RUNTIME
    assert cli.main(argv_for(train_dir, "d2.psv", "--force")) == proc.OK
    assert len(calls) == 2


def test_an_interrupted_training_resumes_from_the_latest_checkpoint(trainer):
    calls, train_dir = trainer
    assert cli.main(argv_for(train_dir)) == proc.OK
    # 最終ネットが書かれる前に止まった状態を作る
    (paths.NETS / "x.hmwr").unlink()
    latest = paths.CHECKPOINTS / "x" / "latest.ckpt"
    latest.write_bytes(b"ckpt")

    assert cli.main(argv_for(train_dir)) == proc.OK
    assert calls[1][-2:] == ["--resume", str(latest)]
    assert "--resume" not in calls[0]


def test_a_checkpoint_of_other_conditions_is_not_resumed(trainer):
    calls, train_dir = trainer
    assert cli.main(argv_for(train_dir)) == proc.OK
    (paths.NETS / "x.hmwr").unlink()
    (paths.CHECKPOINTS / "x" / "latest.ckpt").write_bytes(b"ckpt")
    assert cli.main(argv_for(train_dir, "d2.psv")) == proc.RUNTIME
    assert len(calls) == 1


def test_a_checkpoint_without_a_record_needs_adopt(trainer):
    calls, train_dir = trainer
    latest = paths.CHECKPOINTS / "x" / "latest.ckpt"
    latest.parent.mkdir(parents=True)
    latest.write_bytes(b"ckpt")
    assert cli.main(argv_for(train_dir)) == proc.RUNTIME
    assert cli.main(argv_for(train_dir, "d.psv", "--adopt")) == proc.OK
    assert "--resume" in calls[0]
