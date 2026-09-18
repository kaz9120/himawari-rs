"""焦点のprobe（ADR-0213）を検証する。

見るのは2つ。`hmwr net probe` が組み立てるコマンド列と、的中率の計算である。
学習は回さない。学習器側の検査はtorchとhimawari拡張が要るので、入っている
環境でだけ走る。
"""

import sys

import pytest

from hmwr import cli, config, paths, proc
from hmwr.tools import focus_labels as fl


def dry(capsys, argv):
    """--dry-run で走らせ、学習器を起動する行を返す。"""
    code = cli.main(["--dry-run", *argv])
    out = capsys.readouterr().out
    lines = [x for x in out.splitlines() if x.startswith("[dry-run] python3")]
    return code, lines[0] if lines else ""


# --- コマンド列 --------------------------------------------------------


def test_probe_freezes_the_ft_and_learns_only_the_focus_head(capsys):
    """probeの条件は3つ揃って意味を持つ。FTの凍結、評価値の切り離し、焦点の的。"""
    code, line = dry(capsys, ["net", "probe", "x", "--focus", "focus_1M"])
    assert code == proc.OK
    assert "--freeze-ft" in line
    assert "--lambda-value 0" in line
    assert "--focus-head linear" in line
    assert f"--data data/train/focus_1M{fl.SUFFIX}" in line


def test_probe_reads_the_current_net_by_default(capsys):
    _, line = dry(capsys, ["net", "probe", "x", "--focus", "focus_1M"])
    assert f"--init-net {config.EVAL_FILE}" in line


def test_probe_with_none_leaves_the_ft_random(capsys):
    """自明解Rは学習した表現を何も読まない。"""
    _, line = dry(
        capsys, ["net", "probe", "x", "--focus", "focus_1M", "--init-net", "none"]
    )
    assert "--init-net" not in line
    assert "--freeze-ft" in line


def test_probe_resolves_a_net_name_to_its_place(capsys):
    _, line = dry(
        capsys, ["net", "probe", "x", "--focus", "focus_1M", "--init-net", "other"]
    )
    assert "--init-net data/nets/other.hmwr" in line


def test_probe_logs_under_its_area(capsys):
    code = cli.main(["--dry-run", "net", "probe", "x", "--focus", "focus_1M"])
    assert code == proc.OK
    assert "data/logs/probe-x.log" in capsys.readouterr().out


def test_probe_reports_ten_times_within_the_run(capsys, tmp_path, monkeypatch):
    """既定の刻みは本番規模の値で、90万局面では検証が1回も出ない。"""
    data = tmp_path / "small.focus"
    data.write_bytes(b"\0" * (fl.RECORD_BYTES * 30000))
    _, line = dry(
        capsys,
        ["net", "probe", "x", "--focus", str(data), "--valid-count", "10000",
         "--batch", "1000"],
    )  # fmt: skip
    # 学習に残るのは2万局面。バッチ1000で20ステップなので、2ステップおき
    assert "--valid-interval 2" in line
    assert "--log-interval 2" in line


def test_probe_rejects_a_valid_count_that_leaves_no_training_data(capsys, tmp_path):
    data = tmp_path / "small.focus"
    data.write_bytes(b"\0" * (fl.RECORD_BYTES * 100))
    code = cli.main(
        ["--dry-run", "net", "probe", "x", "--focus", str(data),
         "--valid-count", "100"]
    )  # fmt: skip
    assert code == proc.USAGE
    assert "--valid-count が大きすぎる" in capsys.readouterr().err


# --- 的中率 ------------------------------------------------------------


@pytest.fixture(scope="module")
def trainer_model():
    """学習器のモデルを読む。拡張とtorchが入っていない環境では飛ばす。"""
    pytest.importorskip("torch")
    pytest.importorskip("himawari")
    sys.path.insert(0, str(paths.REPO / "training"))
    import model

    return model


