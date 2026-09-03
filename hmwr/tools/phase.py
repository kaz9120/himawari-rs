#!/usr/bin/env python3
"""進行度の指標の候補を、評価の系統誤差で比べる（ADR-0198）。

`psv phase` が書いたTSV（局面ごとの教師信号・静的評価・指標）を読み、
指標ごとに局面を4クラスへ切って、クラス別のアフィン補正がBCE損失を
どれだけ下げるかを測る。

    ΔL(指標) = L(クラス別アフィン) − L(全体アフィン)

ΔLが負に大きい指標ほど、その分類が評価の偏りと尺度の違いを説明している。
ノイズの床は乱数で4クラスへ分けた分割の最良値で置く。読み方の事前登録は
[ADR-0198](../../docs/adr/0198-phase-indicator.md)にある。

`--weights` と `--psv` を渡すと、学習器のモデルで同じ局面のL2活性を取り、
クラスごとに線形ヘッド（33パラメータ）を当てた損失の差ΔL2も出す。
ADR-0137が提案した「最終段だけの分岐」と同じ容量になる。

終了コード: 0=成功、2=引数エラー、3=実行時エラー（ADR-0122）。
"""

import argparse
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

# 学習と同じ勝率変換（crates/py の SIGMOID_SCALE）と目標の混合比
SIGMOID_SCALE = 600.0
DEFAULT_LAMBDA = 0.7
CLASSES = 4
RANDOM_SPLITS = 20
# 評価値の残差は詰みスコアが支配するので、この絶対値以下に絞る
CP_LIMIT = 3000


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
        prog="phase.py",
        description="進行度の指標ごとに、クラス別のアフィン補正が損失を下げる量を比べる。",
    )
    parser.add_argument("tsv", help="psv phase が書いたTSV")
    parser.add_argument(
        "--lambda",
        type=float,
        default=DEFAULT_LAMBDA,
        dest="lambda_",
        help="目標の混合比。1で教師のscoreだけ、0で勝敗だけ",
    )
    parser.add_argument("--seed", type=int, default=0, help="乱数分割の種")
    parser.add_argument(
        "--weights", help="L2活性を取るネット（*.hmwr か *.ckpt）。--psv と組む"
    )
    parser.add_argument("--psv", help="TSVを書いたときと同じpsv")
    parser.add_argument("--batch", type=int, default=8192, help="L2活性を取るバッチ")
    parser.add_argument("--threads", type=int, default=2, help="torchのスレッド数")
    return parser


def sigmoid(x):
    import numpy as np

    return 1.0 / (1.0 + np.exp(-x))


def bce(p, t):
    import numpy as np

    eps = 1e-7
    p = np.clip(p, eps, 1.0 - eps)
    return float(-np.mean(t * np.log(p) + (1.0 - t) * np.log(1.0 - p)))


def fit_affine(z, t, iters=30):
    """ロジットのアフィン変換 a·z+b をNewton法でBCEに当てはめ、損失を返す。"""
    import numpy as np

    a, b = 1.0, 0.0
    for _ in range(iters):
        p = sigmoid(a * z + b)
        g = p - t
        w = p * (1.0 - p)
        grad = np.array([np.mean(g * z), np.mean(g)])
        hess = np.array(
            [[np.mean(w * z * z), np.mean(w * z)], [np.mean(w * z), np.mean(w)]]
        )
        try:
            step = np.linalg.solve(hess, grad)
        except np.linalg.LinAlgError:
            break
        a -= step[0]
        b -= step[1]
        if np.max(np.abs(step)) < 1e-9:
            break
    return bce(sigmoid(a * z + b), t), a, b


