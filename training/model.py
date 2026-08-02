"""NNUE model definition (ADR-0034, ADR-0040, ADR-0045).

ADR-0045で利き塔を除去し、純粋HalfKP構成にした。
"""

import torch
import torch.nn as nn
import torch.nn.functional as F

import himawari

# 次元はRust側のビルドから読む（ADR-0067・ADR-0127）。
# HIMAWARI_ARCH を指定してビルドしたhimawariモジュールを入れれば、
# 学習側も同じ構成になる。定数を二重に持たないので、推論と学習の
# 取り違えが起きない
FT_IN = himawari.FT_IN
FT_OUT = himawari.FT_OUT
L1_OUT = himawari.L1_OUT
L2_OUT = himawari.L2_OUT
# 0なら3層。0でなければ隠れ層をもう1つ挟む（ADR-0127）
L3_OUT = himawari.L3_OUT
LAST_HIDDEN = L3_OUT if L3_OUT else L2_OUT
# 補助ヘッドの分類クラス数（ADR-0129）。fromは盤上81マス＋打つ駒7種
MOVE_FROM_CLASSES = himawari.MOVE_FROM_CLASSES
MOVE_TO_CLASSES = himawari.MOVE_TO_CLASSES
ARCH = himawari.ARCH
FE_END = FT_IN // 81
CONCAT = FT_OUT * 2

# Evaluation scale (ADR-0036)
SIGMOID_SCALE = 600.0
FV_SCALE = 16

# Quantization limits (model.rs:18-21)
HIDDEN_W_LIMIT = 127.0 / 64.0
OUT_W_SCALE = SIGMOID_SCALE * FV_SCALE / 127.0
OUT_W_LIMIT = 127.0 / OUT_W_SCALE


class NnueModel(nn.Module):
    """HalfKPのNNUE。

    `factorized=True` のとき、学習時だけ玉位置に依らない駒の特徴
    （BonaPiece単独）を並列に持つ（ADR-0066）。`halfkp_index` は
    `king * FE_END + bona_piece` なので、実特徴のインデックスを
    FE_ENDで割った余りが仮想特徴のインデックスになる。
    推論側の構造は変わらない。重みは書き出し時に畳み込む。
    """

    def __init__(self, sparse_ft=True, factorized=False, policy=False):
        super().__init__()
        self.ft = nn.EmbeddingBag(FT_IN, FT_OUT, mode="sum", sparse=sparse_ft)
        self.ft_p = (
            nn.EmbeddingBag(FE_END, FT_OUT, mode="sum", sparse=sparse_ft)
            if factorized
            else None
        )
        self.ft_bias = nn.Parameter(torch.full((FT_OUT,), 0.5))
        self.l2 = nn.Linear(CONCAT, L1_OUT)
        self.l3 = nn.Linear(L1_OUT, L2_OUT)
        self.l4 = nn.Linear(L2_OUT, L3_OUT) if L3_OUT else None
        self.out = nn.Linear(LAST_HIDDEN, 1)
        # 補助ヘッド（ADR-0129）。FT出力からの線形1層に限る。深くすると
        # ヘッド自身がタスクを解いてしまい、FTへ表現を押し込む圧力が弱まる
        self.policy_from = nn.Linear(CONCAT, MOVE_FROM_CLASSES) if policy else None
        self.policy_to = nn.Linear(CONCAT, MOVE_TO_CLASSES) if policy else None
        self._init_weights()

    def _init_weights(self):
        nn.init.uniform_(self.ft.weight, -0.05, 0.05)
        if self.ft_p is not None:
            nn.init.zeros_(self.ft_p.weight)
        nn.init.uniform_(self.l2.weight, -0.1, 0.1)
        nn.init.zeros_(self.l2.bias)
        nn.init.uniform_(self.l3.weight, -0.3, 0.3)
        nn.init.zeros_(self.l3.bias)
        if self.l4 is not None:
            nn.init.uniform_(self.l4.weight, -0.3, 0.3)
            nn.init.zeros_(self.l4.bias)
        nn.init.uniform_(self.out.weight, -0.3, 0.3)
        nn.init.zeros_(self.out.bias)
        for head in (self.policy_from, self.policy_to):
            if head is not None:
                nn.init.uniform_(head.weight, -0.05, 0.05)
                nn.init.zeros_(head.bias)

    def transform(self, idx, off):
        z = self.ft(idx, off)
        if self.ft_p is not None:
            z = z + self.ft_p(idx % FE_END, off)
        return z + self.ft_bias

    def folded_ft_weight(self):
        """仮想特徴を畳み込んだFT重みを返す（書き出し用）。"""
        w = self.ft.weight.detach().float()
        if self.ft_p is None:
            return w
        virtual = self.ft_p.weight.detach().float()
        return (w.view(81, FE_END, FT_OUT) + virtual.unsqueeze(0)).view(FT_IN, FT_OUT)

    def transform_both(self, stm_idx, stm_off, opp_idx, opp_off):
        """FT出力を2視点ぶん連結して返す。補助ヘッドもここから生やす。"""
        z_stm = self.transform(stm_idx, stm_off)
        z_opp = self.transform(opp_idx, opp_off)
        return torch.cat([z_stm.clamp(0.0, 1.0), z_opp.clamp(0.0, 1.0)], dim=1)

    def value(self, x):
        """FT出力の連結から評価値を出す（推論と同じ経路）。"""
        h = self.l3(self.l2(x).clamp(0.0, 1.0)).clamp(0.0, 1.0)
        if self.l4 is not None:
            h = self.l4(h).clamp(0.0, 1.0)
        return self.out(h).squeeze(1)

    def forward(self, stm_idx, stm_off, opp_idx, opp_off):
        return self.value(self.transform_both(stm_idx, stm_off, opp_idx, opp_off))

    def clip_weights(self):
        with torch.no_grad():
            self.l2.weight.clamp_(-HIDDEN_W_LIMIT, HIDDEN_W_LIMIT)
            self.l3.weight.clamp_(-HIDDEN_W_LIMIT, HIDDEN_W_LIMIT)
            if self.l4 is not None:
                self.l4.weight.clamp_(-HIDDEN_W_LIMIT, HIDDEN_W_LIMIT)
            self.out.weight.clamp_(-OUT_W_LIMIT, OUT_W_LIMIT)


def loss_fn(output, target):
    return F.binary_cross_entropy_with_logits(output, target)
