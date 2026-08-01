"""Quantization and .hmwr file I/O via Rust PyO3 bridge (ADR-0043, ADR-0045)."""

import torch

import himawari
from model import SIGMOID_SCALE, FV_SCALE, NnueModel


def quantize(model: NnueModel) -> dict:
    """Extract f32 weights from model and quantize to integer types."""
    with torch.no_grad():
        # factorizer使用時は仮想特徴を畳み込む（ADR-0066）
        ft_w = model.folded_ft_weight()
        ft_b = model.ft_bias.detach().float()
        w2 = model.l2.weight.detach().float()
        b2 = model.l2.bias.detach().float()
        w3 = model.l3.weight.detach().float()
        b3 = model.l3.bias.detach().float()
        # 4層構成でだけ持つ隠れ層3（ADR-0127）
        w4 = model.l4.weight.detach().float() if model.l4 is not None else None
        b4 = model.l4.bias.detach().float() if model.l4 is not None else None
        w_out = model.out.weight.detach().float().squeeze(0)
        b_out = model.out.bias.detach().float().item()

    out_w_scale = SIGMOID_SCALE * FV_SCALE / 127.0

    def hidden_w(w):
        return (w * 64).round().clamp(-128, 127).to(torch.int8)

    def hidden_b(b):
        return (b * 64 * 127).round().to(torch.int32)

    return {
        "ft_w": (ft_w * 127).round().clamp(-32768, 32767).to(torch.int16),
        "ft_b": (ft_b * 127).round().clamp(-32768, 32767).to(torch.int16),
        "w2": hidden_w(w2),
        "b2": hidden_b(b2),
        "w3": hidden_w(w3),
        "b3": hidden_b(b3),
        "w4": None if w4 is None else hidden_w(w4),
        "b4": None if b4 is None else hidden_b(b4),
        "w_out": (w_out * out_w_scale).round().clamp(-128, 127).to(torch.int8),
        "b_out": round(b_out * SIGMOID_SCALE * FV_SCALE),
    }


def save_hmwr(model: NnueModel, lineage: str, path: str):
    """Quantize model and write .hmwr file via Rust."""
    q = quantize(model)
    himawari.save_hmwr(
        path, lineage,
        q["ft_w"].flatten().tolist(),
        q["ft_b"].tolist(),
        q["w2"].flatten().tolist(),
        q["b2"].tolist(),
        q["w3"].flatten().tolist(),
        q["b3"].tolist(),
        q["w_out"].tolist(),
        q["b_out"],
        None if q["w4"] is None else q["w4"].flatten().tolist(),
        None if q["b4"] is None else q["b4"].tolist(),
    )
