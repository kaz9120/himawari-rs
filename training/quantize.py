"""Quantization and .hmwr file I/O via Rust PyO3 bridge (ADR-0043)."""

import torch

import himawari
from model import (
    FT_IN, FT_OUT, EFFECT_IN, EFFECT_OUT, HIDDEN,
    SIGMOID_SCALE, FV_SCALE, NnueModel, ARCH_HALFKP_EFFECT,
)


def quantize(model: NnueModel) -> dict:
    """Extract f32 weights from model and quantize to integer types."""
    with torch.no_grad():
        ft_w = model.ft.weight.detach().float()
        ft_b = model.ft_bias.detach().float()
        ef_w = model.ef.weight.detach().float()
        ef_b = model.ef_bias.detach().float()
        w2 = model.l2.weight.detach().float()
        b2 = model.l2.bias.detach().float()
        w3 = model.l3.weight.detach().float()
        b3 = model.l3.bias.detach().float()
        w4 = model.l4.weight.detach().float().squeeze(0)
        b4 = model.l4.bias.detach().float().item()

    out_w_scale = SIGMOID_SCALE * FV_SCALE / 127.0

    return {
        "ft_w": (ft_w * 127).round().clamp(-32768, 32767).to(torch.int16),
        "ft_b": (ft_b * 127).round().clamp(-32768, 32767).to(torch.int16),
        "ef_w": (ef_w * 127).round().clamp(-32768, 32767).to(torch.int16),
        "ef_b": (ef_b * 127).round().clamp(-32768, 32767).to(torch.int16),
        "w2": (w2 * 64).round().clamp(-128, 127).to(torch.int8),
        "b2": (b2 * 64 * 127).round().to(torch.int32),
        "w3": (w3 * 64).round().clamp(-128, 127).to(torch.int8),
        "b3": (b3 * 64 * 127).round().to(torch.int32),
        "w4": (w4 * out_w_scale).round().clamp(-128, 127).to(torch.int8),
        "b4": round(b4 * SIGMOID_SCALE * FV_SCALE),
    }


def save_hmwr(model: NnueModel, lineage: str, path: str):
    """Quantize model and write .hmwr file via Rust."""
    q = quantize(model)
    himawari.save_hmwr(
        path, lineage,
        q["ft_w"].flatten().tolist(),
        q["ft_b"].tolist(),
        q["ef_w"].flatten().tolist(),
        q["ef_b"].tolist(),
        q["w2"].flatten().tolist(),
        q["b2"].tolist(),
        q["w3"].flatten().tolist(),
        q["b3"].tolist(),
        q["w4"].tolist(),
        q["b4"],
    )


def load_hmwr(path: str) -> tuple[dict, str]:
    """Load .hmwr file via Rust. Returns (quantized_dict, lineage)."""
    d = himawari.load_hmwr(path)
    lineage = d["lineage"]
    return {
        "ft_w": torch.tensor(d["ft_w"], dtype=torch.int16).reshape(FT_IN, FT_OUT),
        "ft_b": torch.tensor(d["ft_b"], dtype=torch.int16),
        "ef_w": torch.tensor(d["ef_w"], dtype=torch.int16).reshape(EFFECT_IN, EFFECT_OUT),
        "ef_b": torch.tensor(d["ef_b"], dtype=torch.int16),
        "w2": torch.tensor(d["w2"], dtype=torch.int8).reshape(HIDDEN, FT_OUT * 2 + EFFECT_OUT),
        "b2": torch.tensor(d["b2"], dtype=torch.int32),
        "w3": torch.tensor(d["w3"], dtype=torch.int8).reshape(HIDDEN, HIDDEN),
        "b3": torch.tensor(d["b3"], dtype=torch.int32),
        "w4": torch.tensor(d["w4"], dtype=torch.int8),
        "b4": d["b4"],
    }, lineage
