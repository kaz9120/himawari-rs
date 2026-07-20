"""NNUE model definition (ADR-0034, ADR-0040, ADR-0045).

ADR-0045で利き塔を除去し、純粋HalfKP構成にした。
"""

import torch
import torch.nn as nn
import torch.nn.functional as F

# Architecture constants (ADR-0034, bonapiece.rs, nnue.rs)
FE_END = 1548
FT_IN = 81 * FE_END  # 125,388
FT_OUT = 256
CONCAT = FT_OUT * 2  # 512 (ADR-0045: 利き塔除去で旧544→512)
HIDDEN = 32

# Evaluation scale (ADR-0036)
SIGMOID_SCALE = 600.0
FV_SCALE = 16

# Quantization limits (model.rs:18-21)
HIDDEN_W_LIMIT = 127.0 / 64.0
OUT_W_SCALE = SIGMOID_SCALE * FV_SCALE / 127.0
OUT_W_LIMIT = 127.0 / OUT_W_SCALE


class NnueModel(nn.Module):
    def __init__(self):
        super().__init__()
        self.ft = nn.EmbeddingBag(FT_IN, FT_OUT, mode="sum", sparse=True)
        self.ft_bias = nn.Parameter(torch.full((FT_OUT,), 0.5))
        self.l2 = nn.Linear(CONCAT, HIDDEN)
        self.l3 = nn.Linear(HIDDEN, HIDDEN)
        self.l4 = nn.Linear(HIDDEN, 1)
        self._init_weights()

    def _init_weights(self):
        nn.init.uniform_(self.ft.weight, -0.05, 0.05)
        nn.init.uniform_(self.l2.weight, -0.1, 0.1)
        nn.init.zeros_(self.l2.bias)
        nn.init.uniform_(self.l3.weight, -0.3, 0.3)
        nn.init.zeros_(self.l3.bias)
        nn.init.uniform_(self.l4.weight, -0.3, 0.3)
        nn.init.zeros_(self.l4.bias)

    def forward(self, stm_idx, stm_off, opp_idx, opp_off):
        z_stm = self.ft(stm_idx, stm_off) + self.ft_bias
        z_opp = self.ft(opp_idx, opp_off) + self.ft_bias
        x = torch.cat([z_stm.clamp(0.0, 1.0), z_opp.clamp(0.0, 1.0)], dim=1)
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
