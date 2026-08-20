"""コマンド行の解析と振り分けを検証する。

実際にビルドや対局を起こす経路は副作用が大きいので触らない。`--dry-run` が
組み立てるコマンド列を固定する。**このCLIの価値は同じ入力から同じコマンドが
出ることなので、そこを測る。**
"""

import pytest

from hmwr import cli, paths, proc


def dry(capsys, argv):
    """--dry-run で走らせ、表示された行を返す。"""
    code = cli.main(["--dry-run", *argv])
    out = capsys.readouterr().out
    return code, [line for line in out.splitlines() if line.startswith("[dry-run]")]


# --- 入口 --------------------------------------------------------------


def test_no_command_prints_help(capsys):
    assert cli.main([]) == proc.USAGE
    assert "himawari-rs の開発コマンド" in capsys.readouterr().out


def test_area_without_operation_prints_area_help(capsys):
    """領域だけ渡したら、その領域のヘルプを出す。全体のヘルプではない。"""
    assert cli.main(["sprt"]) == proc.USAGE
    out = capsys.readouterr().out
    assert "hmwr sprt" in out
    assert "run" in out


def test_help_does_not_mention_adr_numbers():
    """ヘルプは利用者のためのもので、設計記録の番号は出さない。"""
    import io
    from contextlib import redirect_stdout

    parser = cli.build_parser()
    buf = io.StringIO()
    with redirect_stdout(buf):
        parser.print_help()
        for action in parser._actions:
            choices = getattr(action, "choices", None)
            if isinstance(choices, dict):
                for sub in choices.values():
                    sub.print_help()
                    for a in sub._actions:
                        deeper = getattr(a, "choices", None)
                        if isinstance(deeper, dict):
                            for leaf in deeper.values():
                                leaf.print_help()
    assert "ADR" not in buf.getvalue()


def test_every_area_has_a_handler_or_children():
    """サブコマンドを足したときにハンドラを付け忘れないようにする。"""
    parser = cli.build_parser()
    for action in parser._actions:
        choices = getattr(action, "choices", None)
        if not isinstance(choices, dict):
            continue
        for name, sub in choices.items():
            has_func = sub.get_default("func") is not None
            has_children = any(
                isinstance(getattr(a, "choices", None), dict) for a in sub._actions
            )
            assert has_func or has_children, f"{name} にハンドラがない"


# --- 名前の検証 --------------------------------------------------------


@pytest.mark.parametrize(
    "name", ["adr0180-cli", "pairprod_2990M_q1", "shape-256x16", "x", "net.v1"]
)
def test_valid_names_pass(name):
    assert paths.check_name(name) == name


@pytest.mark.parametrize(
    "name", ["", "-leading", "with space", "slash/inside", "../escape", "日本語"]
)
def test_invalid_names_are_rejected(name):
    with pytest.raises(paths.BadName):
        paths.check_name(name)


def test_invalid_name_returns_usage_code(capsys):
    assert cli.main(["sprt", "run", "bad name"]) == proc.USAGE
    assert "実験名に使えない文字" in capsys.readouterr().err


# --- 置き場 ------------------------------------------------------------


def test_log_path_has_area_prefix():
    """ログ名は呼び出し側でなくCLIが決める。"""
    assert paths.log("sprt", "adr0180").name == "sprt-adr0180.log"
    assert paths.log("bench", "adr0180").name == "bench-adr0180.log"
    assert paths.log("verify", "x").parent.name == "logs"


def test_log_path_validates_name():
    with pytest.raises(paths.BadName):
        paths.log("sprt", "../escape")


def test_rel_strips_repo_prefix():
    assert paths.rel(paths.BIN / "x") == "data/bin/x"
    assert paths.rel("/elsewhere/x") == "/elsewhere/x"


def test_pad_counts_fullwidth_as_two():
    assert paths.pad("完了", 6) == "完了" + " " * 2
    assert paths.pad("done", 6) == "done" + " " * 2


# --- sprt --------------------------------------------------------------


def test_sprt_run_builds_verifies_then_starts(capsys):
    """順番を固定する。機能検証を飛ばせない形にすることが目的である。"""
    code, lines = dry(capsys, ["sprt", "run", "adr0180-x"])
    assert code == proc.OK
    joined = "\n".join(lines)
    # ビルド → 機能検証 → 起動 の順で並ぶ
    build_at = next(i for i, x in enumerate(lines) if "cargo build" in x)
    verify_at = next(i for i, x in enumerate(lines) if "--bin verify" in x)
    start_at = next(i for i, x in enumerate(lines) if "--worker" in x)
    assert build_at < verify_at < start_at
    assert "git checkout origin/main -- crates/" in joined
    assert "sprt run adr0180-x --worker" in lines[start_at]


