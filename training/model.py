"""NNUE model definition (ADR-0034, ADR-0040, ADR-0044)."""

import torch
import torch.nn as nn
import torch.nn.functional as F

# Architecture constants (ADR-0034, bonapiece.rs, nnue.rs)
FE_END = 1548
FT_IN = 81 * FE_END  # 125,388
FT_OUT = 256
EFFECT_IN = 800  # 2 slots * 25 cells * 16 classes
EFFECT_OUT = 32
HIDDEN = 32

# King-line feature constants (ADR-0044)
N_DIRECTIONS = 8
N_PIECE_ENC = 113  # 2 colors * 14 types * 4 distance_buckets + 1 empty
KINGLINE_IN = 2 * N_DIRECTIONS * N_PIECE_ENC  # 1,808

# Evaluation scale (ADR-0036)
SIGMOID_SCALE = 600.0
FV_SCALE = 16

# Quantization limits (model.rs:18-21)
HIDDEN_W_LIMIT = 127.0 / 64.0
OUT_W_SCALE = SIGMOID_SCALE * FV_SCALE / 127.0
OUT_W_LIMIT = 127.0 / OUT_W_SCALE

# Architecture names
ARCH_HALFKP_EFFECT = "halfkp_effect"  # current: HalfKP + effect tower
ARCH_HALFKP = "halfkp"                # condition 1: pure HalfKP
ARCH_HALFKP_KINGLINE = "halfkp_kingline"  # condition 2: HalfKP + king-line


def concat_size(arch):
    if arch == ARCH_HALFKP:
        return FT_OUT * 2
    elif arch == ARCH_HALFKP_EFFECT:
        return FT_OUT * 2 + EFFECT_OUT
    elif arch == ARCH_HALFKP_KINGLINE:
        return FT_OUT * 2 + EFFECT_OUT
    raise ValueError(f"unknown arch: {arch}")


def second_tower_dim(arch):
    if arch == ARCH_HALFKP:
        return 0
    elif arch == ARCH_HALFKP_EFFECT:
        return EFFECT_OUT
    elif arch == ARCH_HALFKP_KINGLINE:
        return EFFECT_OUT
    raise ValueError(f"unknown arch: {arch}")


def second_tower_in(arch):
    if arch == ARCH_HALFKP:
        return 0
    elif arch == ARCH_HALFKP_EFFECT:
        return EFFECT_IN
    elif arch == ARCH_HALFKP_KINGLINE:
        return KINGLINE_IN
    raise ValueError(f"unknown arch: {arch}")


class NnueModel(nn.Module):
    def __init__(self, arch=ARCH_HALFKP_EFFECT):
        super().__init__()
        self.arch = arch
        c = concat_size(arch)
        st_in = second_tower_in(arch)
        st_out = second_tower_dim(arch)

        self.ft = nn.EmbeddingBag(FT_IN, FT_OUT, mode="sum", sparse=True)
        self.ft_bias = nn.Parameter(torch.full((FT_OUT,), 0.5))

        if st_in > 0:
            self.ef = nn.EmbeddingBag(st_in, st_out, mode="sum", sparse=True)
            self.ef_bias = nn.Parameter(torch.full((st_out,), 0.5))
        else:
            self.ef = None
            self.ef_bias = None

        self.l2 = nn.Linear(c, HIDDEN)
        self.l3 = nn.Linear(HIDDEN, HIDDEN)
        self.l4 = nn.Linear(HIDDEN, 1)
        self._init_weights()

    def _init_weights(self):
        nn.init.uniform_(self.ft.weight, -0.05, 0.05)
        if self.ef is not None:
            nn.init.uniform_(self.ef.weight, -0.05, 0.05)
        nn.init.uniform_(self.l2.weight, -0.1, 0.1)
        nn.init.zeros_(self.l2.bias)
        nn.init.uniform_(self.l3.weight, -0.3, 0.3)
        nn.init.zeros_(self.l3.bias)
        nn.init.uniform_(self.l4.weight, -0.3, 0.3)
        nn.init.zeros_(self.l4.bias)

    def forward(self, stm_idx, stm_off, opp_idx, opp_off, ef_idx, ef_off):
        z_stm = self.ft(stm_idx, stm_off) + self.ft_bias
        z_opp = self.ft(opp_idx, opp_off) + self.ft_bias
        parts = [z_stm.clamp(0.0, 1.0), z_opp.clamp(0.0, 1.0)]
        if self.ef is not None:
            z_ef = self.ef(ef_idx, ef_off) + self.ef_bias
            parts.append(z_ef.clamp(0.0, 1.0))
        x = torch.cat(parts, dim=1)
        h2 = self.l2(x).clamp(0.0, 1.0)
        h3 = self.l3(h2).clamp(0.0, 1.0)
        return self.l4(h3).squeeze(1)

    def clip_weights(self):
        with torch.no_grad():
            self.l2.weight.clamp_(-HIDDEN_W_LIMIT, HIDDEN_W_LIMIT)
            self.l3.weight.clamp_(-HIDDEN_W_LIMIT, HIDDEN_W_LIMIT)
            self.l4.weight.clamp_(-OUT_W_LIMIT, OUT_W_LIMIT)


def loss_fn(output, target):
    return F.binary_cross_entropy_with_logits(output, target)
