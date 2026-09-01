#!/usr/bin/env python3
"""FT出力の対が死ぬ原因を、a側とb側に分けて測る（ADR-0194）。

対の積（ADR-0171）は片側がゼロなら結果もゼロになる。死んだ次元を回収する
手を選ぶ前に、積が死ぬ原因が「片側の死」なのか「両側が同時に発火しない」
のかを切り分ける。前者なら片側だけを見ればよく、後者なら対の組み方の
問題になる。

活性は学習器と同じf32で測る。エンジンの活性ダンプ（`hmwr net reorder` が
読む）は量子化を通った後の値なので、同じ次元でもゼロ率が変わる。両方を
突き合わせるとき、量子化・局面分布・サンプル数の3つが同時に動くことに
注意する（ADR-0194の測定を見よ）。

終了コード: 0=成功、2=引数エラー、3=実行時エラー（ADR-0122）。
"""

import argparse
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

# 死んだとみなすゼロ率。ADR-0168からの慣用で0.95を使う
DEAD_RATE = 0.95


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
        prog="dead-dims.py",
        description="FT出力の対が死ぬ原因をa側とb側に分けて測る（ADR-0194）。",
        epilog="検証集合は学習データと同じ土俵へ揃える（ADR-0136）。",
    )
    parser.add_argument(
        "weights",
        help="学習チェックポイント（*.ckpt）か、書き出し後のネット（*.hmwr）。"
        "拡張子で見分ける。同じ局面で両方を測ると量子化の効果だけが分かれる",
    )
    parser.add_argument("valid", help="測る局面のpsv")
    parser.add_argument("--batch", type=int, default=8192, help="バッチの大きさ")
    parser.add_argument("--threads", type=int, default=2, help="torchのスレッド数")
    return parser


def measure(args):
    """視点ごとにa側・b側・積の非ゼロ回数を数える。"""
    import torch

    sys.path.insert(0, str(REPO / "training"))
    from dataset import PsvBatchLoader
    from model import HALF, NnueModel

    torch.set_num_threads(args.threads)
    model = NnueModel(sparse_ft=False, factorized=True)
    if args.weights.endswith(".hmwr"):
        from quantize import load_into

        # 量子化の逆変換なので、元のf32とは丸めのぶんだけ違う
        label = load_into(model, args.weights)
    else:
        state = torch.load(args.weights, map_location="cpu", weights_only=False)
        model.load_state_dict(state["model"])
        label = f"step={state.get('step', '?')}"
    model.eval()

    loader = PsvBatchLoader(args.valid, batch=args.batch, shuffle=False)
    keys = ("a", "b", "prod", "qa", "qb", "qprod")
    nz = {k: torch.zeros(HALF) for k in keys}
    n = 0
    with torch.no_grad():
        for batch in loader:
            if batch is None:
                continue
            stm_i, stm_o, opp_i, opp_o = batch[:4]
            # stmとoppは同じ重みを通る。視点ぶんを合算して数える
            for idx, off in ((stm_i, stm_o), (opp_i, opp_o)):
                z = model.transform(idx, off)
                a = z[:, :HALF].clamp(0.0, 1.0)
                b = z[:, HALF:].clamp(0.0, 1.0)
                # エンジンはaccumulatorを0..127のu8へ丸め、活性を
                # (clip(a)*clip(b)+64)>>7 で作る（nnue.rsのpair_activation）。
                # f32のまま数えると、丸めで消える微小な積を非ゼロと数える
                ai = (a * 127).round().clamp(0, 127)
                bi = (b * 127).round().clamp(0, 127)
                qi = torch.div(ai * bi + 64, 128, rounding_mode="floor")
                nz["a"] += (a > 0).sum(0).float()
                nz["b"] += (b > 0).sum(0).float()
                nz["prod"] += ((a * b) > 0).sum(0).float()
                nz["qa"] += (ai > 0).sum(0).float()
                nz["qb"] += (bi > 0).sum(0).float()
                nz["qprod"] += (qi > 0).sum(0).float()
                n += a.shape[0]
    if n == 0:
        raise ValueError(f"測る局面がない: {args.valid}")
    return {k: 1 - v / n for k, v in nz.items()}, n, HALF, label


def report(zero, n, half, label):
    """ゼロ率の要約と、積が死ぬ原因の内訳を出す。"""
    print(f"{n}サンプル（2視点ぶん）、対の数={half}、{label}")
    for title, suffix in (("f32（学習器と同じ）", ""), ("量子化後（エンジンと同じ）", "q")):
        print(title)
        for name, key in (("a側", "a"), ("b側", "b"), ("積 ", "prod")):
            z = zero[suffix + key]
            print(
                f"  {name}: 全ゼロ {int((z >= 1.0).sum())} / "
                f"ゼロ率{DEAD_RATE}以上 {int((z >= DEAD_RATE).sum())} / "
                f"平均ゼロ率 {z.mean():.4f}"
            )
        dead_a = zero[suffix + "a"] >= DEAD_RATE
        dead_b = zero[suffix + "b"] >= DEAD_RATE
        dead_p = zero[suffix + "prod"] >= DEAD_RATE
        print(
            f"  死んだ積の内訳: a側だけ {int((dead_a & ~dead_b).sum())} / "
            f"b側だけ {int((dead_b & ~dead_a).sum())} / "
            f"両側 {int((dead_a & dead_b).sum())} / "
            f"両側とも生存 {int((dead_p & ~dead_a & ~dead_b).sum())}"
        )
    print()
    print("「両側とも生存」は、片側ずつは発火するのに積が立たない対を表す。")
    print("量子化後の側は、積が64未満だと丸めで0になることを含む。")


def main(argv=None):
    """argvを省くとsys.argvを読む。hmwr net deadは引数リストで呼ぶ。"""
    args = build_parser().parse_args(argv)
    if args.batch <= 0:
        error(f"バッチは正整数で指定する: {args.batch}")
        return 2
    try:
        report(*measure(args))
    except (OSError, ValueError, KeyError) as e:
        error(e)
        return 3
    return 0


if __name__ == "__main__":
    sys.exit(main())
