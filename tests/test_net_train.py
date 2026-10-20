"""`hmwr net train` の完了マーカーと再開を検証する（ADR-0208・ADR-0209）。

学習は回さない。学習器の起動を差し替え、渡された引数だけを見る。
"""

from pathlib import Path

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
    monkeypatch.setattr(net, "_ensure_extension", lambda *_, **__: None)

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


# --- 置き場の解決（#850） -----------------------------------------------


def test_a_name_is_resolved_under_the_train_place(trainer):
    """名前は paths.TRAIN から組む。文字列で組むとテストの退避が効かない（#850）。"""
    calls, _ = trainer
    paths.TRAIN.mkdir(parents=True, exist_ok=True)
    for name in ("d.psv", "v.psv"):
        (paths.TRAIN / name).write_bytes(b"x")

    assert net._train_file("d", ".psv") == paths.TRAIN / "d.psv"
    assert cli.main(["net", "train", "x", "--data", "d", "--valid", "v"]) == proc.OK
    # 学習器へは相対で渡す。学習の記録の備考に載るので、置き場に依存させない
    argv = calls[0]
    assert argv[argv.index("--data") + 1] == "data/train/d.psv"
    assert argv[argv.index("--valid") + 1] == "data/train/v.psv"


def test_a_path_is_passed_through_as_given(trainer):
    """区切りか拡張子があればパスとして通す。名前の解決に巻き込まない。"""
    _, train_dir = trainer
    assert net._train_file(str(train_dir / "d.psv"), ".psv") == train_dir / "d.psv"
    assert net._train_file("data/train/d.psv", ".psv") == Path("data/train/d.psv")


# --- py拡張の構成（#804） ----------------------------------------------


def test_default_arch_is_read_from_build_rs():
    """既定の構成はbuild.rsが持つ。Python側に数を写さない。"""
    text = net.BUILD_RS.read_text(encoding="utf-8")
    assert f'const DEFAULT_ARCH: &str = "{net.default_arch()}";' in text


def test_default_arch_fails_when_build_rs_changes_shape(tmp_path, monkeypatch):
    rs = tmp_path / "build.rs"
    rs.write_text('const OTHER: &str = "1x1";\n', encoding="utf-8")
    monkeypatch.setattr(net, "BUILD_RS", rs)
    with pytest.raises(proc.Fail, match="DEFAULT_ARCH"):
        net.default_arch()


@pytest.fixture
def extension(monkeypatch):
    """入っている拡張の構成を差し替え、ビルドと入れ直しの呼び出しを控える。"""
    state = {"installed": "", "builds": [], "installs": []}
    monkeypatch.setattr(net, "default_arch", lambda: "1024x16x32")
    monkeypatch.setattr(proc, "capture", lambda argv, **_: state["installed"] + "\n")

    def fake_run(argv, **kw):
        state["builds"].append(kw.get("env", {}))
        return proc.OK

    def fake_install(wheels, spec, *, dry_run):
        state["installs"].append(spec)

    monkeypatch.setattr(proc, "run", fake_run)
    monkeypatch.setattr(net, "_install_wheel", fake_install)
    return state


def test_extension_of_the_default_arch_is_kept(extension):
    extension["installed"] = "1024x16x32"
    net._ensure_extension(dry_run=False)
    assert extension["builds"] == [] and extension["installs"] == []


def test_extension_left_by_net_shapes_is_rebuilt_with_the_default(extension, capsys):
    """net shapesが残した別構成の拡張のまま学習しない（#804）。"""
    extension["installed"] = "512x16x64"
    net._ensure_extension(dry_run=False)
    assert [b["HIMAWARI_ARCH"] for b in extension["builds"]] == ["1024x16x32"]
    assert extension["installs"] == ["1024x16x32"]
    assert "512x16x64" in capsys.readouterr().out


def test_missing_extension_is_built_with_the_default(extension):
    net._ensure_extension(dry_run=False)
    assert [b["HIMAWARI_ARCH"] for b in extension["builds"]] == ["1024x16x32"]


def test_dry_run_neither_builds_nor_installs(extension):
    extension["installed"] = "512x16x64"
    net._ensure_extension(dry_run=True)
    assert extension["builds"] == [] and extension["installs"] == []


def test_arch_rebuilds_the_extension_with_that_arch(extension):
    """--archで別構成を指定したら、その構成で作り直す（ADR-0229の段階2の道）。"""
    extension["installed"] = "1024x16x32"
    net._ensure_extension("512x16x64", dry_run=False)
    assert [b["HIMAWARI_ARCH"] for b in extension["builds"]] == ["512x16x64"]
    assert extension["installs"] == ["512x16x64"]


def test_extension_of_the_asked_arch_is_kept(extension):
    extension["installed"] = "512x16x64"
    net._ensure_extension("512x16x64", dry_run=False)
    assert extension["builds"] == [] and extension["installs"] == []


def test_train_passes_the_arch_and_records_it_as_a_condition(trainer, monkeypatch):
    """net train --arch は拡張をその構成にし、構成を学習の条件に数える。"""
    calls, train_dir = trainer
    asked = []
    monkeypatch.setattr(net, "_ensure_extension", lambda arch=None, **_: asked.append(arch))
    assert cli.main(argv_for(train_dir, "d.psv", "--arch", "512x16x64")) == proc.OK
    assert asked == ["512x16x64"]
    record = (paths.CHECKPOINTS / "x" / "train.cond").read_text(encoding="utf-8")
    assert "HIMAWARI_ARCH=512x16x64" in record
    # 同じ名前で既定の構成を頼むと、条件が違うので上書きしない
    assert cli.main(argv_for(train_dir)) == proc.RUNTIME


def test_train_defaults_to_the_build_rs_arch_without_changing_the_record(trainer, monkeypatch):
    """既定の構成では、--archを入れる前と同じ条件の行を控える。"""
    calls, train_dir = trainer
    asked = []
    monkeypatch.setattr(net, "_ensure_extension", lambda arch=None, **_: asked.append(arch))
    assert cli.main(argv_for(train_dir)) == proc.OK
    assert asked == [net.default_arch()]
    record = (paths.CHECKPOINTS / "x" / "train.cond").read_text(encoding="utf-8")
    assert "HIMAWARI_ARCH" not in record


def test_a_malformed_arch_is_a_usage_error(trainer):
    _, train_dir = trainer
    assert cli.main(argv_for(train_dir, "d.psv", "--arch", "big")) == proc.USAGE