def test_top_k_precision_counts_hits_among_the_top_squares(trainer_model):
    """局面ごとに上位k升を取り、熱地図が1だった割合を平均する。"""
    import torch

    pred = torch.tensor([[5.0, 4.0, 3.0, 2.0, 1.0, 0.0],
                         [0.0, 1.0, 2.0, 3.0, 4.0, 5.0]])
    heat = torch.tensor([[1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
                         [0.0, 0.0, 0.0, 0.0, 1.0, 1.0]])
    # 上の局面は0番と1番を選んで1つ的中、下の局面は5番と4番を選んで2つ的中
    assert trainer_model.top_k_precision(pred, heat, k=2).item() == pytest.approx(0.75)


def test_a_position_independent_prediction_picks_the_same_squares(trainer_model):
    """頻度事前も同じ関数で測る。全局面へ広げれば同じk升が選ばれる。"""
    import torch

    prior = torch.tensor([[0.1, 0.9, 0.5, 0.0, 0.0, 0.0]])
    heat = torch.tensor([[0.0, 1.0, 1.0, 0.0, 0.0, 0.0],
                         [0.0, 1.0, 0.0, 0.0, 0.0, 0.0]])
    got = trainer_model.top_k_precision(prior.expand(2, -1), heat, k=2)
    assert got.item() == pytest.approx(0.75)


def test_perfect_and_empty_predictions_hit_the_ends_of_the_scale(trainer_model):
    import torch

    heat = torch.tensor([[1.0, 1.0, 0.0, 0.0]])
    assert trainer_model.top_k_precision(
        torch.tensor([[9.0, 8.0, 0.0, 0.0]]), heat, k=2
    ).item() == pytest.approx(1.0)
    assert trainer_model.top_k_precision(
        torch.tensor([[0.0, 0.0, 9.0, 8.0]]), heat, k=2
    ).item() == pytest.approx(0.0)


# --- 学習と検証の分割 --------------------------------------------------


def test_the_focus_loader_splits_by_record_range(trainer_model, tmp_path):
    """学習は先頭、検証は末尾。区間はレコード単位で切る。"""
    import numpy as np

    sys.path.insert(0, str(paths.REPO / "training"))
    from dataset import FocusBatchLoader

    rows = 10
    raw = np.zeros((rows, fl.RECORD_BYTES), dtype=np.uint8)
    # 熱地図の先頭の升だけを立て、後ろ半分のレコードで頻度を変える
    raw[:, fl.PSV_BYTES] = 1
    raw[rows // 2 :, fl.PSV_BYTES + 1] = 1
    path = tmp_path / "split.focus"
    path.write_bytes(raw.tobytes())

    head = FocusBatchLoader(str(path), 2, lo=0, hi=rows // 2)
    tail = FocusBatchLoader(str(path), 2, lo=rows // 2, hi=rows)
    assert (head.n, tail.n) == (5, 5)
    assert head.heat_mean()[1] == 0.0
    assert tail.heat_mean()[1] == 1.0


def test_the_focus_loader_turns_the_heat_map_for_the_side_to_move(
    trainer_model, tmp_path
):
    """蒸留器は手番側から見た向きで並ぶので、後手番の熱地図は180度回す。"""
    import numpy as np

    from dataset import FocusBatchLoader

    raw = np.zeros((2, fl.RECORD_BYTES), dtype=np.uint8)
    # 1行目は先手番、2行目は後手番。どちらも0番の升だけを立てる
    raw[1, 0] = 1
    raw[:, fl.PSV_BYTES] = 1
    path = tmp_path / "turn.focus"
    path.write_bytes(raw.tobytes())

    board = FocusBatchLoader(str(path), 2, orient="board").heat_mean()
    stm = FocusBatchLoader(str(path), 2, orient="stm").heat_mean()
    assert (board[0], board[80]) == (1.0, 0.0)
    # 後手番の1件だけが80番へ移る
    assert (stm[0], stm[80]) == (0.5, 0.5)


def test_the_focus_loader_refuses_an_unknown_orientation(trainer_model, tmp_path):
    from dataset import FocusBatchLoader

    path = tmp_path / "x.focus"
    path.write_bytes(b"\0" * fl.RECORD_BYTES)
    with pytest.raises(ValueError, match="向きが不明"):
        FocusBatchLoader(str(path), 2, orient="white")


def test_probe_passes_the_orientation_through(capsys):
    _, line = dry(
        capsys, ["net", "probe", "x", "--focus", "focus_1M", "--orient", "stm"]
    )
    assert "--focus-orient stm" in line


def test_the_focus_loader_refuses_a_file_of_the_wrong_length(trainer_model, tmp_path):
    from dataset import FocusBatchLoader

    path = tmp_path / "broken.focus"
    path.write_bytes(b"\0" * (fl.RECORD_BYTES + 1))
    with pytest.raises(ValueError, match="202の倍数でない"):
        FocusBatchLoader(str(path), 2)
