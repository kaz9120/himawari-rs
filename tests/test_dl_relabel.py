"""DL系モデルによる付け直し（ADR-0215）を検証する。

モデルは走らせない。勝率→評価値の変換、scoreの2バイト以外を触らないこと、
完了印の扱いを、偽のラベラーで確かめる。
"""

import json
import struct

import numpy as np
import pytest

from hmwr import cli, dataops, paths, proc
from hmwr.tools import dl_relabel


def test_to_score_is_the_inverse_of_the_trainer_sigmoid():
    p = np.array([0.5, 1 / (1 + np.exp(-1.0)), 1 / (1 + np.exp(1.0))])
    assert dl_relabel.to_score(p, 600.0).tolist() == [0, 600, -600]
    assert dl_relabel.to_score(p, 300.0).tolist() == [0, 300, -300]


def test_to_score_saturates_at_zero_and_one():
    """0と1はEPSで飽和させ、±30000を超えない。"""
    p = np.array([0.0, 1.0, 1e-9, 1 - 1e-9, dl_relabel.EPS])
    got = dl_relabel.to_score(p, 600.0)
    assert got.dtype == np.int16
    assert got[0] == got[2] == got[4] < 0
    assert got[1] == got[3] == -got[0]
    assert dl_relabel.to_score(p, 5000.0).tolist()[:2] == [-30000, 30000]


def records(scores):
    return b"".join(
        bytes([i % 251] * 32) + struct.pack("<h", s) + struct.pack("<HHbB", 7, 40 + i, 1, 0)
        for i, s in enumerate(scores)
    )


def test_relabel_rewrites_only_the_score(tmp_path):
    src = tmp_path / "s.psv"
    dst = tmp_path / "t.psv"
    src.write_bytes(records([100, -200, 30000]))

    values = iter([np.array([0.5, 0.25]), np.array([0.75])])

    def labeler(rows):
        assert rows.shape[1] == 40
        return next(values)

    stats = dl_relabel.relabel(src, dst, labeler, scale=600.0, batch=2, report=lambda _: None)
    out = dst.read_bytes()
    assert len(out) == 120
    for i, want in enumerate([0, -659, 659]):
        row = out[i * 40 : (i + 1) * 40]
        assert row[:32] == bytes([i % 251] * 32)
        assert struct.unpack_from("<h", row, 32)[0] == want
        assert struct.unpack_from("<HHbB", row, 34) == (7, 40 + i, 1, 0)
    assert stats["positions"] == 3
    assert stats["scale"] == 600.0
    assert stats["mean_abs_new"] == pytest.approx((0 + 659 + 659) / 3, abs=0.1)


def test_relabel_limit_stops_early(tmp_path):
    src = tmp_path / "s.psv"
    dst = tmp_path / "t.psv"
    src.write_bytes(records([1, 2, 3, 4]))
    stats = dl_relabel.relabel(
        src, dst, lambda rows: np.full(len(rows), 0.5), limit=3, report=lambda _: None
    )
    assert stats["positions"] == 3
    assert dst.stat().st_size == 120


@pytest.fixture
def train(tmp_path, monkeypatch):
    train = tmp_path / "train"
    train.mkdir()
    (train / "s.psv").write_bytes(records([100, -100]))
    monkeypatch.setattr(paths, "TRAIN", train)
    monkeypatch.setattr(paths, "LOGS", tmp_path / "logs")
    monkeypatch.setattr(paths, "STATUS", tmp_path / "status")
    monkeypatch.setattr(
        dataops, "make_labeler", lambda args: (lambda rows: np.full(len(rows), 0.5))
    )
    return train


def test_cli_relabel_writes_output_and_done_mark(train):
    argv = ["data", "relabel", "t", "--in", "s", "--scale", "430"]
    assert cli.main(argv) == proc.OK
    out = (train / "t.psv").read_bytes()
    assert struct.unpack_from("<h", out, 32)[0] == 0
    done = json.loads((train / "t.psv.done").read_text())
    assert done["commands"] == [
        "relabel --labeler dlshogi --model data/models/dlshogi/model-dr2_exhi.onnx"
        f" --scale 430 --in {paths.rel(train / 's.psv')}"
    ]
    assert done["stats"]["positions"] == 2
    # 同じ条件なら済み、違う条件なら止める
    assert cli.main(argv) == proc.OK
    assert cli.main(["data", "relabel", "t", "--in", "s", "--scale", "600"]) == proc.RUNTIME


def test_cli_relabel_dry_run(capsys):
    assert cli.main(["--dry-run", "data", "relabel", "t", "--in", "s"]) == proc.OK
    lines = [x for x in capsys.readouterr().out.splitlines() if x.startswith("[dry-run]")]
    assert lines[0].startswith("[dry-run] relabel --labeler dlshogi")
    assert lines[1] == "[dry-run] mv data/train/t.psv.part data/train/t.psv"
    assert lines[2] == "[dry-run] ログ: data/logs/relabel-t.log"


