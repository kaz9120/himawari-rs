"""NNUE model definition.

HalfKPに、利きを入力にする側のaccumulatorを足した構成を学習する。
次元と構成はRust側のビルド（crates/engine/build.rs）から読む。
"""

import torch
import torch.nn as nn
import torch.nn.functional as F

import himawari

# 次元はRust側のビルドから読む。
# HIMAWARI_ARCH を指定してビルドしたhimawariモジュールを入れれば、
# 学習側も同じ構成になる。定数を二重に持たないので、推論と学習の
# 取り違えが起きない
FT_IN = himawari.FT_IN
FT_OUT = himawari.FT_OUT
L1_OUT = himawari.L1_OUT
L2_OUT = himawari.L2_OUT
# 0なら隠れ層1つ
LAST_HIDDEN = L2_OUT or L1_OUT
ARCH = himawari.ARCH
FE_END = FT_IN // 81
# 第1層の入力幅。片視点のFT出力を2つに割って掛けるので、視点あたり
# FT_OUT//2 になる。Rust側の定数をそのまま使う
CONCAT = himawari.CONCAT
# HalfKPの片視点の活性の幅。利きのaccumulatorを持つ構成では、CONCATの後ろに
# 利きの活性が2視点ぶん続くので、CONCAT//2 ではなくFT_OUTから決める
HALF = FT_OUT // 2
# 利きのaccumulator（F2）。構成の末尾の e<E> で幅が決まり、0なら
# 持たない。古い拡張は定数を持たないので0として扱う
EFF_IN = getattr(himawari, "EFF_IN", 0)
EFF_OUT = getattr(himawari, "EFF_OUT", 0)
# F2と最小の攻め駒の組。側の入力を `side_feat` の種類で作る。
# 学習側は表の末尾に詰め物の行（添字 EFF_IN）を1本足して持ち、書き出しで落とす
EFF_GENERIC = getattr(himawari, "EFF_GENERIC", False)
# 1局面1視点で並ぶ利きの特徴の数。F2は升ごとに1つで81、F2と最小の攻め駒の組は上限の数
EFF_ACTIVE = getattr(himawari, "EFF_ACTIVE", 81)
# 利きの特徴の使い方。f2は特徴を立てる処置群、noneは立てない統制群。
# F2と最小の攻め駒の組でも、f2は「構成の表し方で立てる」の意味になる
EFFECT_INPUTS = ("f2", "none")
# 積の値域合わせ。推論側は clip(a)*clip(b) >> 7 で、最大が
# 127×127/128 = 126 にしかならない。学習側にも同じ127/128を掛けておくと、
# 活性127・隠れ層64という量子化の係数を1つも変えずに済む
PAIR_SCALE = 127.0 / 128.0

# Evaluation scale
SIGMOID_SCALE = 600.0
FV_SCALE = 16

# Quantization limits (model.rs:18-21)
HIDDEN_W_LIMIT = 127.0 / 64.0
OUT_W_SCALE = SIGMOID_SCALE * FV_SCALE / 127.0
OUT_W_LIMIT = 127.0 / OUT_W_SCALE


