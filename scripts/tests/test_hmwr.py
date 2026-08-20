"""scripts/hmwr の引数処理を検証する（ADR-0179）。

実際にビルドや対局を起こす経路は副作用が大きいので触らない。`--dry-run` が
組み立てるコマンド列と、名前の検証・ログの置き場・終了コードを確かめる。
**このCLIの価値は「同じ入力から同じコマンドが出ること」なので、そこを固定する。**
"""

import pytest
from conftest import load_module

hmwr = load_module("hmwr", "hmwr")


def dry(capsys, argv):
    """--dry-run で走らせ、出力の行を返す。"""
    code = hmwr.main(["--dry-run", *argv])
    out = capsys.readouterr().out
    return code, [line for line in out.splitlines() if line.startswith("[dry-run]")]


# --- 名前の検証 --------------------------------------------------------


@pytest.mark.parametrize(
    "name",
    ["adr0179-cli", "pairprod_2990M_q1", "shape-256x16", "x", "net.v1"],
)
def test_valid_names_pass(name):
    assert hmwr.check_name(name) == name


@pytest.mark.parametrize(
    "name",
    ["", "-leading", "with space", "slash/inside", "../escape", "日本語"],
)
def test_invalid_names_raise_usage_error(name):
    with pytest.raises(hmwr.Fail) as e:
        hmwr.check_name(name)
    assert e.value.code == hmwr.EXIT_USAGE


def test_invalid_name_returns_two(capsys):
    assert hmwr.main(["sprt", "start", "bad name"]) == hmwr.EXIT_USAGE
    assert "実験名に使えない文字" in capsys.readouterr().err


# --- ログの置き場 ------------------------------------------------------


def test_log_path_has_area_prefix():
    """ログ名は呼び出し側でなくCLIが決める（ADR-0149）。"""
    assert hmwr.log_path("sprt", "adr0179").name == "sprt-adr0179.log"
    assert hmwr.log_path("bench", "adr0179").name == "bench-adr0179.log"
    assert hmwr.log_path("verify", "x").parent.name == "logs"


def test_rel_strips_repo_prefix():
    assert hmwr.rel(hmwr.REPO / "data" / "bin" / "x") == "data/bin/x"
    assert hmwr.rel("/elsewhere/x") == "/elsewhere/x"


def test_pad_counts_fullwidth_as_two():
    assert hmwr.pad("完了", 6) == "完了" + " " * 2
    assert hmwr.pad("done", 6) == "done" + " " * 2


# --- sprt --------------------------------------------------------------


def test_sprt_start_runs_build_verify_then_detach(capsys):
    """順番を固定する。verifyを飛ばせない形にすることが目的である。"""
    code, lines = dry(capsys, ["sprt", "start", "adr0179-x"])
    assert code == hmwr.EXIT_OK
    assert "build-pair.sh adr0179-x" in lines[0]
    assert "--bin verify" in lines[1]
    assert "sprt-detach.py" in lines[3]


def test_sprt_start_no_verify_skips_verification(capsys):
    code, lines = dry(capsys, ["sprt", "start", "adr0179-x", "--no-verify"])
    assert code == hmwr.EXIT_OK
    assert not any("--bin verify" in line for line in lines)


def test_sprt_start_noninferiority_sets_hypothesis(capsys):
    """非劣性はADR-0163の elo0=-5 / elo1=0 になる。"""
    _, lines = dry(capsys, ["sprt", "start", "adr0179-x", "--noninferiority"])
    detach = [line for line in lines if "sprt-detach.py" in line][0]
    assert "SPRT_ELO0=-5" in detach
    assert "SPRT_ELO1=0" in detach


def test_sprt_start_tc_and_set_are_passed(capsys):
    _, lines = dry(
        capsys,
        ["sprt", "start", "adr0179-x", "--tc", "60+0.6", "--set", "SPRT_MAX_PAIRS=100"],
    )
    detach = [line for line in lines if "sprt-detach.py" in line][0]
    assert "SPRT_TC=60+0.6" in detach
    assert "SPRT_MAX_PAIRS=100" in detach


def test_sprt_start_rejects_malformed_set(capsys):
    code = hmwr.main(["--dry-run", "sprt", "start", "adr0179-x", "--set", "NOEQUALS"])
    assert code == hmwr.EXIT_USAGE
    assert "KEY=VALUE" in capsys.readouterr().err


def test_sprt_start_short_circuits_when_decided(tmp_path, monkeypatch, capsys):
    """判定済みなら何も起動しない（ADR-0175の冪等性）。"""
    result = tmp_path / "data" / "sprt" / "done.result"
    result.parent.mkdir(parents=True)
    result.write_text("name=done\ndecision=H1\n", encoding="utf-8")
    monkeypatch.setattr(hmwr, "REPO", tmp_path)

    code = hmwr.main(["sprt", "start", "done"])
    out = capsys.readouterr().out
    assert code == hmwr.EXIT_OK
    assert "判定済み" in out
    assert "decision=H1" in out


