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
LAST_HIDDEN = L3_OUT or L2_OUT or L1_OUT
# 補助ヘッドの分類クラス数（ADR-0129）。fromは盤上81マス＋打つ駒7種
MOVE_FROM_CLASSES = himawari.MOVE_FROM_CLASSES
MOVE_TO_CLASSES = himawari.MOVE_TO_CLASSES
# 利きラベル1本の長さ（ADR-0133）。手番側81升＋相手側81升
EFFECT_LEN = himawari.EFFECT_LEN
# 利きヘッドの出力次元。短い利きと長い利きを続けて並べる
EFFECT_OUT = EFFECT_LEN * 2
# 2層MLPヘッドの中間の幅
EFFECT_MLP_HIDDEN = 256
# 利き数の正規化に使う。1升に8枚も利いていれば十分に多い
EFFECT_SCALE = 8.0
ARCH = himawari.ARCH
# 玉バケットの数（ADR-0157）。左右対称な玉位置は同じバケットを共有する
KING_BUCKETS = himawari.KING_BUCKETS
FE_END = FT_IN // KING_BUCKETS
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

    def __init__(self, sparse_ft=True, factorized=False, policy=False,
                 pretrain=False, distill_out=0, effect_head=None):
        super().__init__()
        self.ft = nn.EmbeddingBag(FT_IN, FT_OUT, mode="sum", sparse=sparse_ft)
        self.ft_p = (
            nn.EmbeddingBag(FE_END, FT_OUT, mode="sum", sparse=sparse_ft)
            if factorized
            else None
        )
        self.ft_bias = nn.Parameter(torch.full((FT_OUT,), 0.5))
        self.l2 = nn.Linear(CONCAT, L1_OUT)
        # 隠れ層は書いたぶんだけ持つ（ADR-0127）。L2_OUT=0 なら隠れ層1つ
        self.l3 = nn.Linear(L1_OUT, L2_OUT) if L2_OUT else None
        self.l4 = nn.Linear(L2_OUT, L3_OUT) if L3_OUT else None
        self.out = nn.Linear(LAST_HIDDEN, 1)
        # 補助ヘッド（ADR-0129）。FT出力からの線形1層に限る。深くすると
        # ヘッド自身がタスクを解いてしまい、FTへ表現を押し込む圧力が弱まる
        self.policy_from = nn.Linear(CONCAT, MOVE_FROM_CLASSES) if policy else None
        self.policy_to = nn.Linear(CONCAT, MOVE_TO_CLASSES) if policy else None
        # FT事前学習では評価値ヘッドも線形1層にする（ADR-0129）。深い
        # ヘッドはタスクを自分で解いてしまい、FTへ表現を押し込む圧力が
        # 弱まる。評価値を当てる圧力自体は残す
        self.pretrain_value = nn.Linear(CONCAT, 1) if pretrain else None
        # 表現蒸留の写像（ADR-0132）。生徒のFT出力から教師のFT出力へ当てる。
        # 線形1層に限るのは補助ヘッドと同じ理由で、写像自身が圧縮を解いて
        # しまうとFTへ表現を押し込む圧力が弱まるためである。
        # 書き出しには載らないので推論は変わらない
        self.distill = nn.Linear(CONCAT, distill_out) if distill_out else None
        # 利き予測ヘッド（ADR-0133）。FT出力から升ごとの利き数を当てる。
        # 線形1層と2層MLPのどちらが良い表現を作るかは決め打たず、比較軸に
        # する。深いヘッドは遮りの論理積を自分で計算できてしまう一方、
        # SimCLRは非線形の写像のほうが良い表現になると報告している。
        # このヘッドも書き出しには載らないので推論は変わらない
        self.effect = self._build_effect_head(effect_head)
        self._init_weights()

    @staticmethod
    def _build_effect_head(kind):
        if kind is None:
            return None
        if kind == "linear":
            return nn.Linear(CONCAT, EFFECT_OUT)
        if kind == "mlp":
            return nn.Sequential(
                nn.Linear(CONCAT, EFFECT_MLP_HIDDEN),
                nn.ReLU(),
                nn.Linear(EFFECT_MLP_HIDDEN, EFFECT_OUT),
            )
        raise ValueError(f"利きヘッドの種類が不明: {kind}")

    def _init_weights(self):
        nn.init.uniform_(self.ft.weight, -0.05, 0.05)
        if self.ft_p is not None:
            nn.init.zeros_(self.ft_p.weight)
        nn.init.uniform_(self.l2.weight, -0.1, 0.1)
        nn.init.zeros_(self.l2.bias)
        if self.l3 is not None:
            nn.init.uniform_(self.l3.weight, -0.3, 0.3)
            nn.init.zeros_(self.l3.bias)
        if self.l4 is not None:
            nn.init.uniform_(self.l4.weight, -0.3, 0.3)
            nn.init.zeros_(self.l4.bias)
        nn.init.uniform_(self.out.weight, -0.3, 0.3)
        nn.init.zeros_(self.out.bias)
        heads = [self.policy_from, self.policy_to, self.pretrain_value,
                 self.distill]
        # 利きヘッドはMLPのこともある。線形層を取り出して同じ初期化を当てる
        if self.effect is not None:
            heads.extend(m for m in self.effect.modules() if isinstance(m, nn.Linear))
        for head in heads:
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
        return (w.view(KING_BUCKETS, FE_END, FT_OUT) + virtual.unsqueeze(0)).view(
            FT_IN, FT_OUT
        )

    def transform_both(self, stm_idx, stm_off, opp_idx, opp_off):
        """FT出力を2視点ぶん連結して返す。補助ヘッドもここから生やす。"""
        z_stm = self.transform(stm_idx, stm_off)
        z_opp = self.transform(opp_idx, opp_off)
        return torch.cat([z_stm.clamp(0.0, 1.0), z_opp.clamp(0.0, 1.0)], dim=1)

    def value(self, x):
        """FT出力の連結から評価値を出す（推論と同じ経路）。

        事前学習中は線形1層で代替する。この層は書き出しに載らない。
        """
        if self.pretrain_value is not None:
            return self.pretrain_value(x).squeeze(1)
        h = self.l2(x).clamp(0.0, 1.0)
        for layer in (self.l3, self.l4):
            if layer is not None:
                h = layer(h).clamp(0.0, 1.0)
        return self.out(h).squeeze(1)

    def forward(self, stm_idx, stm_off, opp_idx, opp_off):
        return self.value(self.transform_both(stm_idx, stm_off, opp_idx, opp_off))

    def clip_ft_weights(self, limit=1.0):
        """畳み込み後のFT重みを±limitへ収める（ADR-0138）。

        書き出しに使うのは `folded_ft_weight()` の値なので、制約は畳み込み後に
        掛ける必要がある。超過分は実特徴側（`ft`）から引く。仮想特徴（`ft_p`）は
        全ての玉バケットで共有されるため、そちらを動かすと無関係な升へ波及する。

        i8で格納すると量子化値が±127に収まらない重みは飽和する。飽和は
        0.055%（1800個に1個）でも-59.3 Eloになる（ADR-0138のリーグ戦）。
        一方、飽和がなければ刻みを5倍粗くしても差が出ない。**効くのは
        飽和の有無であって刻みの細かさではない。**
        """
        with torch.no_grad():
            folded = self.folded_ft_weight()
            self.ft.weight.sub_(folded - folded.clamp(-limit, limit))

    def clip_weights(self):
        with torch.no_grad():
            self.l2.weight.clamp_(-HIDDEN_W_LIMIT, HIDDEN_W_LIMIT)
            if self.l3 is not None:
                self.l3.weight.clamp_(-HIDDEN_W_LIMIT, HIDDEN_W_LIMIT)
            if self.l4 is not None:
                self.l4.weight.clamp_(-HIDDEN_W_LIMIT, HIDDEN_W_LIMIT)
            self.out.weight.clamp_(-OUT_W_LIMIT, OUT_W_LIMIT)


def loss_fn(output, target):
    return F.binary_cross_entropy_with_logits(output, target)


def effect_loss_fn(pred, eff_short, eff_long):
    """利き数の回帰損失を、短い利きと長い利きに分けて返す（ADR-0133）。

    ラベルは升ごとの利き数（u8）で、EFFECT_SCALEで割って正規化してから
    当てる。損失は升ごとのMSEである。

    短い利きは加法で解けるので、ほぼ0まで落ちるはずである。落ちなければ
    実装かλがおかしい。長い利きは遮りが絡んで加法では書けず、こちらが
    FTの容量を測る本体になる。だから2つを混ぜずに返す。
    """
    target = torch.cat([eff_short, eff_long], dim=1).float() / EFFECT_SCALE
    short_loss = F.mse_loss(pred[:, :EFFECT_LEN], target[:, :EFFECT_LEN])
    long_loss = F.mse_loss(pred[:, EFFECT_LEN:], target[:, EFFECT_LEN:])
    return short_loss, long_loss