class NnueModel(nn.Module):
    """HalfKPのNNUE。

    `factorized=True` のとき、学習時だけ玉位置に依らない駒の特徴
    （BonaPiece単独）を並列に持つ。`halfkp_index` は
    `king * FE_END + bona_piece` なので、実特徴のインデックスを
    FE_ENDで割った余りが仮想特徴のインデックスになる。
    推論側の構造は変わらない。重みは書き出し時に畳み込む。
    """

    def __init__(self, sparse_ft=True, factorized=False, effect_inputs=None):
        super().__init__()
        # 利きのaccumulatorは構成で決まる。持つ構成では、統制群も
        # 処置群も同じ層を作る。**構造が同じなので、初期化で乱数を消費する
        # 順序も同じになる**。違いは特徴を立てるかどうかだけである
        if (EFF_OUT > 0) != (effect_inputs is not None):
            raise ValueError(
                f"構成 {ARCH} の利きの幅は{EFF_OUT}。利きの構成では effect_inputs"
                f"（{'/'.join(EFFECT_INPUTS)}）が要り、それ以外では渡せない"
            )
        if effect_inputs is not None and effect_inputs not in EFFECT_INPUTS:
            raise ValueError(f"effect_inputs が不明: {effect_inputs}")
        self.eff_on = effect_inputs == "f2"
        self.ft = nn.EmbeddingBag(FT_IN, FT_OUT, mode="sum", sparse=sparse_ft)
        self.ft_p = (
            nn.EmbeddingBag(FE_END, FT_OUT, mode="sum", sparse=sparse_ft)
            if factorized
            else None
        )
        self.ft_bias = nn.Parameter(torch.full((FT_OUT,), 0.5))
        # 利きの特徴は玉位置に依らないので、factorizerは掛けない。
        # 1局面で81行が立ち、表の大半の行が毎バッチ触られるので密の勾配にする
        self.eff = (
            nn.EmbeddingBag(
                EFF_IN + 1 if EFF_GENERIC else EFF_IN, EFF_OUT, mode="sum",
                padding_idx=EFF_IN if EFF_GENERIC else None,
            )
            if EFF_OUT
            else None
        )
        self.eff_bias = (
            nn.Parameter(torch.full((EFF_OUT,), 0.5)) if EFF_OUT else None
        )
        self.l2 = nn.Linear(CONCAT, L1_OUT)
        # 隠れ層は書いたぶんだけ持つ。L2_OUT=0 なら隠れ層1つ
        self.l3 = nn.Linear(L1_OUT, L2_OUT) if L2_OUT else None
        self.out = nn.Linear(LAST_HIDDEN, 1)
        self._init_weights()

    def _init_weights(self):
        nn.init.uniform_(self.ft.weight, -0.05, 0.05)
        if self.ft_p is not None:
            nn.init.zeros_(self.ft_p.weight)
        if self.eff is not None:
            # 統制群も乱数を引いてから捨てる。引かないと以降の層の初期値が
            # 処置群とずれる
            nn.init.uniform_(self.eff.weight, -0.05, 0.05)
            if EFF_GENERIC:
                # 詰め物の行は0に保つ（padding_idxは勾配を止めるだけで、初期値は引いた乱数のまま）
                with torch.no_grad():
                    self.eff.weight[EFF_IN].zero_()
            if not self.eff_on:
                with torch.no_grad():
                    self.eff.weight.zero_()
                # 統制群の表は特徴を受けないので学習もしない。ゼロのまま
                # 書き出すと、推論側で特徴を数えても足すのは0になる
                self.eff.weight.requires_grad_(False)
        nn.init.uniform_(self.l2.weight, -0.1, 0.1)
        nn.init.zeros_(self.l2.bias)
        if self.l3 is not None:
            nn.init.uniform_(self.l3.weight, -0.3, 0.3)
            nn.init.zeros_(self.l3.bias)
        nn.init.uniform_(self.out.weight, -0.3, 0.3)
        nn.init.zeros_(self.out.bias)

    def transform(self, idx, off):
        z = self.ft(idx, off)
        if self.ft_p is not None:
            z = z + self.ft_p(idx % FE_END, off)
        return z + self.ft_bias

    def folded_eff_weight(self):
        """側の表から詰め物の行を落として返す（書き出し用）。"""
        return self.eff.weight.detach().float()[:EFF_IN].clone()

    def folded_ft_weight(self):
        """仮想特徴を畳み込んだFT重みを返す（書き出し用）。"""
        w = self.ft.weight.detach().float()
        if self.ft_p is None:
            return w
        virtual = self.ft_p.weight.detach().float()
        return (w.view(81, FE_END, FT_OUT) + virtual.unsqueeze(0)).view(FT_IN, FT_OUT)

    @staticmethod
    def pair_activation(z):
        """FT出力の前半と後半を要素ごとに掛ける。

        駒対の相互作用を低ランクで持つための積で、展開すると
        `Σ_{i,m} W_ji W'_jm x_i x_m` になる。入力の直積を作らずに済む。
        """
        a, b = z[:, :HALF].clamp(0.0, 1.0), z[:, HALF:].clamp(0.0, 1.0)
        return a * b * PAIR_SCALE

    def effect_activation(self, idx, n):
        """利きのaccumulatorを0〜1へクリップして返す。対の積は掛けない。

        `idx` は1局面81個ずつ並んだ特徴。統制群は特徴を立てず、バイアスだけの
        定数になる。推論側の `effect_transform` と同じ形である。
        """
        if self.eff_on:
            if idx is None or idx.numel() != n * EFF_ACTIVE:
                raise ValueError(
                    "利きの特徴がバッチに入っていない。ローダに effect_inputs を渡す"
                )
            z = self.eff(idx.view(n, EFF_ACTIVE)) + self.eff_bias
        else:
            z = self.eff_bias.expand(n, -1)
        return z.clamp(0.0, 1.0)

    def transform_both(self, stm_idx, stm_off, opp_idx, opp_off,
                       eff_stm=None, eff_opp=None):
        """FT出力を2視点ぶん連結して返す。

        利きのaccumulatorを持つ構成では、HalfKPの活性の後ろに利きの活性を
        手番視点・非手番視点の順に連結する。
        """
        z_stm = self.transform(stm_idx, stm_off)
        z_opp = self.transform(opp_idx, opp_off)
        parts = [self.pair_activation(z_stm), self.pair_activation(z_opp)]
        if self.eff is not None:
            n = stm_off.size(0)
            parts += [self.effect_activation(eff_stm, n),
                      self.effect_activation(eff_opp, n)]
        return torch.cat(parts, dim=1)

    def value(self, x):
        """FT出力の連結から評価値を出す（推論と同じ経路）。"""
        h = self.l2(x).clamp(0.0, 1.0)
        if self.l3 is not None:
            h = self.l3(h).clamp(0.0, 1.0)
        return self.out(h).squeeze(1)

    def forward(self, stm_idx, stm_off, opp_idx, opp_off, eff_stm=None, eff_opp=None):
        return self.value(
            self.transform_both(stm_idx, stm_off, opp_idx, opp_off, eff_stm, eff_opp)
        )

    def clip_ft_weights(self, limit=1.0):
        """畳み込み後のFT重みを±limitへ収める。

        書き出しに使うのは `folded_ft_weight()` の値なので、制約は畳み込み後に
        掛ける必要がある。超過分は実特徴側（`ft`）から引く。仮想特徴（`ft_p`）は
        81の玉位置で共有されるため、そちらを動かすと無関係な升へ波及する。

        i8で格納すると量子化値が±127に収まらない重みは飽和する。飽和は
        0.055%（1800個に1個）でも-59.3 Eloになる。
        一方、飽和がなければ刻みを5倍粗くしても差が出ない。**効くのは
        飽和の有無であって刻みの細かさではない。**
        """
        with torch.no_grad():
            folded = self.folded_ft_weight()
            self.ft.weight.sub_(folded - folded.clamp(-limit, limit))

    def clip_eff_weights(self, limit):
        """側の表の重み（畳み込み後）とバイアスを±limitへ収める。

        差分で持つ側のaccをi16にするための制約である。F2と最小の攻め駒の組では1視点に
        162行が立つので、重みとバイアスが±limitに収まれば、accの絶対値は量子化の127倍で
        `127 × 163 × limit` 以下になる。limit=1.5なら31,051で、i16の上限32,767に収まる。
        """
        if self.eff is None:
            return
        with torch.no_grad():
            self.eff.weight[:EFF_IN].clamp_(-limit, limit)
            self.eff_bias.clamp_(-limit, limit)

    def clip_weights(self):
        with torch.no_grad():
            self.l2.weight.clamp_(-HIDDEN_W_LIMIT, HIDDEN_W_LIMIT)
            if self.l3 is not None:
                self.l3.weight.clamp_(-HIDDEN_W_LIMIT, HIDDEN_W_LIMIT)
            self.out.weight.clamp_(-OUT_W_LIMIT, OUT_W_LIMIT)


def loss_fn(output, target):
    return F.binary_cross_entropy_with_logits(output, target)
