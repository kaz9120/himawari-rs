"""DL系モデルの推論1回の評価値で、psvのscoreを付け直す（ADR-0215）。

局面・指し手・勝敗・手数はそのまま残し、scoreの2バイトだけを書き換える。
モデルは勝率pを返すので、学習器の勝率変換 `sigmoid(cp / 600)` の逆で
`cp = scale × logit(p)` へ戻す。scaleを600から下げることは、推論側の
`FV_SCALE` を上げることと、λの評価値部分では等価になる。

推論の裏側は差し替えられる。いまはdlshogiのONNX（cshogiで入力特徴を作り、
onnxruntimeで推論する）だけがある。cshogiとonnxruntimeは学習の依存には
入れず、使うときだけ読み込む。
"""

from __future__ import annotations

import json
import math
import os
import time
from pathlib import Path
from typing import Callable, Protocol

import numpy as np

PSV_BYTES = 40
SCORE_OFFSET = 32  # i16、リトルエンディアン
SCORE_LIMIT = 30000  # mateの飽和。psv relabel（ADR-0205）と揃える
DEFAULT_SCALE = 600.0  # training/model.py の SIGMOID_SCALE
DEFAULT_MODEL = "data/models/dlshogi/model-dr2_exhi.onnx"
EPS = 1e-6


class Labeler(Protocol):
    def __call__(self, records: np.ndarray) -> np.ndarray:
        """psvのレコード（N×40のuint8）に、手番側の勝率（N、float）を返す。"""


def to_score(p: np.ndarray, scale: float) -> np.ndarray:
    """勝率をscoreへ戻す。0と1は飽和させ、±SCORE_LIMITで止める。"""
    q = np.clip(p.astype(np.float64), EPS, 1.0 - EPS)
    cp = scale * np.log(q / (1.0 - q))
    return np.clip(np.rint(cp), -SCORE_LIMIT, SCORE_LIMIT).astype(np.int16)


def relabel(
    src: Path,
    dst: Path,
    labeler: Labeler,
    *,
    scale: float = DEFAULT_SCALE,
    batch: int = 4096,
    limit: int | None = None,
    report: Callable[[str], None] = print,
) -> dict:
    """srcのscoreを付け直してdstへ書く。元と新しいscoreの相関と規模を返す。"""
    total = src.stat().st_size // PSV_BYTES
    if limit is not None:
        total = min(total, limit)
    started = time.time()
    done = 0
    last = started
    # 元のscoreと新しいscoreの相関を、1パスで積む
    n = 0.0
    sx = sy = sxx = syy = sxy = 0.0
    abs_x = abs_y = 0.0
    with open(src, "rb") as fin, open(dst, "wb") as fout:
        while done < total:
            want = min(batch, total - done)
            chunk = np.frombuffer(fin.read(want * PSV_BYTES), dtype=np.uint8)
            if chunk.size == 0:
                break
            rows = chunk.reshape(-1, PSV_BYTES).copy()
            old = rows[:, SCORE_OFFSET : SCORE_OFFSET + 2].copy().view("<i2").ravel()
            new = to_score(labeler(rows), scale)
            rows[:, SCORE_OFFSET : SCORE_OFFSET + 2] = new.astype("<i2").view(np.uint8).reshape(-1, 2)
            fout.write(rows.tobytes())
            x = old.astype(np.float64)
            y = new.astype(np.float64)
            n += len(x)
            sx += x.sum()
            sy += y.sum()
            sxx += (x * x).sum()
            syy += (y * y).sum()
            sxy += (x * y).sum()
            abs_x += np.abs(x).sum()
            abs_y += np.abs(y).sum()
            done += len(rows)
            now = time.time()
            if now - last >= 60:
                rate = done / (now - started)
                eta = (total - done) / rate if rate > 0 else float("inf")
                report(f"{done:,}/{total:,} 局面 {rate:,.0f} 局面/秒 残り {eta / 3600:.1f} 時間")
                last = now
    elapsed = time.time() - started
    cov = sxy / n - (sx / n) * (sy / n) if n else 0.0
    vx = sxx / n - (sx / n) ** 2 if n else 0.0
    vy = syy / n - (sy / n) ** 2 if n else 0.0
    corr = cov / math.sqrt(vx * vy) if vx > 0 and vy > 0 else float("nan")
    return {
        "positions": done,
        "seconds": round(elapsed, 1),
        "positions_per_second": round(done / elapsed) if elapsed > 0 else 0,
        "scale": scale,
        "corr_old_new": round(corr, 4),
        "mean_abs_old": round(abs_x / n, 1) if n else 0.0,
        "mean_abs_new": round(abs_y / n, 1) if n else 0.0,
    }


