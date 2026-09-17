"""`hmwr data` の対応表を検証する（ADR-0208）。

見るのは3つ。表から同じコマンドが出ること、表の綴りが `psv` の実装と
食い違っていないこと、完了印が再実行を正しく裁くこと。`psv` の実物は
走らせず、`--out` へ書くだけの偽物に差し替える。
"""

import re
import stat

import pytest

from hmwr import cli, dataops, paths, proc


def dry(capsys, argv):
    code = cli.main(["--dry-run", "data", *argv])
    out = capsys.readouterr().out
    return code, [line for line in out.splitlines() if line.startswith("[dry-run]")]


# --- 表から出るコマンド ------------------------------------------------


def test_mix_matches_the_adr0206_chain(capsys):
    """ADR-0206のチェーンが直接叩いた行と、出力の `.part` を除いて一致する。"""
    _, lines = dry(capsys, ["mix", "ekmix20_300M", "--in", "train_300M_q1", "--in", "ek_60M_q1"])
    assert lines[0] == (
        "[dry-run] target/release/psv shuffle "
        "--in data/train/train_300M_q1.psv,data/train/ek_60M_q1.psv "
        "--out data/train/ekmix20_300M.psv.part --seed 1"
    )
    assert lines[1] == "[dry-run] mv data/train/ekmix20_300M.psv.part data/train/ekmix20_300M.psv"
    assert lines[2] == "[dry-run] ログ: data/logs/mix-ekmix20_300M.log"


def test_split_maps_to_head(capsys):
    _, lines = dry(capsys, ["split", "t", "--in", "s", "--count", "100", "--skip", "5"])
    assert lines[0].endswith(
        "psv head --in data/train/s.psv --out data/train/t.psv.part --count 100 --skip 5"
    )


def test_quiet_passes_the_settled_defaults(capsys, monkeypatch):
    """並列数は出力を変えるので、既定を必ず明示して渡す。"""
    monkeypatch.delenv("EVAL_FILE", raising=False)
    _, lines = dry(capsys, ["quiet", "t_q1", "--in", "t"])
    assert "--max-plies 1" in lines[0]
    assert f"--jobs {dataops.QUIET_JOBS}" in lines[0]
    assert "--eval-file data/nets/" in lines[0]


def test_rank_splits_by_absolute_position(capsys):
    _, lines = dry(
        capsys, ["rank", "r", "--in", "t", "--skip", "200", "--limit", "1000", "--jobs", "3"]
    )
    runs = [x for x in lines if " rank " in x]
    assert [re.search(r"--skip (\d+) --limit (\d+)", x).groups() for x in runs] == [
        ("200", "334"),
        ("534", "334"),
        ("868", "332"),
    ]
    assert "data/train/r.rankpsv.part000" in runs[0]
    assert any("mv data/train/r.rankpsv.part data/train/r.rankpsv" in x for x in lines)


def test_rank_jobs_needs_limit():
    assert cli.main(["--dry-run", "data", "rank", "r", "--in", "t", "--jobs", "4"]) == proc.USAGE


def test_mix_needs_two_inputs():
    assert cli.main(["--dry-run", "data", "mix", "m", "--in", "a"]) == proc.USAGE


def test_inputs_are_names_not_paths():
    assert cli.main(["--dry-run", "data", "split", "t", "--in", "data/train/s.psv", "--count", "1"]) == proc.USAGE


def test_shuffle_reads_a_raw_dataset(capsys):
    _, lines = dry(capsys, ["shuffle", "s", "--raw", "no-such-dataset"])
    assert "--in data/raw/no-such-dataset/*.bin" in lines[0]


# --- 表とpsvの食い違い -------------------------------------------------


def _psv_arm(sub: str) -> str:
    """psv.rs の引数解析から、サブコマンド1つ分の節を取り出す。"""
    src = (paths.REPO / "crates" / "tools" / "src" / "bin" / "psv.rs").read_text()
    m = re.search(rf'\n        "{sub}" => \{{(.*?)\n        \}}', src, re.S)
    assert m, f"psv.rs に {sub} の節がない"
    return m.group(1)


@pytest.mark.parametrize("op", dataops.OPS, ids=lambda op: op.name)
def test_every_flag_exists_in_psv(op):
    arm = _psv_arm(op.psv)
    for o in op.opts:
        assert f'"{o.flag}"' in arm, f"psv {op.psv} は {o.flag} を受けない"


# --- 完了印 ------------------------------------------------------------


@pytest.fixture
def fake(tmp_path, monkeypatch):
    """`--out` の先へ `--skip` の値を書くだけの偽psv。"""
    script = tmp_path / "psv"
    script.write_text(
        "#!/bin/sh\n"
        'out=""; skip="-"\n'
        'while [ $# -gt 0 ]; do\n'
        '  case "$1" in --out) out="$2";; --skip) skip="$2";; esac; shift\n'
        "done\n"
        'echo "$skip" > "$out"\n'
    )
    script.chmod(script.stat().st_mode | stat.S_IXUSR)
    train = tmp_path / "train"
    train.mkdir()
    (train / "s.psv").write_bytes(b"x")
    monkeypatch.setattr(paths, "TRAIN", train)
    monkeypatch.setattr(paths, "LOGS", tmp_path / "logs")
    monkeypatch.setattr(dataops, "psv_bin", lambda: script)
    return train


def test_done_mark_skips_the_same_conditions(fake, capsys):
    argv = ["data", "split", "t", "--in", "s", "--count", "10"]
    assert cli.main(argv) == proc.OK
    assert (fake / "t.psv").is_file() and (fake / "t.psv.done").is_file()
    assert not (fake / "t.psv.part").exists()
    capsys.readouterr()
    assert cli.main(argv) == proc.OK
    assert "済み" in capsys.readouterr().out


def test_done_mark_refuses_other_conditions(fake):
    assert cli.main(["data", "split", "t", "--in", "s", "--count", "10"]) == proc.OK
    assert cli.main(["data", "split", "t", "--in", "s", "--count", "20"]) == proc.RUNTIME
    assert cli.main(["data", "split", "t", "--in", "s", "--count", "20", "--force"]) == proc.OK
    assert cli.main(["data", "split", "t", "--in", "s", "--count", "20"]) == proc.OK


def test_output_without_a_done_mark_is_not_trusted(fake):
    (fake / "t.psv").write_bytes(b"legacy")
    assert cli.main(["data", "split", "t", "--in", "s", "--count", "10"]) == proc.RUNTIME
    assert (fake / "t.psv").read_bytes() == b"legacy"


def test_missing_input_fails(fake):
    assert cli.main(["data", "split", "t", "--in", "nope", "--count", "10"]) == proc.RUNTIME


def test_rank_pieces_are_joined_in_order(fake):
    argv = ["data", "rank", "r", "--in", "s", "--limit", "90", "--jobs", "3"]
    assert cli.main(argv) == proc.OK
    assert (fake / "r.rankpsv").read_text().split() == ["0", "30", "60"]
    assert list(fake.glob("r.rankpsv.part*")) == []
