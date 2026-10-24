"""NNUE model definition (ADR-0034, ADR-0040, ADR-0045).

ADR-0045で利き入力を除去し、純粋HalfKP構成にした。
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
# 焦点のヒートマップの長さ（ADR-0213）。盤の81升をそのまま並べる
FOCUS_OUT = 81
# 的中率を測る上位マスの数（ADR-0213の測定の設計）
FOCUS_TOPK = 5
ARCH = himawari.ARCH
FE_END = FT_IN // 81
# 第1層の入力幅。片視点のFT出力を2つに割って掛けるので、視点あたり
# FT_OUT//2 になる（ADR-0171）。Rust側の定数をそのまま使う
CONCAT = himawari.CONCAT
# HalfKPの片視点の活性の幅。利きのaccumulatorを持つ構成では、CONCATの後ろに
# 利きの活性が2視点ぶん続くので、CONCAT//2 ではなくFT_OUTから決める
HALF = FT_OUT // 2
# 利きのaccumulator（ADR-0237のF2）。構成の末尾の e<E> で幅が決まり、0なら
# 持たない。古い拡張は定数を持たないので0として扱う
EFF_IN = getattr(himawari, "EFF_IN", 0)
EFF_OUT = getattr(himawari, "EFF_OUT", 0)
# 表し方を替えた構成（ADR-0245）。側の入力を `side_feat` の種類で作り、対の積を掛けることがある。
# 学習側は表の末尾に詰め物の行（添字 EFF_IN）を1本足して持ち、書き出しで落とす
EFF_GENERIC = getattr(himawari, "EFF_GENERIC", False)
EFF_PAIR = getattr(himawari, "EFF_PAIR", False)
# 1局面1視点で並ぶ利きの特徴の数。F2は升ごとに1つで81、表し方を替えた構成は上限の数
EFF_ACTIVE = getattr(himawari, "EFF_ACTIVE", 81)
# 側の表のうち、HalfKP（玉×BonaPiece）の区画の起点（ADR-0245の習甦方式）。無ければ-1。
# factorizerを使う学習では、この区画にも玉位置に依らない駒の成分を足す（ADR-0066）
EFF_KP_BASE = getattr(himawari, "EFF_KP_BASE", -1)
# 玉の近傍の部分モデル（ADR-0246）。構成名の k<K> で出力の幅が決まり、0なら持たない。
# 入力は側の入力の1行の後ろに玉1つぶん SUB_LEN 個つながって届く。前の SUB_WIN 個が
# 升のトークン、続く7個が相手の持ち駒の枚数、最後が玉の側の手番かである
SUB_OUT = getattr(himawari, "SUB_OUT", 0)
SUB_LEN = getattr(himawari, "SUB_LEN", 0) if SUB_OUT else 0
SUB_SLOT = getattr(himawari, "SUB_SLOT", 0)
SUB_WIN = getattr(himawari, "SUB_WIN", 25)
SUB_TOKENS = getattr(himawari, "SUB_TOKENS", 480)
SUB_EMB = getattr(himawari, "SUB_EMB", 16)
SUB_EXTRA = getattr(himawari, "SUB_EXTRA", 8)
SUB_HID = getattr(himawari, "SUB_HID", 32)
SUB_HAND_SCALE = getattr(himawari, "SUB_HAND_SCALE", [1.0] * 7)
# 利きの特徴の使い方。f2は特徴を立てる処置群、noneは立てない統制群（ADR-0237）。
# 表し方を替えた構成（ADR-0245）でも、f2は「構成の表し方で立てる」の意味になる
EFFECT_INPUTS = ("f2", "none")
# 積の値域合わせ（ADR-0171）。推論側は clip(a)*clip(b) >> 7 で、最大が
# 127×127/128 = 126 にしかならない。学習側にも同じ127/128を掛けておくと、
# 活性127・隠れ層64という量子化の係数を1つも変えずに済む
PAIR_SCALE = 127.0 / 128.0

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
                 pretrain=False, effect_head=None,
                 focus_head=None, effect_inputs=None, halfkp_inputs=True):
        super().__init__()
        # 利きのaccumulatorは構成で決まる（ADR-0237）。持つ構成では、統制群も
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
        # HalfKPの特徴を立てない置き換えの群（ADR-0245）。FTの表を0で凍結し、主の
        # accumulatorをバイアスだけの定数にする。推論側はFTの0の行を足すだけになる
        self.halfkp_on = halfkp_inputs
        self.ft = nn.EmbeddingBag(FT_IN, FT_OUT, mode="sum", sparse=sparse_ft)
        self.ft_p = (
            nn.EmbeddingBag(FE_END, FT_OUT, mode="sum", sparse=sparse_ft)
            if factorized
            else None
        )
        self.ft_bias = nn.Parameter(torch.full((FT_OUT,), 0.5))
        # 利きの特徴は玉位置に依らないので、factorizerは掛けない（ADR-0237）。
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
        # 側に入れたHalfKPのfactorizer（ADR-0245）。主のFTと同じく、学習時だけ駒単独の
        # 成分を81の玉位置で共有し、書き出しで畳み込む。末尾は詰め物の行
        self.eff_p = (
            nn.EmbeddingBag(FE_END + 1, EFF_OUT, mode="sum", padding_idx=FE_END)
            if factorized and EFF_KP_BASE >= 0
            else None
        )
        # 玉の近傍の部分モデル（ADR-0246）。乱数は後段の初期化の後で引くので、
        # 持たない構成の乱数の消費順は変わらない
        self.sub = SubModel() if SUB_OUT else None
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
        # 利き予測ヘッド（ADR-0133）。FT出力から升ごとの利き数を当てる。
        # 線形1層と2層MLPのどちらが良い表現を作るかは決め打たず、比較軸に
        # する。深いヘッドは遮りの論理積を自分で計算できてしまう一方、
        # SimCLRは非線形の写像のほうが良い表現になると報告している。
        # このヘッドも書き出しには載らないので推論は変わらない
        self.effect = self._build_effect_head(effect_head)
        # 焦点のヒートマップを当てるヘッド（ADR-0213）。利きヘッドと同じCONCATから
        # 生やす線形1層で、probeの指標になる。深くするとヘッド自身が
        # タスクを解いてしまい、「FTが焦点を持っているか」を測れなくなる。
        # 書き出しには載らないので推論は変わらない
        self.focus = self._build_focus_head(focus_head)
        self._init_weights()

    @staticmethod
    def _build_focus_head(kind):
        if kind is None:
            return None
        if kind == "linear":
            return nn.Linear(CONCAT, FOCUS_OUT)
        if kind == "mlp":
            # 線形で当たらない焦点が非線形なら当たるかを分けるための対照。
            # 中間は利きヘッドのmlpと同じ256にする
            return nn.Sequential(
                nn.Linear(CONCAT, EFFECT_MLP_HIDDEN),
                nn.ReLU(),
                nn.Linear(EFFECT_MLP_HIDDEN, FOCUS_OUT),
            )
        raise ValueError(f"焦点ヘッドの種類が不明: {kind}")

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
        if not self.halfkp_on:
            # 乱数は引いてから捨てる。以降の層の初期値を処置群と揃える（ADR-0237の統制と同じ）
            with torch.no_grad():
                self.ft.weight.zero_()
            self.ft.weight.requires_grad_(False)
            if self.ft_p is not None:
                self.ft_p.weight.requires_grad_(False)
        if self.eff is not None:
            # 統制群も乱数を引いてから捨てる。引かないと以降の層の初期値が
            # 処置群とずれる（ADR-0237の統制、ADR-0158の教訓）
            nn.init.uniform_(self.eff.weight, -0.05, 0.05)
            if EFF_GENERIC:
                # 詰め物の行は0に保つ（padding_idxは勾配を止めるだけで、初期値は引いた乱数のまま）
                with torch.no_grad():
                    self.eff.weight[EFF_IN].zero_()
            if self.eff_p is not None:
                nn.init.zeros_(self.eff_p.weight)
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
        if self.l4 is not None:
            nn.init.uniform_(self.l4.weight, -0.3, 0.3)
            nn.init.zeros_(self.l4.bias)
        nn.init.uniform_(self.out.weight, -0.3, 0.3)
        nn.init.zeros_(self.out.bias)
        heads = [self.policy_from, self.policy_to, self.pretrain_value]
        # 利きヘッドと焦点ヘッドはMLPのこともある。線形層を取り出して同じ初期化を当てる
        for head in (self.effect, self.focus):
            if head is not None:
                heads.extend(m for m in head.modules() if isinstance(m, nn.Linear))
        for head in heads:
            if head is not None:
                nn.init.uniform_(head.weight, -0.05, 0.05)
                nn.init.zeros_(head.bias)
        if self.sub is not None:
            self.sub.init_weights()

    def transform(self, idx, off):
        z = self.ft(idx, off)
        if self.ft_p is not None:
            z = z + self.ft_p(idx % FE_END, off)
        return z + self.ft_bias

    def folded_eff_weight(self):
        """側の表から詰め物の行を落とし、HalfKPの区画にfactorizerを畳み込んで返す（書き出し用）。"""
        w = self.eff.weight.detach().float()[:EFF_IN].clone()
        if self.eff_p is not None:
            virtual = self.eff_p.weight.detach().float()[:FE_END]
            seg = w[EFF_KP_BASE:EFF_KP_BASE + 81 * FE_END].view(81, FE_END, EFF_OUT)
            seg += virtual.unsqueeze(0)
        return w

    def folded_ft_weight(self):
        """仮想特徴を畳み込んだFT重みを返す（書き出し用）。"""
        w = self.ft.weight.detach().float()
        if self.ft_p is None:
            return w
        virtual = self.ft_p.weight.detach().float()
        return (w.view(81, FE_END, FT_OUT) + virtual.unsqueeze(0)).view(FT_IN, FT_OUT)

    @staticmethod
    def pair_activation(z):
        """FT出力の前半と後半を要素ごとに掛ける（ADR-0171）。

        駒対の相互作用を低ランクで持つための積で、展開すると
        `Σ_{i,m} W_ji W'_jm x_i x_m` になる。入力の直積を作らずに済む。
        """
        a, b = z[:, :HALF].clamp(0.0, 1.0), z[:, HALF:].clamp(0.0, 1.0)
        return a * b * PAIR_SCALE

    def effect_activation(self, idx, n):
        """利きのaccumulatorを0〜1へクリップして返す（ADR-0237）。対の積は掛けない。

        `idx` は1局面81個ずつ並んだ特徴。統制群は特徴を立てず、バイアスだけの
        定数になる。推論側の `effect_transform` と同じ形である。
        """
        if self.eff_on:
            if idx is None or idx.numel() != n * (EFF_ACTIVE + SUB_LEN):
                raise ValueError(
                    "利きの特徴がバッチに入っていない。ローダに effect_inputs を渡す"
                )
            v = idx.view(n, EFF_ACTIVE + SUB_LEN)[:, :EFF_ACTIVE]
            z = self.eff(v) + self.eff_bias
            if self.eff_p is not None:
                kp = (v >= EFF_KP_BASE) & (v < EFF_KP_BASE + 81 * FE_END)
                z = z + self.eff_p(torch.where(kp, (v - EFF_KP_BASE) % FE_END, FE_END))
        else:
            z = self.eff_bias.expand(n, -1)
        if EFF_PAIR:
            # 側でも前半と後半を掛ける（ADR-0245）。推論側の `side_feat::fill` と同じ形
            h = EFF_OUT // 2
            return z[:, :h].clamp(0.0, 1.0) * z[:, h:].clamp(0.0, 1.0) * PAIR_SCALE
        return z.clamp(0.0, 1.0)

    def transform_both(self, stm_idx, stm_off, opp_idx, opp_off,
                       eff_stm=None, eff_opp=None):
        """FT出力を2視点ぶん連結して返す。補助ヘッドもここから生やす。

        利きのaccumulatorを持つ構成では、HalfKPの活性の後ろに利きの活性を
        手番視点・非手番視点の順に連結する（ADR-0237）。
        """
        z_stm = self.transform(stm_idx, stm_off)
        z_opp = self.transform(opp_idx, opp_off)
        parts = [self.pair_activation(z_stm), self.pair_activation(z_opp)]
        if self.eff is not None:
            n = stm_off.size(0)
            parts += [self.effect_activation(eff_stm, n),
                      self.effect_activation(eff_opp, n)]
        if self.sub is not None:
            # 手番側の玉・非手番側の玉の順。余りの列は0で、推論側も0を置く
            n = stm_off.size(0)
            for idx in (eff_stm, eff_opp):
                if idx is None:
                    raise ValueError("部分モデルの入力がバッチに入っていない。ローダに effect_inputs を渡す")
                parts.append(self.sub(idx.view(n, EFF_ACTIVE + SUB_LEN)[:, EFF_ACTIVE:]))
            pad = SUB_SLOT - 2 * SUB_OUT
            if pad:
                parts.append(z_stm.new_zeros(n, pad))
        return torch.cat(parts, dim=1)

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

    def forward(self, stm_idx, stm_off, opp_idx, opp_off, eff_stm=None, eff_opp=None):
        return self.value(
            self.transform_both(stm_idx, stm_off, opp_idx, opp_off, eff_stm, eff_opp)
        )

    def clip_ft_weights(self, limit=1.0):
        """畳み込み後のFT重みを±limitへ収める（ADR-0138）。

        書き出しに使うのは `folded_ft_weight()` の値なので、制約は畳み込み後に
        掛ける必要がある。超過分は実特徴側（`ft`）から引く。仮想特徴（`ft_p`）は
        81の玉位置で共有されるため、そちらを動かすと無関係な升へ波及する。

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