def fit_linear(h, t, w0=None, iters=50):
    """線形ヘッド w·h+b をNewton法でBCEに当てはめ、損失を返す。

    hは局面×次元。パラメータは次元+1で、ADR-0137の出力段の分岐と同じ容量。
    w0は初期値（ネットの出力層をそのまま渡す）。損失が増える更新は
    刻みを半分にして受け入れる（減衰Newton）。
    """
    import numpy as np

    x = np.hstack([h, np.ones((len(h), 1))])
    w = np.zeros(x.shape[1]) if w0 is None else np.array(w0, dtype=np.float64)
    ridge = 1e-4 * np.eye(x.shape[1])
    # torchを読んだ後のmacOSでは、有限の入力でもmatmulが偽のFP例外を
    # 立てる（Accelerateの残留フラグ）。結果は正しいので警告だけ抑える
    with np.errstate(all="ignore"):
        return _fit_linear(x, w, t, ridge, iters)


def _fit_linear(x, w, t, ridge, iters):
    import numpy as np

    loss = bce(sigmoid(x @ w), t)
    for _ in range(iters):
        p = sigmoid(x @ w)
        grad = x.T @ (p - t) / len(x)
        hess = (x * (p * (1.0 - p))[:, None]).T @ x / len(x) + ridge
        step = np.linalg.solve(hess, grad)
        scale = 1.0
        while scale > 1e-4:
            cand = w - scale * step
            cand_loss = bce(sigmoid(x @ cand), t)
            if cand_loss <= loss:
                break
            scale *= 0.5
        else:
            break
        improved = loss - cand_loss
        w, loss = cand, cand_loss
        if improved < 1e-12:
            break
    return loss


def classwise_loss(z, t, cls, h=None, w0=None):
    """クラスごとに別の補正を当て、全体の損失（局面数で重み付け）を返す。

    hを渡すとL2活性の線形ヘッド、渡さなければ出力のアフィンを当てる。
    """
    total = 0.0
    for k in range(int(cls.max()) + 1):
        m = cls == k
        if not m.any():
            continue
        if h is None:
            loss, _, _ = fit_affine(z[m], t[m])
        else:
            loss = fit_linear(h[m], t[m], w0)
        total += loss * m.sum()
    return total / len(z)


def hidden_activations(weights, psv, batch, threads):
    """学習器のモデルで、psvの局面順にL2活性（出力直前の隠れ層）を返す。"""
    import numpy as np
    import torch

    sys.path.insert(0, str(REPO / "training"))
    from dataset import PsvBatchLoader
    from model import NnueModel

    torch.set_num_threads(threads)
    model = NnueModel(sparse_ft=False, factorized=True)
    if weights.endswith(".hmwr"):
        from quantize import load_into

        load_into(model, weights)
    else:
        state = torch.load(weights, map_location="cpu", weights_only=False)
        model.load_state_dict(state["model"])
    model.eval()

    loader = PsvBatchLoader(psv, batch=batch, shuffle=False)
    hs, outs = [], []
    with torch.no_grad():
        for b in loader:
            if b is None:
                continue
            x = model.transform_both(*b[:4])
            h = model.l2(x).clamp(0.0, 1.0)
            for layer in (model.l3, model.l4):
                if layer is not None:
                    h = layer(h).clamp(0.0, 1.0)
            hs.append(h.numpy().astype(np.float64))
            outs.append(model.out(h).squeeze(1).numpy().astype(np.float64))
    w0 = np.concatenate(
        [model.out.weight.detach().numpy().reshape(-1), model.out.bias.detach().numpy()]
    ).astype(np.float64)
    return np.vstack(hs), np.concatenate(outs), w0


def quartile_classes(v):
    """値を四分位で4クラスへ切る。離散値は四分位にいちばん近い境界で切る。"""
    import numpy as np

    values, counts = np.unique(v, return_counts=True)
    cum = np.cumsum(counts) / len(v)
    bounds = []
    for k in range(1, CLASSES):
        i = int(np.argmin(np.abs(cum - k / CLASSES)))
        # 境界より大きい値が次のクラスになる
        bound = values[i]
        if not bounds or bound > bounds[-1]:
            bounds.append(bound)
    cls = np.zeros(len(v), dtype=np.int64)
    for bound in bounds:
        cls += v > bound
    return cls, bounds