# --- verify ------------------------------------------------------------


def test_verify_with_name_expands_to_pair(capsys):
    """引数1つでファイルでなければ実験名とみなす。"""
    _, lines = dry(capsys, ["verify", "adr0179-x"])
    assert "data/bin/base-adr0179-x data/bin/cand-adr0179-x" in lines[0]


def test_verify_with_two_binaries_passes_through(capsys):
    _, lines = dry(capsys, ["verify", "Cargo.toml", "Cargo.lock"])
    assert "Cargo.toml Cargo.lock" in lines[0]


def test_verify_uses_native_rustflags(capsys):
    """計測は -C target-cpu=native で行う（ADR-0003）。"""
    _, lines = dry(capsys, ["verify", "adr0179-x"])
    assert "RUSTFLAGS=-C target-cpu=native" in lines[0]


def test_native_env_carries_eval_file(monkeypatch):
    """env.sh を source しなくても評価関数の場所が渡る（ADR-0122）。"""
    monkeypatch.setattr(hmwr, "_SHELL_DEFAULTS", {"EVAL_FILE": "/x/net.hmwr"})
    monkeypatch.delenv("EVAL_FILE", raising=False)
    env = hmwr.native_env()
    assert env["EVAL_FILE"] == "/x/net.hmwr"
    assert env["RUSTFLAGS"] == "-C target-cpu=native"


def test_explicit_eval_file_wins_over_shell_default(monkeypatch):
    """呼び出し側が明示した値を上書きしない。"""
    monkeypatch.setattr(hmwr, "_SHELL_DEFAULTS", {"EVAL_FILE": "/x/net.hmwr"})
    monkeypatch.setenv("EVAL_FILE", "/mine.hmwr")
    assert "EVAL_FILE" not in hmwr.native_env()


# --- train / eval / quiet ----------------------------------------------


def test_train_folds_flags_into_environment(capsys):
    """フラグを環境変数へ畳むのがCLIの仕事である。"""
    _, lines = dry(
        capsys,
        [
            "train",
            "ft1024_300M_q1",
            "--data",
            "data/train/t.psv",
            "--lr",
            "1e-4",
            "--seed",
            "3",
            "--device",
            "cpu",
            # ハイフンで始まる値は = でつなぐ。argparse がオプションと誤読するため
            "--extra=--mirror-factor",
        ],
    )
    line = lines[0]
    assert "TRAIN_PEAK_LR=1e-4" in line
    assert "TRAIN_SEED=3" in line
    assert "TRAIN_DEVICE=cpu" in line
    assert "TRAIN_EXTRA_ARGS=--mirror-factor" in line
    assert line.endswith("train-net.sh ft1024_300M_q1 data/train/t.psv")


def test_train_requires_data():
    with pytest.raises(SystemExit) as e:
        hmwr.main(["train", "x"])
    assert e.value.code == 2


def test_eval_passes_valid_sets(capsys):
    _, lines = dry(capsys, ["eval", "a.hmwr", "b.hmwr", "--valid", "v1.psv,v2.psv"])
    assert "EVAL_VALIDS=v1.psv,v2.psv" in lines[0]
    assert lines[0].endswith("eval-net.sh a.hmwr b.hmwr")


def test_quiet_folds_options(capsys):
    _, lines = dry(capsys, ["quiet", "in.psv", "out.psv", "--max-plies", "16"])
    assert "QUIET_MAX_PLIES=16" in lines[0]


# --- release -----------------------------------------------------------


def test_release_is_dry_by_default(capsys):
    """外から見える操作は既定で実行しない（ADR-0122）。"""
    _, lines = dry(capsys, ["release", "net", "x.hmwr", "5"])
    assert "--apply" not in lines[0]


def test_release_apply_is_explicit(capsys):
    _, lines = dry(capsys, ["release", "net", "x.hmwr", "5", "--apply"])
    assert lines[0].endswith("--apply")


# --- 入口 --------------------------------------------------------------


def test_no_command_prints_help(capsys):
    assert hmwr.main([]) == hmwr.EXIT_USAGE
    assert "himawari-rsの日常操作" in capsys.readouterr().out


def test_every_subcommand_has_a_handler():
    """サブコマンドを足したときにハンドラを付け忘れないようにする。"""
    parser = hmwr.build_parser()
    actions = [a for a in parser._actions if hasattr(a, "choices") and a.choices]
    for action in actions:
        for name, sub in action.choices.items():
            has_func = sub.get_default("func") is not None
            has_children = any(
                hasattr(a, "choices") and a.choices for a in sub._actions
            )
            assert has_func or has_children, f"{name} にハンドラがない"
