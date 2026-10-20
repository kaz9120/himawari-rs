"""利きの入力特徴（ADR-0237のF2）の、学習側と推論側の一致を確かめる。

2つを見る。

1. 特徴: Rustの抽出（`extract_batch(effect_inputs=True)`）が返す特徴を、
   Pythonで別に組んだ特徴と突き合わせる。Python側は `shadow_flips` の
   利き数（段階0の測定に使う実装）とcshogiの盤から作る。実装を共有しないので、
   視点の反転・駒の番号・遮りの取り違えがあれば割れる
2. 値: 学習側の模型を量子化して書き出し、推論側の全計算（`evaluate_records`）
   で同じ局面を評価する。模型の出力×600と、量子化の誤差の範囲で一致することを
   見る。統制群（特徴を立てない）と処置群（立てる）の両方で確かめる

拡張・cshogi・torchが入っていない環境（CI）では飛ばす。値のテストは、利きの
構成（`HIMAWARI_ARCH=...e64`）で作った拡張でだけ走る。
"""

import random
import sys
from pathlib import Path

import pytest

np = pytest.importorskip("numpy")
cshogi = pytest.importorskip("cshogi")
himawari = pytest.importorskip("himawari")
if not hasattr(himawari, "EFF_IN"):
    pytest.skip("利きの入力特徴を持たない古い拡張", allow_module_level=True)

from hmwr.tools import shadow_flips as sf  # noqa: E402

TRAINING = Path(__file__).resolve().parent.parent / "training"

PSV_BYTES = 40
SQUARES = 81
PIECES = 29
LEVELS = 4

# cshogiの駒種（1〜14）から、エンジンの駒種（2〜15、piece.rs）への写像
CSHOGI_TO_ENGINE = {
    1: 10,  # 歩
    2: 11,  # 香
    3: 12,  # 桂
    4: 13,  # 銀
    5: 14,  # 角
    6: 15,  # 飛
    7: 9,  # 金
    8: 8,  # 玉
    9: 2,  # と
    10: 3,  # 成香
    11: 4,  # 成桂
    12: 5,  # 成銀
    13: 6,  # 馬
    14: 7,  # 龍
}


def positions(count=200, seed=1):
    """ランダムに進めた局面をpsvのレコード列とcshogiの盤の組で返す。"""
    rng = random.Random(seed)
    records = bytearray()
    boards = []
    buf = np.zeros(32, dtype=np.uint8)
    while len(boards) < count:
        board = cshogi.Board()
        for ply in range(rng.randrange(10, 160)):
            moves = list(board.legal_moves)
            if not moves or board.is_game_over():
                break
            board.push(rng.choice(moves))
            if ply % 9 == 4:
                board.to_psfen(buf)
                rec = bytearray(PSV_BYTES)
                rec[:32] = buf.tobytes()
                ply_no = board.move_number
                rec[36:38] = ply_no.to_bytes(2, "little")
                records += rec
                boards.append((list(board.pieces), board.turn))
    return bytes(records[: count * PSV_BYTES]), boards[:count]


def python_features(pieces, color):
    """視点colorのF2の特徴81個を、Rustと別の経路で作る（ADR-0237）。"""
    counts = sf.exact_counts(pieces)
    own, opp = counts[color], counts[1 - color]
    out = [0] * SQUARES
    for sq, code in enumerate(pieces):
        if code:
            white = code > sf.WHITE
            kind = CSHOGI_TO_ENGINE[code & 15] - 1
            pc = kind if int(white) == color else kind + 14
        else:
            pc = 0
        view = sq if color == 0 else SQUARES - 1 - sq
        cap = LEVELS - 1
        out[view] = ((view * PIECES + pc) * LEVELS + min(own[sq], cap)) * LEVELS + min(
            opp[sq], cap
        )
    return out


def test_rust_features_match_python():
    records, boards = positions()
    arrays = himawari.extract_batch(records, 0.7, 0, 0, False, True, effect_inputs=True)
    eff_stm, eff_opp = arrays[9], arrays[10]
    assert len(eff_stm) == len(boards) * SQUARES
    for i, (pieces, turn) in enumerate(boards):
        got_stm = eff_stm[i * SQUARES : (i + 1) * SQUARES].tolist()
        got_opp = eff_opp[i * SQUARES : (i + 1) * SQUARES].tolist()
        assert got_stm == python_features(pieces, turn), f"手番視点 {i}"
        assert got_opp == python_features(pieces, 1 - turn), f"非手番視点 {i}"


def test_features_are_off_unless_asked():
    records, _ = positions(count=4)
    arrays = himawari.extract_batch(records, 0.7, 0, 0, False, True)
    assert len(arrays) == 11
    assert len(arrays[9]) == 0 and len(arrays[10]) == 0


