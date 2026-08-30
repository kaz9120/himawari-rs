"""疎な特徴向けのoptimizer（ADR-0064）。"""

import math

import torch
from torch.optim import Optimizer


class MaskedAdam(Optimizer):
    """そのステップで出現した行だけを更新するAdam。

    `SparseAdam` と同じ意味論を、denseテンソルのまま実現する。
    疎勾配を作らないためMPSで動き、coalesceも要らない。

    出現しなかった行はモーメントも重みも据え置く。素の `Adam` は
    これらの行もモーメントの慣性で動かしてしまい、HalfKPのように
    特徴が疎な場合に稀な行の重みを劣化させる。

    bias correctionは `SparseAdam` に合わせてグローバルなstepで行う。
    出現の判定は勾配の行が全要素ゼロかどうかで見る。EmbeddingBagの
    denseな逆伝播では、出現しない行はきっちりゼロになる。

    更新はin-placeで書く。`torch.where` と非in-placeの式で書くと、FT規模
    （13万行×1024列、f32で513MB）のテンポラリを毎ステップ7個ほど作り、
    メモリ帯域で律速する。マスクを係数 `1-(1-β)·active` に畳んで
    `mul_`・`addcmul_`・`addcdiv_` で書くと更新値は変わらないまま
    テンポラリが2個に減る。MPSの実測で1ステップ106msが53msになった
    （issue #409）。不活性行は勾配が全要素ゼロなので、`add_` と
    `addcmul_` が足すのは0で、モーメントも重みも据え置きが保たれる。
    """

    def __init__(self, params, lr=1e-3, betas=(0.9, 0.999), eps=1e-8):
        if lr < 0.0:
            raise ValueError(f"lrが負: {lr}")
        super().__init__(params, dict(lr=lr, betas=betas, eps=eps))

    @torch.no_grad()
    def step(self, closure=None):
        loss = None
        if closure is not None:
            with torch.enable_grad():
                loss = closure()

        for group in self.param_groups:
            beta1, beta2 = group["betas"]
            lr, eps = group["lr"], group["eps"]
            for p in group["params"]:
                grad = p.grad
                if grad is None:
                    continue
                if grad.is_sparse:
                    raise RuntimeError("MaskedAdamは疎勾配を受け取らない")

                state = self.state[p]
                if len(state) == 0:
                    state["step"] = 0
                    state["exp_avg"] = torch.zeros_like(p)
                    state["exp_avg_sq"] = torch.zeros_like(p)
                state["step"] += 1
                m, v = state["exp_avg"], state["exp_avg_sq"]

                # 行単位の出現マスク。2次元の重み以外は全要素を対象にする
                if grad.dim() >= 2:
                    active = (grad != 0).any(dim=1, keepdim=True)
                else:
                    active = grad != 0
                af = active.to(grad.dtype)

                # 活性なら m·β1 + g·(1-β1)、不活性なら af=0 かつ g=0 で
                # m がそのまま残る。v も同じ形になる
                m.mul_(1.0 - (1.0 - beta1) * af).add_(grad, alpha=1.0 - beta1)
                v.mul_(1.0 - (1.0 - beta2) * af).addcmul_(
                    grad, grad, value=1.0 - beta2
                )

                # bias correctionの掛け方は SparseAdam に合わせる。
                # epsを平方根の外へ出す点が torch.optim.Adam と違う
                bc1 = 1.0 - beta1 ** state["step"]
                bc2 = 1.0 - beta2 ** state["step"]
                step_size = lr * math.sqrt(bc2) / bc1
                denom = v.sqrt().add_(eps)
                # 分子に af を掛けて不活性行の更新を0にする。m.mul_(af) と
                # 書くと状態の m まで消えるので、ここだけテンポラリを許す
                p.addcdiv_(m * af, denom, value=-step_size)

        return loss