def load(path):
    import numpy as np

    with open(path, encoding="utf-8") as f:
        header = f.readline().rstrip("\n").split("\t")
    data = np.loadtxt(path, delimiter="\t", skiprows=1, dtype=np.float64)
    if data.ndim != 2 or data.shape[1] != len(header):
        raise ValueError(f"TSVの形が合わない: {path}")
    return header, data


def measure(args):
    import numpy as np

    header, data = load(args.tsv)
    col = {name: i for i, name in enumerate(header)}
    for need in ("score", "result", "eval"):
        if need not in col:
            raise KeyError(f"列がない: {need}")
    score = data[:, col["score"]]
    result = data[:, col["result"]]
    z = data[:, col["eval"]] / SIGMOID_SCALE
    p_eval = sigmoid(z)
    t = args.lambda_ * sigmoid(score / SIGMOID_SCALE) + (1.0 - args.lambda_) * (
        result + 1.0
    ) / 2.0
    n = len(z)

    base = bce(p_eval, t)
    global_loss, ga, gb = fit_affine(z, t)

    rng = np.random.default_rng(args.seed)
    floor = []
    for _ in range(RANDOM_SPLITS):
        cls = rng.permutation(n) % CLASSES
        floor.append(classwise_loss(z, t, cls) - global_loss)
    floor_best = min(floor)
    floor_mean = float(np.mean(floor))

    # L2活性の線形ヘッド（ADR-0137の機構と同じ容量）
    hidden = None
    if args.weights:
        h, logit, w0 = hidden_activations(
            args.weights, args.psv, args.batch, args.threads
        )
        # TSVはpsvの先頭から書かれる（--limit で短いことがある）
        if len(h) < n:
            raise ValueError(f"局面数が合わない: TSV {n} / モデル {len(h)}")
        h, logit = h[:n], logit[:n]
        # TSVの静的評価と並びが揃っているかを、量子化前後の相関で確かめる
        corr = float(np.corrcoef(logit, z)[0, 1])
        if corr < 0.99:
            raise ValueError(f"TSVとモデルの評価が揃っていない（相関{corr:.3f}）")
        h_global = fit_linear(h, t, w0)
        h_floor = []
        for _ in range(RANDOM_SPLITS):
            cls = rng.permutation(n) % CLASSES
            h_floor.append(classwise_loss(z, t, cls, h, w0) - h_global)
        hidden = {
            "dims": h.shape[1],
            "corr": corr,
            "global": h_global,
            "floor_best": min(h_floor),
            "floor_mean": float(np.mean(h_floor)),
            "h": h,
            "w0": w0,
        }

    indicators = [name for name in header if name not in ("score", "result", "eval")]
    rows = []
    for name in indicators:
        v = data[:, col[name]]
        cls, bounds = quartile_classes(v)
        delta = classwise_loss(z, t, cls) - global_loss
        delta2 = (
            classwise_loss(z, t, cls, hidden["h"], hidden["w0"]) - hidden["global"]
            if hidden
            else None
        )
        detail = []
        for k in range(len(bounds) + 1):
            m = cls == k
            if not m.any():
                continue
            cp = m & (np.abs(score) <= CP_LIMIT)
            detail.append(
                {
                    "n": int(m.sum()),
                    "lo": float(v[m].min()),
                    "hi": float(v[m].max()),
                    "bias_wp": float(np.mean(p_eval[m] - t[m])),
                    "rmse_wp": float(np.sqrt(np.mean((p_eval[m] - t[m]) ** 2))),
                    "bias_cp": float(np.mean(data[cp, col["eval"]] - score[cp]))
                    if cp.any()
                    else float("nan"),
                    "loss": bce(p_eval[m], t[m]),
                }
            )
        rows.append(
            {"name": name, "delta": delta, "delta2": delta2, "classes": detail}
        )
    rows.sort(key=lambda r: r["delta"])
    return {
        "n": n,
        "lambda": args.lambda_,
        "base": base,
        "global": global_loss,
        "global_ab": (ga, gb),
        "floor_best": floor_best,
        "floor_mean": floor_mean,
        "hidden": hidden,
        "rows": rows,
    }