class RescaleLabeler:
    """既存のscoreを勝率へ戻すだけのラベラー。推論はしない。

    `to_score(p, S)` と組むと、scoreが `S / 600` 倍になる。スケールの水準を
    振るとき、推論を1回で済ませるために使う。飽和（±8289）はそのまま比率で縮む。
    """

    device = "none"

    def __init__(self, scale: float = DEFAULT_SCALE):
        self.scale = scale

    def __call__(self, records: np.ndarray) -> np.ndarray:
        old = records[:, SCORE_OFFSET : SCORE_OFFSET + 2].copy().view("<i2").ravel()
        return 1.0 / (1.0 + np.exp(-old.astype(np.float64) / self.scale))


# --- その場で書き換える ----------------------------------------------------


def progress_path(path: Path) -> Path:
    return path.with_name(path.name + ".relabel.json")


def sidecar_path(path: Path) -> Path:
    """書き換える前のscoreを控える場所。レコードの通し番号×2バイトの位置に置く。"""
    return path.with_name(path.name + ".scores-before.i16")


def _fsync(fh) -> None:
    fh.flush()
    os.fsync(fh.fileno())


def relabel_in_place(
    path: Path,
    labeler: Labeler,
    *,
    scale: float,
    start: int,
    count: int,
    record: dict,
    batch: int = 4096,
    report: Callable[[str], None] = print,
) -> dict:
    """pathのscoreを、レコード[start, start+count)の範囲でその場で書き換える。

    出力を別に作らないので、314GBのpsvでも空きが要らない。書き換える前の
    scoreはsidecarへ控え、進み具合はprogressへ書く。順序は、sidecar→psvの
    順で書き、それぞれfsyncしてからprogressを進める。途中で落ちても、
    sidecarに控えた後のレコードしか書き換えていない。

    recordには条件（labeler、model、scale）を入れる。progressがあれば条件を
    照合し、同じなら続きから、違えば止める。
    """
    total = path.stat().st_size // PSV_BYTES
    if start < 0 or count <= 0 or start + count > total:
        raise ValueError(f"範囲が外れている: [{start}, {start + count}) / {total}")
    prog = progress_path(path)
    state = {
        **record,
        "start": start,
        "count": count,
        "done": 0,
        "sidecar_done": 0,
        "started": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "finished": None,
    }
    if prog.is_file():
        before = json.loads(prog.read_text(encoding="utf-8"))
        keys = ("labeler", "model", "scale", "start", "count")
        if any(before.get(k) != state[k] for k in keys):
            raise ValueError(
                f"進み具合の記録と条件が違う: {prog}\n前回: "
                + ", ".join(f"{k}={before.get(k)}" for k in keys)
            )
        if before.get("finished"):
            report(f"済み: {path}（{before['done']:,}局面を書き換え済み）")
            return before
        state.update(done=before["done"], sidecar_done=before["sidecar_done"], started=before["started"])
        report(f"再開: {state['done']:,}/{count:,} から")

    def save() -> None:
        state["updated"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
        tmp = prog.with_name(prog.name + ".tmp")
        tmp.write_text(json.dumps(state, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        tmp.replace(prog)

    began = time.time()
    began_done = state["done"]
    last = began
    with open(path, "r+b") as psv, open(sidecar_path(path), "r+b" if sidecar_path(path).exists() else "w+b") as side:
        while state["done"] < count:
            i = start + state["done"]
            want = min(batch, count - state["done"])
            psv.seek(i * PSV_BYTES)
            rows = np.frombuffer(psv.read(want * PSV_BYTES), dtype=np.uint8).reshape(-1, PSV_BYTES).copy()
            if state["sidecar_done"] <= state["done"]:
                old = rows[:, SCORE_OFFSET : SCORE_OFFSET + 2].copy()
                side.seek(i * 2)
                side.write(old.tobytes())
                _fsync(side)
                state["sidecar_done"] = state["done"] + want
                save()
            new = to_score(labeler(rows), scale)
            rows[:, SCORE_OFFSET : SCORE_OFFSET + 2] = new.astype("<i2").view(np.uint8).reshape(-1, 2)
            psv.seek(i * PSV_BYTES)
            psv.write(rows.tobytes())
            _fsync(psv)
            state["done"] += want
            save()
            now = time.time()
            if now - last >= 60:
                rate = (state["done"] - began_done) / (now - began)
                eta = (count - state["done"]) / rate if rate > 0 else float("inf")
                report(f"{state['done']:,}/{count:,} 局面 {rate:,.0f} 局面/秒 残り {eta / 3600:.1f} 時間")
                last = now
    state["finished"] = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    state["seconds"] = round(time.time() - began, 1)
    save()
    return state


# --- dlshogi --------------------------------------------------------------


def providers_for(device: str) -> list:
    """onnxruntimeのプロバイダ。CoreMLはMLProgram形式でfp32の値がCPUと一致する。

    既定のNeuralNetwork形式はfp16で、勝率が0.005ほどずれた（2026-09-18の実測）。
    """
    if device == "cpu":
        return ["CPUExecutionProvider"]
    if device == "coreml":
        return [("CoreMLExecutionProvider", {"ModelFormat": "MLProgram"}), "CPUExecutionProvider"]
    if device == "cuda":
        return ["CUDAExecutionProvider", "CPUExecutionProvider"]
    raise ValueError(f"知らないデバイス: {device}（cpu / coreml / cuda）")


def auto_device() -> str:
    import onnxruntime as ort

    have = ort.get_available_providers()
    if "CUDAExecutionProvider" in have:
        return "cuda"
    if "CoreMLExecutionProvider" in have:
        return "coreml"
    return "cpu"


class DlshogiLabeler:
    """dlshogiのONNXで、手番側の勝率を返す。

    入力特徴はcshogiの `dlshogi.make_input_features` で作る（62+57面）。
    出力の `output_value` はsigmoid済みの勝率で、手番側から見た値になる。
    psvのscoreも手番側から見た値なので、向きの変換は要らない。
    """

    def __init__(self, model: Path, device: str | None = None):
        try:
            import cshogi
            import onnxruntime as ort
            from cshogi import dlshogi
        except ImportError as e:  # pragma: no cover - 環境依存
            raise RuntimeError(
                "cshogi と onnxruntime が要る（pip install cshogi onnxruntime）。"
                "cshogiがclangで通らないときは、square.hppの列挙子の初期化を"
                "整数へキャストする（Issue #556）"
            ) from e
        self.device = device or auto_device()
        options = ort.SessionOptions()
        options.log_severity_level = 3  # CoreMLの分割の警告を黙らせる。値には影響しない
        self.session = ort.InferenceSession(
            str(model), sess_options=options, providers=providers_for(self.device)
        )
        self.board = cshogi.Board()
        self.dlshogi = dlshogi
        self.f1 = dlshogi.FEATURES1_NUM
        self.f2 = dlshogi.FEATURES2_NUM

    def __call__(self, records: np.ndarray) -> np.ndarray:
        n = len(records)
        x1 = np.zeros((n, self.f1, 9, 9), dtype=np.float32)
        x2 = np.zeros((n, self.f2, 9, 9), dtype=np.float32)
        for k in range(n):
            self.board.set_psfen(records[k, :32])
            self.dlshogi.make_input_features(self.board, x1[k], x2[k])
        out = self.session.run(["output_value"], {"input1": x1, "input2": x2})[0]
        return out.reshape(-1)
