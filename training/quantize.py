"""Quantization and .hmwr file I/O via Rust PyO3 bridge (ADR-0043, ADR-0045)."""

import torch

import himawari
from model import (
    CONCAT, EFF_IN, EFF_OUT, FT_IN, FT_OUT, FV_SCALE, L1_OUT, L2_OUT, L3_OUT,
    LAST_HIDDEN, SIGMOID_SCALE, NnueModel,
)


def quantize(model: NnueModel) -> dict:
    """Extract f32 weights from model and quantize to integer types."""
    with torch.no_grad():
        # factorizer使用時は仮想特徴を畳み込む（ADR-0066）
        ft_w = model.folded_ft_weight()
        ft_b = model.ft_bias.detach().float()
        w2 = model.l2.weight.detach().float()
        b2 = model.l2.bias.detach().float()
        w3 = model.l3.weight.detach().float() if model.l3 is not None else None
        b3 = model.l3.bias.detach().float() if model.l3 is not None else None
        # 4層構成でだけ持つ隠れ層3（ADR-0127）
        w4 = model.l4.weight.detach().float() if model.l4 is not None else None
        b4 = model.l4.bias.detach().float() if model.l4 is not None else None
        w_out = model.out.weight.detach().float().squeeze(0)
        b_out = model.out.bias.detach().float().item()
        # 利きのaccumulator（ADR-0237）。FTと同じ127倍で、i16のまま持つ
        # 表し方を替えた構成（ADR-0245）は末尾に詰め物の行を持つので、EFF_IN行で切る
        # 側に入れたHalfKPのfactorizerもここで畳み込む（ADR-0245）
        eff_w = model.folded_eff_weight() if model.eff is not None else None
        eff_b = model.eff_bias.detach().float() if model.eff is not None else None

    out_w_scale = SIGMOID_SCALE * FV_SCALE / 127.0

    def ft_q(w):
        return (w * 127).round().clamp(-32768, 32767).to(torch.int16)

    def hidden_w(w):
        return (w * 64).round().clamp(-128, 127).to(torch.int8)

    def hidden_b(b):
        return (b * 64 * 127).round().to(torch.int32)

    return {
        "ft_w": (ft_w * 127).round().clamp(-32768, 32767).to(torch.int16),
        "ft_b": (ft_b * 127).round().clamp(-32768, 32767).to(torch.int16),
        "w2": hidden_w(w2),
        "b2": hidden_b(b2),
        "w3": None if w3 is None else hidden_w(w3),
        "b3": None if b3 is None else hidden_b(b3),
        "w4": None if w4 is None else hidden_w(w4),
        "b4": None if b4 is None else hidden_b(b4),
        "w_out": (w_out * out_w_scale).round().clamp(-128, 127).to(torch.int8),
        "b_out": round(b_out * SIGMOID_SCALE * FV_SCALE),
        "eff_w": None if eff_w is None else ft_q(eff_w),
        "eff_b": None if eff_b is None else ft_q(eff_b),
        # 玉の近傍の部分モデル（ADR-0246）はf32のまま載せる
        "sub_w": None if model.sub is None else model.sub.raw_weights(),
    }


def load_into(model: NnueModel, path: str, freeze_ft: bool = False) -> str:
    """既存の.hmwrを初期値として読む（ADR-0130）。

    FTは常に読む。後段は元ファイルの構成が今の構成と一致するときだけ読み、
    違えば乱数初期化のまま残す。後段の形を変えて比べるとき（足切り）はFTだけが
    載り、同じ形で学習を続けるときは全層が載る。

    量子化の逆変換なので、元のf32とは丸めのぶんだけ違う。凍結するなら差は
    動かないので影響しない。
    """
    w = himawari.load_hmwr(path)
    here = f"{himawari.FT_OUT}x{himawari.L1_OUT}x{himawari.L2_OUT}x{himawari.L3_OUT}"
    same_shape = w["src_arch"] == here

    def as_tensor(key, *shape):
        return torch.from_numpy(w[key]).float().view(*shape)

    with torch.no_grad():
        # factorizer使用時、仮想特徴は畳み込み済みなのでゼロに戻す。
        # folded_ft_weight() が ft + ft_p を返すので、ftへ全部入れれば等価
        model.ft.weight.copy_(as_tensor("ft_w", FT_IN, FT_OUT))
        model.ft_bias.copy_(as_tensor("ft_b", FT_OUT))
        if model.ft_p is not None:
            model.ft_p.weight.zero_()
        # 利きのaccumulator（ADR-0237）。利きの構成は同じ構成のファイルしか
        # 読めないので、拡張が返した長さのまま写せる
        if model.eff is not None and len(w["eff_w"]):
            model.eff.weight[:EFF_IN].copy_(as_tensor("eff_w", EFF_IN, EFF_OUT))
            if model.eff_p is not None:
                model.eff_p.weight.zero_()
            model.eff_bias.copy_(as_tensor("eff_b", EFF_OUT))
        if model.sub is not None and len(w["sub_w"]):
            model.sub.load_raw(torch.from_numpy(w["sub_w"]).float())

        if same_shape:
            model.l2.weight.copy_(as_tensor("w2", L1_OUT, CONCAT))
            model.l2.bias.copy_(as_tensor("b2", L1_OUT))
            if model.l3 is not None:
                model.l3.weight.copy_(as_tensor("w3", L2_OUT, L1_OUT))
                model.l3.bias.copy_(as_tensor("b3", L2_OUT))
            if model.l4 is not None:
                model.l4.weight.copy_(as_tensor("w4", L3_OUT, L2_OUT))
                model.l4.bias.copy_(as_tensor("b4", L3_OUT))
            model.out.weight.copy_(as_tensor("w_out", 1, LAST_HIDDEN))
            model.out.bias.fill_(w["b_out"])

    if freeze_ft:
        model.ft.weight.requires_grad_(False)
        model.ft_bias.requires_grad_(False)
        if model.ft_p is not None:
            model.ft_p.weight.requires_grad_(False)

    return (
        f"{path} (構成 {w['src_arch']}、後段は"
        f"{'読んだ' if same_shape else '乱数のまま'}"
        f"{'、FTは凍結' if freeze_ft else ''})"
    )


def save_hmwr(model: NnueModel, lineage: str, path: str):
    """Quantize model and write .hmwr file via Rust."""
    q = quantize(model)
    himawari.save_hmwr(
        path, lineage,
        q["ft_w"].flatten().tolist(),
        q["ft_b"].tolist(),
        q["w2"].flatten().tolist(),
        q["b2"].tolist(),
        [] if q["w3"] is None else q["w3"].flatten().tolist(),
        [] if q["b3"] is None else q["b3"].tolist(),
        q["w_out"].tolist(),
        q["b_out"],
        None if q["w4"] is None else q["w4"].flatten().tolist(),
        None if q["b4"] is None else q["b4"].tolist(),
        **(
            {}
            if q["eff_w"] is None
            else {
                "eff_w": q["eff_w"].flatten().tolist(),
                "eff_b": q["eff_b"].tolist(),
            }
        ),
        **({} if q["sub_w"] is None else {"sub_w": q["sub_w"].tolist()}),
    )