def test_build_pair_restores_the_working_tree(capsys):
    """比較元を作った後、必ず作業木を戻す。"""
    _, lines = dry(capsys, ["build", "pair", "adr0180-c"])
    joined = "\n".join(lines)
    assert "git checkout origin/main -- crates/" in joined
    assert "git checkout HEAD -- crates/" in joined
    assert lines[-1].endswith("git checkout HEAD -- crates/")


def test_build_pair_uses_the_measurement_flags(capsys):
    _, lines = dry(capsys, ["build", "pair", "adr0180-c"])
    assert all("RUSTFLAGS=-C target-cpu=native" in x for x in lines if "cargo build" in x)


def test_build_shapes_separates_target_directories(capsys):
    """構成ごとに出力先を分ける。1つを使い回すと毎回全体が再コンパイルされる。"""
    _, lines = dry(capsys, ["build", "shapes", "256x16", "512x16x32"])
    assert any("CARGO_TARGET_DIR=target/shape/256x16" in x for x in lines)
    assert any("CARGO_TARGET_DIR=target/shape/512x16x32" in x for x in lines)


def test_build_shapes_rejects_a_malformed_spec(capsys):
    assert cli.main(["--dry-run", "build", "shapes", "256"]) == proc.USAGE
    assert "構成の書き方が違う" in capsys.readouterr().err


def test_build_shapes_resizes_from_a_source_net(capsys):
    _, lines = dry(capsys, ["build", "shapes", "256x16", "--from", "Cargo.toml"])
    makenet = [x for x in lines if "release/makenet" in x][0]
    assert "--resize Cargo.toml" in makenet
    # --from を付けると既定の名前が変わる。元ネットごとの結果を混ぜない
    assert "data/nets/exp-256x16.hmwr" in makenet


def test_sprt_run_no_verify_skips_verification(capsys):
    _, lines = dry(capsys, ["sprt", "run", "adr0180-x", "--no-verify"])
    assert not any("--bin verify" in line for line in lines)


def test_sprt_run_noninferiority_sets_hypothesis(capsys):
    _, lines = dry(capsys, ["sprt", "run", "adr0180-x", "--noninferiority"])
    start = [line for line in lines if "--worker" in line][0]
    assert "SPRT_ELO0=-5" in start
    assert "SPRT_ELO1=0" in start


def test_sprt_run_passes_tc_and_settings(capsys):
    _, lines = dry(
        capsys,
        ["sprt", "run", "adr0180-x", "--tc", "60+0.6", "--set", "SPRT_MAX_PAIRS=100"],
    )
    start = [line for line in lines if "--worker" in line][0]
    assert "SPRT_TC=60+0.6" in start
    assert "SPRT_MAX_PAIRS=100" in start


def test_sprt_run_foreground_builds_the_selfplay_command(capsys):
    """切り離さない経路では、対局のコマンドをその場で組み立てる。"""
    _, lines = dry(
        capsys,
        ["sprt", "run", "adr0180-x", "--foreground", "--no-verify", "--tc", "60+0.6"],
    )
    play = [line for line in lines if "selfplay" in line][0]
    assert "--baseline data/bin/base-adr0180-x" in play
    assert "--candidate data/bin/cand-adr0180-x" in play
    assert "--tc 60+0.6" in play
    assert "--out data/sprt/adr0180-x.jsonl" in play


def test_sprt_net_passes_evaluation_files_per_side(capsys):
    """評価関数は片側ずつ渡す。--option と併用しない。"""
    _, lines = dry(
        capsys,
        ["sprt", "net", "Cargo.toml", "Cargo.lock", "adr0180-n", "--foreground"],
    )
    play = [line for line in lines if "selfplay" in line][0]
    assert "--bopt EvalFile=" in play
    assert "--copt EvalFile=" in play
    assert "--option EvalFile=" not in play


def test_sprt_run_rejects_malformed_setting(capsys):
    code = cli.main(["--dry-run", "sprt", "run", "adr0180-x", "--set", "NOEQUALS"])
    assert code == proc.USAGE
    assert "KEY=VALUE" in capsys.readouterr().err


