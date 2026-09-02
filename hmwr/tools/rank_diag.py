#!/usr/bin/env python3
"""ランキング損失のヒンジが何に発火しているかを分ける（ADR-0196）。

[ADR-0185](../../docs/adr/0185-sibling-ranking-loss.md)のヒンジは、正例の葉が
負例の葉より親視点で良いことを課す。学習を最後まで回しても発火率は2割弱で
下げ止まる。下げ止まりの中身には3つの見込みがあり、手当てが違う。

1. **順序が逆**。正例より負例のほうが良く見えている。表現力かデータの問題
2. **マージン不足**。順序は正しいが差がδに届かない。δを動かす話になる
3. **教師の誤り**。depth 9の最善手が実際には最善でない。データの上限

このツールが分けるのは1と2で、3は分けられない（正解を知らないため）。
それでも「順序が逆の群がどれだけ残っているか」が分かれば、αを上げる線と
δを動かす線のどちらに意味があるかを決められる。

終了コード: 0=成功、2=引数エラー、3=実行時エラー（ADR-0122）。
"""

import argparse
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

# ADR-0185の宣言値。学習側の --rank-margin と揃える
DEFAULT_MARGIN = 0.02


def error(message):
    """エラーメッセージを規約の書式でstderrへ出す。"""
    print(f"エラー: {message}", file=sys.stderr)


class ArgParser(argparse.ArgumentParser):
    """引数エラーを「エラー: ...」の書式・終了コード2に揃える。"""

    def error(self, message):
        error(message)
        sys.exit(2)


def build_parser():
    parser = ArgParser(
        prog="rank-diag.py",
        description="ランキング損失のヒンジ発火を、順序の逆転とマージン不足へ分ける。",
        epilog="学習側の --rank-margin と同じ値を渡す。",
    )
    parser.add_argument(
        "weights",
        help="学習チェックポイント（*.ckpt）か、書き出し後のネット（*.hmwr）",
    )
    parser.add_argument("rank_data", help="psv rank が書いた群（*.rankpsv）")
    parser.add_argument(
        "--margin", type=float, default=DEFAULT_MARGIN, help="ヒンジのマージン"
    )
    parser.add_argument("--groups", type=int, default=200_000, help="測る群の数")
    parser.add_argument("--batch", type=int, default=4096, help="1回に引く群の数")
    parser.add_argument("--seed", type=int, default=0, help="群を引く乱数の種")
    parser.add_argument("--threads", type=int, default=2, help="torchのスレッド数")
    return parser


def measure(args):
    """群ごとに正例と負例の差を集め、分類する。"""
    import torch

    sys.path.insert(0, str(REPO / "training"))
    from dataset import RankLoader
    from model import NnueModel

    torch.set_num_threads(args.threads)
    model = NnueModel(sparse_ft=False, factorized=True)
    if args.weights.endswith(".hmwr"):
        from quantize import load_into

        label = load_into(model, args.weights)
    else:
        state = torch.load(args.weights, map_location="cpu", weights_only=False)
        model.load_state_dict(state["model"])
        label = f"step={state.get('step', '?')}"
    model.eval()

    loader = RankLoader(args.rank_data, args.batch, seed=args.seed)
    # 負例2本ぶんの差を1本ずつ数える。群ではなく比較の単位で見る
    gaps = []
    with torch.no_grad():
        while sum(g.numel() for g in gaps) < args.groups * 2:
            stm_i, stm_o, opp_i, opp_o, parity = loader.sample()
            u = model(stm_i, stm_o, opp_i, opp_o)
            sign = 1.0 - 2.0 * parity
            upv = torch.sigmoid(u * sign).view(-1, 3)
            # 正例 − 負例。正なら順序が付いている
            gaps.append((upv[:, 0:1] - upv[:, 1:3]).flatten())
    return torch.cat(gaps)[: args.groups * 2], label, loader.n


def report(gap, label, total, margin):
    """発火の内訳と差の分布を出す。"""
    import torch

    n = gap.numel()
    inverted = (gap <= 0).sum().item()
    thin = ((gap > 0) & (gap < margin)).sum().item()
    ok = n - inverted - thin
    print(f"{n:,}比較（群{n // 2:,}、ファイルは{total:,}群）、{label}")
    print(f"マージン δ={margin}")
    print()
    print(f"  順序が逆      : {inverted:>9,} ({100 * inverted / n:5.1f}%)")
    print(f"  マージン不足  : {thin:>9,} ({100 * thin / n:5.1f}%)")
    print(f"  余裕あり      : {ok:>9,} ({100 * ok / n:5.1f}%)")
    print(f"  ヒンジ発火    : {inverted + thin:>9,} ({100 * (inverted + thin) / n:5.1f}%)")
    print()
    q = torch.tensor([0.01, 0.1, 0.25, 0.5, 0.75, 0.9, 0.99])
    v = torch.quantile(gap.float(), q)
    print("正例 − 負例の分位点")
    print("  " + "  ".join(f"p{int(p * 100)}={x:+.4f}" for p, x in zip(q, v)))
    if inverted:
        deep = gap[gap <= 0]
        print(f"逆転した比較の深さ: 平均{deep.mean():+.4f} 最小{deep.min():+.4f}")


def main(argv=None):
    """argvを省くとsys.argvを読む。hmwr net rankは引数リストで呼ぶ。"""
    args = build_parser().parse_args(argv)
    if args.groups <= 0 or args.batch <= 0:
        error("群の数とバッチは正整数で指定する")
        return 2
    try:
        gap, label, total = measure(args)
        report(gap, label, total, args.margin)
    except (OSError, ValueError, KeyError) as e:
        error(e)
        return 3
    return 0


if __name__ == "__main__":
    sys.exit(main())