def test_rescale_labeler_shrinks_scores_by_the_ratio(tmp_path):
    src = tmp_path / "s.psv"
    dst = tmp_path / "t.psv"
    src.write_bytes(records([600, -1200, 8289, 0]))
    dl_relabel.relabel(src, dst, dl_relabel.RescaleLabeler(), scale=300.0, report=lambda _: None)
    out = dst.read_bytes()
    got = [struct.unpack_from("<h", out, i * 40 + 32)[0] for i in range(4)]
    assert got == [300, -600, 4144, 0]


def test_cli_rescale_runs_without_a_model(tmp_path, monkeypatch):
    """rescale はモデルを読まないので、モデルの無い環境でも通る。"""
    train = tmp_path / "train"
    train.mkdir()
    (train / "s.psv").write_bytes(records([600, -600]))
    monkeypatch.setattr(paths, "TRAIN", train)
    monkeypatch.setattr(paths, "LOGS", tmp_path / "logs")
    monkeypatch.setattr(paths, "STATUS", tmp_path / "status")
    monkeypatch.setattr(paths, "REPO", tmp_path)  # モデルの既定パスが存在しない
    argv = ["data", "relabel", "t", "--in", "s", "--labeler", "rescale", "--scale", "300"]
    assert cli.main(argv) == proc.OK
    out = (train / "t.psv").read_bytes()
    assert [struct.unpack_from("<h", out, i * 40 + 32)[0] for i in range(2)] == [300, -300]
    done = json.loads((train / "t.psv.done").read_text())
    assert done["commands"][0].startswith("relabel --labeler rescale --scale 300 --in ")


def test_in_place_rewrites_scores_keeps_a_sidecar_and_resumes(tmp_path):
    """その場の書き換えは、元のscoreをsidecarへ控え、進み具合から続けられる。"""
    src = tmp_path / "s.psv"
    src.write_bytes(records([100, -200, 300, -400, 500]))
    rec = {"labeler": "rescale", "model": None, "scale": 300.0}
    # 先に2件だけ書き換えたところで止まった状態を作る
    state = dl_relabel.relabel_in_place(
        src, dl_relabel.RescaleLabeler(), scale=300.0, start=1, count=2, record=rec,
        batch=1, report=lambda _: None,
    )
    assert state["done"] == 2 and state["finished"]
    out = src.read_bytes()
    got = [struct.unpack_from("<h", out, i * 40 + 32)[0] for i in range(5)]
    assert got == [100, -100, 150, -400, 500]
    side = dl_relabel.sidecar_path(src).read_bytes()
    assert struct.unpack_from("<hh", side, 2) == (-200, 300)
    # 同じ条件で呼び直すと済みで何もしない
    again = dl_relabel.relabel_in_place(
        src, dl_relabel.RescaleLabeler(), scale=300.0, start=1, count=2, record=rec,
        report=lambda _: None,
    )
    assert again["done"] == 2
    assert src.read_bytes() == out
    # 条件が違えば止まる
    with pytest.raises(ValueError):
        dl_relabel.relabel_in_place(
            src, dl_relabel.RescaleLabeler(), scale=600.0, start=1, count=2, record={**rec, "scale": 600.0},
            report=lambda _: None,
        )


def test_in_place_resumes_from_progress(tmp_path, monkeypatch):
    src = tmp_path / "s.psv"
    src.write_bytes(records([600, 600, 600, 600]))
    rec = {"labeler": "rescale", "model": None, "scale": 300.0}
    calls = []

    def labeler(rows):
        calls.append(len(rows))
        if len(calls) == 2:
            raise RuntimeError("落ちた")
        return dl_relabel.RescaleLabeler()(rows)

    with pytest.raises(RuntimeError):
        dl_relabel.relabel_in_place(src, labeler, scale=300.0, start=0, count=4, record=rec,
                                    batch=2, report=lambda _: None)
    prog = json.loads(dl_relabel.progress_path(src).read_text())
    assert prog["done"] == 2 and prog["sidecar_done"] == 4
    state = dl_relabel.relabel_in_place(src, dl_relabel.RescaleLabeler(), scale=300.0, start=0, count=4,
                                        record=rec, batch=2, report=lambda _: None)
    assert state["done"] == 4
    out = src.read_bytes()
    assert [struct.unpack_from("<h", out, i * 40 + 32)[0] for i in range(4)] == [300] * 4
    side = dl_relabel.sidecar_path(src).read_bytes()
    assert struct.unpack_from("<hhhh", side, 0) == (600, 600, 600, 600)


def test_cli_in_place_dry_run(capsys):
    assert cli.main(["--dry-run", "data", "relabel", "t", "--in-place", "--start", "0", "--count", "10"]) == proc.OK
    out = capsys.readouterr().out
    assert "その場で書き換える" in out and "scores-before.i16" in out