def test_sprt_run_short_circuits_when_decided(tmp_path, monkeypatch, capsys):
    """判定済みなら何も起動しない。"""
    sprt_dir = tmp_path / "data" / "sprt"
    sprt_dir.mkdir(parents=True)
    (sprt_dir / "done.result").write_text("name=done\ndecision=H1\n", encoding="utf-8")
    monkeypatch.setattr(paths, "SPRT", sprt_dir)
    monkeypatch.setattr(paths, "BIN", tmp_path / "data" / "bin")

    assert cli.main(["sprt", "run", "done"]) == proc.OK
    out = capsys.readouterr().out
    assert "判定済み" in out
    assert "decision=H1" in out


# --- verify / bench ----------------------------------------------------


def test_verify_with_name_expands_to_pair(capsys):
    """引数1つでファイルでなければ実験名とみなす。"""
    _, lines = dry(capsys, ["verify", "adr0180-x"])
    assert "data/bin/base-adr0180-x data/bin/cand-adr0180-x" in lines[0]


def test_verify_with_two_binaries_passes_through(capsys):
    _, lines = dry(capsys, ["verify", "Cargo.toml", "Cargo.lock"])
    assert "Cargo.toml Cargo.lock" in lines[0]


def test_verify_carries_measurement_environment(capsys):
    """評価関数の場所を明示しなくても渡る。"""
    _, lines = dry(capsys, ["verify", "adr0180-x"])
    assert "RUSTFLAGS=" in lines[0]


def test_bench_forwards_options(capsys):
    _, lines = dry(capsys, ["bench", "a", "b", "--nodes", "5000", "--runs", "3"])
    assert "--nodes 5000" in lines[0]
    assert "--runs 3" in lines[0]


# --- net ---------------------------------------------------------------


def test_net_train_folds_flags_into_environment(capsys):
    """フラグを環境変数へ畳むのがCLIの仕事である。"""
    _, lines = dry(
        capsys,
        [
            "net", "train", "ft1024_300M_q1",
            "--data", "data/train/t.psv",
            "--lr", "1e-4",
            "--seed", "3",
            "--device", "cpu",
            "--extra=--mirror-factor",
        ],
    )
    line = lines[0]
    assert "TRAIN_PEAK_LR=1e-4" in line
    assert "TRAIN_SEED=3" in line
    assert "TRAIN_DEVICE=cpu" in line
    assert "TRAIN_EXTRA_ARGS=--mirror-factor" in line
    assert line.endswith("train-net.sh ft1024_300M_q1 data/train/t.psv")


def test_net_train_requires_data():
    with pytest.raises(SystemExit) as e:
        cli.main(["net", "train", "x"])
    assert e.value.code == 2


def test_net_eval_passes_valid_sets(capsys):
    _, lines = dry(capsys, ["net", "eval", "a.hmwr", "--valid", "v1.psv,v2.psv"])
    assert "EVAL_VALIDS=v1.psv,v2.psv" in lines[0]


def test_net_release_is_dry_by_default(capsys):
    """外から見える操作は既定で実行しない。"""
    _, lines = dry(capsys, ["net", "release", "x.hmwr", "5"])
    assert "--apply" not in lines[0]


def test_net_release_apply_is_explicit(capsys):
    _, lines = dry(capsys, ["net", "release", "x.hmwr", "5", "--apply"])
    assert lines[0].endswith("--apply")


# --- data --------------------------------------------------------------


def test_data_quiet_folds_options(capsys):
    _, lines = dry(capsys, ["data", "quiet", "in.psv", "out.psv", "--max-plies", "16"])
    assert "QUIET_MAX_PLIES=16" in lines[0]


def test_data_fetch_defaults_to_all(capsys):
    _, lines = dry(capsys, ["data", "fetch"])
    assert lines[0].endswith("fetch-dataset.sh all")


# --- 設定 --------------------------------------------------------------


def test_measure_env_prefers_explicit_value(monkeypatch):
    """呼び出し側が明示した値を上書きしない。"""
    from hmwr import config

    monkeypatch.setenv("EVAL_FILE", "/mine.hmwr")
    assert "EVAL_FILE" not in config.measure_env()


def test_measure_env_includes_rustflags():
    from hmwr import config

    assert config.measure_env()["RUSTFLAGS"]