@pytest.mark.parametrize("mode", ["f2", "none"])
def test_quantized_net_matches_the_model(mode, tmp_path):
    """模型の出力と、書き出したネットの推論側の評価値が一致する（ADR-0237）。"""
    torch = pytest.importorskip("torch")
    if himawari.EFF_OUT == 0:
        pytest.skip("利きの構成（e<E>）の拡張でだけ測れる")
    sys.path.insert(0, str(TRAINING))
    from dataset import to_tensors
    from model import FV_SCALE, SIGMOID_SCALE, NnueModel
    from quantize import save_hmwr

    torch.manual_seed(3)
    model = NnueModel(sparse_ft=False, effect_inputs=mode)
    with torch.no_grad():
        # 利きの経路が評価値に効く大きさにする。既定の初期値のままでは
        # 量子化の誤差に埋もれ、経路が切れていても一致してしまう
        if mode == "f2":
            model.eff.weight.uniform_(-0.1, 0.1)
        model.eff_bias.uniform_(0.2, 0.8)
        cols = model.l2.weight[:, -2 * himawari.EFF_OUT :]
        cols.uniform_(-1.5, 1.5)
        model.clip_weights()
        # 重みを量子化の格子へ寄せる。残る差は活性の丸め（u8への切り捨てと
        # 隠れ層の >>6）だけになり、利きの経路の取り違えと区別しやすくなる
        out_scale = SIGMOID_SCALE * FV_SCALE / 127
        snap = [(model.ft.weight, 127), (model.ft_bias, 127), (model.eff.weight, 127),
                (model.eff_bias, 127), (model.l2.weight, 64), (model.l3.weight, 64),
                (model.l2.bias, 64 * 127), (model.l3.bias, 64 * 127),
                (model.out.weight, out_scale), (model.out.bias, SIGMOID_SCALE * FV_SCALE)]
        for t, scale in snap:
            t.copy_((t * scale).round() / scale)
    model.eval()

    records, _ = positions(count=120, seed=7)
    arrays = to_tensors(
        himawari.extract_batch(records, 0.7, 0, 0, False, True, effect_inputs=True)
    )
    stm_i, stm_o, opp_i, opp_o = arrays[:4]
    with torch.no_grad():
        want = model(stm_i, stm_o, opp_i, opp_o, arrays[9], arrays[10]) * SIGMOID_SCALE
        # 利きの活性を0にした出力。経路が効いていることの対照にする
        x = model.transform_both(stm_i, stm_o, opp_i, opp_o, arrays[9], arrays[10])
        x[:, -2 * himawari.EFF_OUT :] = 0
        blind = model.value(x) * SIGMOID_SCALE

    path = tmp_path / "eff.hmwr"
    save_hmwr(model, f"test {mode}", str(path))
    got = torch.tensor(himawari.evaluate_records(str(path), records), dtype=torch.float32)

    err = (got - want).abs()
    effect = (blind - want).abs()
    # 重みを格子へ寄せたので、残る差は活性の丸めだけで平均1点前後（歩=90、
    # 2026-10-18の実測で平均0.9〜1.0、最大3〜4）。利きの経路の寄与は平均70点
    # 前後あるので、経路が切れていれば平均誤差がそこまで開く
    assert err.mean() < 2.0, f"平均誤差 {err.mean():.2f} 最大 {err.max():.2f}"
    assert err.max() < 8.0, f"最大誤差 {err.max():.2f}"
    assert effect.mean() > 10 * err.mean(), (
        f"利きの寄与 {effect.mean():.2f} が量子化の誤差 {err.mean():.2f} に埋もれている"
    )


def test_groups_start_from_the_same_weights():
    """統制群と処置群は、利きの表を除いて同じ初期値から始まる（ADR-0237）。

    構造を揃えたのは乱数の消費順序を揃えるためである（ADR-0158の教訓）。
    統制群も表の乱数を引いてから0にするので、後の層の初期値がずれない。
    """
    torch = pytest.importorskip("torch")
    if himawari.EFF_OUT == 0:
        pytest.skip("利きの構成（e<E>）の拡張でだけ測れる")
    sys.path.insert(0, str(TRAINING))
    from model import NnueModel

    states = {}
    for mode in ("f2", "none"):
        torch.manual_seed(0)
        model = NnueModel(sparse_ft=False, factorized=True, effect_inputs=mode)
        states[mode] = model.state_dict()
    f2, none = states["f2"], states["none"]
    assert f2.keys() == none.keys()
    for key in f2:
        if key == "eff.weight":
            continue
        assert torch.equal(f2[key], none[key]), key
    assert not torch.equal(f2["eff.weight"], none["eff.weight"])
    assert torch.count_nonzero(none["eff.weight"]) == 0, "統制群の表は0"