def report(r):
    print(f"局面数: {r['n']}　目標の混合比λ: {r['lambda']}")
    print(f"補正なしのBCE: {r['base']:.5f}")
    ga, gb = r["global_ab"]
    print(f"全体アフィン後: {r['global']:.5f}（a={ga:.3f} b={gb:+.3f}）")
    print(
        f"乱数分割{RANDOM_SPLITS}回のΔL: 最良{r['floor_best'] * 1e4:+.3f}"
        f" 平均{r['floor_mean'] * 1e4:+.3f}（×1e-4）"
    )
    hidden = r["hidden"]
    if hidden:
        print(
            f"L2活性{hidden['dims']}次元の線形ヘッド: 全体で当て直すと{hidden['global']:.5f}"
            f"（TSVの評価との相関{hidden['corr']:.4f}）"
        )
        print(
            f"乱数分割{RANDOM_SPLITS}回のΔL2: 最良{hidden['floor_best'] * 1e4:+.3f}"
            f" 平均{hidden['floor_mean'] * 1e4:+.3f}（×1e-4）"
        )
    print()
    print("指標ごとのΔL（クラス別 − 全体、×1e-4）。負に大きいほど系統誤差を説明する")
    print("ΔLは出力のアフィン2パラメータ、ΔL2はL2活性の線形ヘッド（ADR-0137の容量）")
    head = f"{'指標':<10} {'ΔL':>8} {'床比':>6}"
    if hidden:
        head += f" {'ΔL2':>8} {'床比':>6}"
    print(f"{head}  クラス境界（局面数）")
    for row in r["rows"]:
        ratio = row["delta"] / r["floor_best"] if r["floor_best"] < 0 else float("nan")
        line = f"{row['name']:<10} {row['delta'] * 1e4:+8.3f} {ratio:6.1f}"
        if hidden:
            ratio2 = (
                row["delta2"] / hidden["floor_best"]
                if hidden["floor_best"] < 0
                else float("nan")
            )
            line += f" {row['delta2'] * 1e4:+8.3f} {ratio2:6.1f}"
        bounds = "  ".join(
            f"{c['lo']:.0f}〜{c['hi']:.0f}({c['n']})" for c in row["classes"]
        )
        print(f"{line}  {bounds}")
    print()
    print("クラス別の残差（ネット − 目標。偏りは勝率と評価値、RMSEは勝率）")
    for row in r["rows"]:
        print(f"[{row['name']}]")
        for c in row["classes"]:
            print(
                f"  {c['lo']:>4.0f}〜{c['hi']:<4.0f} n={c['n']:>7}"
                f"  偏り {c['bias_wp'] * 100:+6.2f}%  {c['bias_cp']:+7.1f}cp"
                f"  RMSE {c['rmse_wp'] * 100:5.2f}%  BCE {c['loss']:.5f}"
            )


def main(argv=None):
    """argvを省くとsys.argvを読む。hmwr net phaseは引数リストで呼ぶ。"""
    args = build_parser().parse_args(argv)
    if not 0.0 <= args.lambda_ <= 1.0:
        error(f"--lambda は0〜1で指定する: {args.lambda_}")
        return 2
    if bool(args.weights) != bool(args.psv):
        error("--weights と --psv は組で渡す")
        return 2
    try:
        report(measure(args))
    except (OSError, ValueError, KeyError) as e:
        error(e)
        return 3
    return 0


if __name__ == "__main__":
    sys.exit(main())