class SubModel(nn.Module):
    """玉の近傍の部分モデル（ADR-0246）。推論側は `crates/engine/src/submodel.rs`。

    升のトークンを16次元へ埋め込んで位置の順に並べ、相手の持ち駒と手番を足して
    32→`SUB_OUT` のMLPに通す。出力は0〜1へクリップし、FTの活性と同じ縮尺（1が127）で
    連結ベクトルへ入る。推論側は埋め込みと第1層を畳み込んだ表で同じ値を出す。
    """

    def __init__(self):
        super().__init__()
        self.emb = nn.Embedding(SUB_TOKENS, SUB_EMB)
        self.l1 = nn.Linear(SUB_WIN * SUB_EMB + SUB_EXTRA, SUB_HID)
        self.l2 = nn.Linear(SUB_HID, SUB_OUT)
        self.register_buffer(
            "hand_scale", torch.tensor(SUB_HAND_SCALE, dtype=torch.float32),
            persistent=False,
        )

    def init_weights(self):
        nn.init.uniform_(self.emb.weight, -0.5, 0.5)
        nn.init.uniform_(self.l1.weight, -0.1, 0.1)
        nn.init.zeros_(self.l1.bias)
        nn.init.uniform_(self.l2.weight, -0.3, 0.3)
        nn.init.zeros_(self.l2.bias)

    def forward(self, v):
        """`v` は1局面1玉ぶん `SUB_LEN` 個の整数（推論側の `submodel::inputs` と同じ並び）。"""
        n = v.size(0)
        tok = self.emb(v[:, :SUB_WIN]).view(n, SUB_WIN * SUB_EMB)
        hand = v[:, SUB_WIN:SUB_WIN + 7].float() / self.hand_scale
        stm = v[:, SUB_WIN + 7:SUB_WIN + 8].float()
        h = self.l1(torch.cat([tok, hand, stm], dim=1)).clamp(0.0, 1.0)
        return self.l2(h).clamp(0.0, 1.0)

    def raw_weights(self):
        """推論側のファイルの形（埋め込み・第1層の重みとバイアス・第2層の重みとバイアスの順）で返す。"""
        return torch.cat([
            p.detach().float().flatten()
            for p in (self.emb.weight, self.l1.weight, self.l1.bias,
                      self.l2.weight, self.l2.bias)
        ])

    def load_raw(self, raw):
        """`raw_weights` の逆。"""
        at = 0
        with torch.no_grad():
            for p in (self.emb.weight, self.l1.weight, self.l1.bias,
                      self.l2.weight, self.l2.bias):
                p.copy_(raw[at:at + p.numel()].view_as(p))
                at += p.numel()


def loss_fn(output, target):
    return F.binary_cross_entropy_with_logits(output, target)


def focus_loss_fn(pred, heat):
    """焦点のヒートマップのBCE（ADR-0213）。

    的は升ごとの0/1で、「続くk手で駒が動いたか取られたか」を表す。
    81升を独立に当てるので、多ラベル分類のBCEになる。
    """
    return F.binary_cross_entropy_with_logits(pred, heat.float())


def top_k_precision(pred, heat, k=FOCUS_TOPK):
    """局面ごとに予測の上位k升を取り、ヒートマップが1だった割合を平均する。

    ADR-0213がprobeの指標に選んだ「上位5マスの的中率」である。**ベースラインも
    同じ関数で測る。** 局面によらない予測（頻度ベースライン）を全局面へ広げて渡せば、
    どの局面でも同じk升が選ばれ、比較の条件が揃う。
    """
    idx = pred.topk(k, dim=1).indices
    return heat.gather(1, idx).float().mean()


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
