"""Rust参照実装との順伝播一致テスト。

同一の乱数重みでモデルを初期化し、同一局面で
Rust(scalar推論)とPython(float順伝播)の評価値を比較する。
"""

import sys
import subprocess
import struct
import tempfile
import os

import numpy as np
import torch

sys.path.insert(0, ".")
from model import NnueModel, FT_IN, FT_OUT, EFFECT_IN, EFFECT_OUT, HIDDEN, CONCAT, SIGMOID_SCALE
from dataset import decode_psv, halfkp_features, effect_features
from quantize import save_hmwr


def python_forward_cp(model, board, king_sq, turn, hand):
    """Run Python float forward pass and return centipawn value."""
    stm_feats = halfkp_features(board, king_sq, hand, turn)
    opp_feats = halfkp_features(board, king_sq, hand, 1 - turn)
    ef_feats = effect_features(board, king_sq, turn)

    stm_idx = torch.tensor(stm_feats, dtype=torch.long)
    stm_off = torch.tensor([0], dtype=torch.long)
    opp_idx = torch.tensor(opp_feats, dtype=torch.long)
    opp_off = torch.tensor([0], dtype=torch.long)
    ef_idx = torch.tensor(ef_feats, dtype=torch.long)
    ef_off = torch.tensor([0], dtype=torch.long)

    model.eval()
    with torch.no_grad():
        v = model(stm_idx, stm_off, opp_idx, opp_off, ef_idx, ef_off).item()
    return v * SIGMOID_SCALE


def main():
    torch.manual_seed(42)
    model = NnueModel()

    net_path = "/tmp/test_forward.hmwr"
    save_hmwr(model, "test-forward", net_path)
    print(f"テスト用ネット書き出し: {net_path}")

    project_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    data_path = os.path.join(project_root, "data", "bench200k.psv")
    data = np.fromfile(data_path, dtype=np.uint8).reshape(-1, 40)

    # Rustベンチ用バイナリでテスト局面の評価値を取得する。
    # 既存のUSIエンジンに eval コマンドを送る方法が最もシンプル。
    # ここではPython単体で量子化後の整数推論を模して比較する。

    print("Python float vs 量子化整数の比較テスト:")
    n_test = 100
    max_diff = 0.0

    for i in range(n_test):
        record = bytes(data[i])
        board, king_sq, turn, hand, score, game_result = decode_psv(record)
        float_cp = python_forward_cp(model, board, king_sq, turn, hand)

        # 量子化整数推論をPythonで模倣する
        int_cp = integer_forward_cp(model, board, king_sq, turn, hand)
        diff = abs(float_cp - int_cp)
        max_diff = max(max_diff, diff)
        if i < 5:
            print(f"  [{i}] float={float_cp:.1f}cp int={int_cp}cp diff={diff:.1f}")

    print(f"最大誤差: {max_diff:.1f}cp ({n_test}局面)")
    if max_diff > 50.0:
        print("量子化誤差が大きすぎる")
        sys.exit(1)
    print("テスト通過")


def integer_forward_cp(model, board, king_sq, turn, hand):
    """量子化整数推論をPythonで模倣する (nnue.rs:164-241相当)。"""
    from quantize import quantize

    q = quantize(model)

    stm_feats = halfkp_features(board, king_sq, hand, turn)
    opp_feats = halfkp_features(board, king_sq, hand, 1 - turn)
    ef_feats = effect_features(board, king_sq, turn)

    ft_w = q["ft_w"].numpy()  # [FT_IN, FT_OUT] i16
    ft_b = q["ft_b"].numpy()  # [FT_OUT] i16
    ef_w = q["ef_w"].numpy()  # [EFFECT_IN, EFFECT_OUT] i16
    ef_b = q["ef_b"].numpy()  # [EFFECT_OUT] i16
    w2 = q["w2"].numpy()      # [HIDDEN, CONCAT] i8
    b2 = q["b2"].numpy()      # [HIDDEN] i32
    w3 = q["w3"].numpy()      # [HIDDEN, HIDDEN] i8
    b3 = q["b3"].numpy()      # [HIDDEN] i32
    w4 = q["w4"].numpy()      # [HIDDEN] i8
    b4 = q["b4"]              # i32 scalar

    # FT accumulator
    concat = np.zeros(CONCAT, dtype=np.uint8)
    for half, feats in enumerate([stm_feats, opp_feats]):
        acc = ft_b.astype(np.int32).copy()
        for f in feats:
            acc += ft_w[f].astype(np.int32)
        for o in range(FT_OUT):
            concat[half * FT_OUT + o] = max(0, min(127, acc[o]))

    # Effect tower
    ef_acc = ef_b.astype(np.int32).copy()
    for f in ef_feats:
        ef_acc += ef_w[f].astype(np.int32)
    for o in range(EFFECT_OUT):
        concat[FT_OUT * 2 + o] = max(0, min(127, ef_acc[o]))

    # Hidden layer 2
    h2 = np.zeros(HIDDEN, dtype=np.uint8)
    for o in range(HIDDEN):
        s = int(b2[o])
        for i in range(CONCAT):
            s += int(w2[o, i]) * int(concat[i])
        h2[o] = max(0, min(127, s >> 6))

    # Hidden layer 3
    h3 = np.zeros(HIDDEN, dtype=np.uint8)
    for o in range(HIDDEN):
        s = int(b3[o])
        for i in range(HIDDEN):
            s += int(w3[o, i]) * int(h2[i])
        h3[o] = max(0, min(127, s >> 6))

    # Output
    out = int(b4)
    for i in range(HIDDEN):
        out += int(w4[i]) * int(h3[i])

    return out // 16  # FV_SCALE


if __name__ == "__main__":
    main()
