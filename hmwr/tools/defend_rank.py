#!/usr/bin/env python3
"""攻撃的な受けの局面で、最善手の順位を一般の局面と比べる（ADR-0213）。

`psv defend` が書いたTSVを読み、局面を2つの群へ分けて最善手の順位を
集計する。群は「攻撃的な受け」の印で分ける。印は、教師の最善手の移動先から
利く相手の駒のうち、自玉の周囲8マスへ利いているものが1枚でもあるかで付く。

順位は、全合法手の子をqsearchの葉まで進めて親視点の評価値で並べたときの
教師手の位置である。同値は同順位とし、順位は「厳密に良い手の数 + 1」になる。

読み方はADR-0213の事前登録にある。ここは数字を出すだけで、判定はしない。

終了コード: 0=成功、2=引数エラー、3=実行時エラー（ADR-0122）。
"""

import argparse
import sys
import unicodedata

COLUMNS = ("index", "defend", "moves", "rank", "value", "best")
# 上位何位までを「見えている」側として数えるか
TOP_K = 3
GROUPS = (("攻撃的な受け", 1), ("一般", 0))


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
        prog="defend-rank.py",
        description="攻撃的な受けの局面と一般の局面で、最善手の順位を比べる（ADR-0213）。",
    )
    parser.add_argument("tsv", help="psv defend が書いたTSV")
    parser.add_argument("--log", help="表を書き出す先。省くと標準出力だけに出す")
    return parser


def read_rows(path):
    """TSVを読み、列を整数の辞書にして返す。"""
    rows = []
    with open(path, encoding="utf-8") as fh:
        header = fh.readline().rstrip("\n").split("\t")
        if tuple(header) != COLUMNS:
            raise ValueError(f"列が違う: {header}（期待 {list(COLUMNS)}）")
        for line in fh:
            line = line.rstrip("\n")
            if not line:
                continue
            values = line.split("\t")
            if len(values) != len(COLUMNS):
                raise ValueError(f"列数が合わない行がある: {line}")
            rows.append(dict(zip(COLUMNS, (int(v) for v in values))))
    return rows


def median(values):
    """中央値。偶数個なら中央2つの平均を返す。"""
    if not values:
        return 0.0
    s = sorted(values)
    half = len(s) // 2
    if len(s) % 2:
        return float(s[half])
    return (s[half - 1] + s[half]) / 2


def summarize(rows):
    """1つの群の統計を出す。件数が0なら None を返す。"""
    if not rows:
        return None
    ranks = [r["rank"] for r in rows]
    n = len(ranks)
    mean = sum(ranks) / n
    # 平均の標準誤差。群の差が事前登録のしきい値の近くに出たとき、
    # 標本の揺れで足りているかを読み手が自分で判断できるようにする
    var = sum((x - mean) ** 2 for x in ranks) / (n - 1) if n > 1 else 0.0
    return {
        "n": n,
        "mean": mean,
        "se": (var / n) ** 0.5,
        "median": median(ranks),
        "top1": 100.0 * sum(1 for r in ranks if r == 1) / n,
        "topk": 100.0 * sum(1 for r in ranks if r <= TOP_K) / n,
        "moves": sum(r["moves"] for r in rows) / n,
        "loss": sum(r["best"] - r["value"] for r in rows) / n,
    }


def aggregate(rows):
    """群ごとの統計を名前つきで返す。"""
    return {name: summarize([r for r in rows if r["defend"] == flag]) for name, flag in GROUPS}


def _width(text):
    """端末での表示幅。全角を2桁と数える。"""
    return sum(2 if unicodedata.east_asian_width(c) in "WF" else 1 for c in text)


def _cell(text, width, left=False):
    """表示幅で揃えた桁を作る。"""
    pad = " " * max(width - _width(text), 1)
    return text + pad if left else pad + text


def _row(name, cells, widths):
    return _cell(name, widths[0], left=True) + "".join(
        _cell(text, w) for text, w in zip(cells, widths[1:])
    )


def report(stats, rows, path):
    """表を行のリストで返す。標準出力とログへ同じものを出す。"""
    lines = []
    lines.append("=== 攻撃的な受けの局面での最善手の順位（ADR-0213） ===")
    lines.append(f"入力  : {path}")
    lines.append(f"局面数: {len(rows):,}")
    matched = stats["攻撃的な受け"]
    share = 100.0 * matched["n"] / len(rows) if rows and matched else 0.0
    lines.append(f"該当率: {share:.2f}%")
    lines.append("")
    lines.append("順位は、全合法手の子を葉まで進めて並べたときの教師手の位置である。")
    lines.append("同値は同順位とし、1位が最良になる。")
    lines.append("")
    widths = (14, 10, 10, 10, 8, 10, 10, 10)
    header = ("件数", "平均順位", "中央値", "1位率%", f"{TOP_K}位以内%", "合法手数", "平均損cp")
    lines.append(_row("群", header, widths))
    for name, _ in GROUPS:
        s = stats[name]
        if s is None:
            lines.append(_row(name, ("（該当なし）",), widths))
            continue
        cells = (
            f"{s['n']:,}",
            f"{s['mean']:.2f}",
            f"{s['median']:.1f}",
            f"{s['top1']:.1f}",
            f"{s['topk']:.1f}",
            f"{s['moves']:.1f}",
            f"{s['loss']:.1f}",
        )
        lines.append(_row(name, cells, widths))
    both = [stats[name] for name, _ in GROUPS]
    if all(both):
        # 平均順位の桁へ差を置く。件数の桁は空けたままにする
        gap = both[0]["mean"] - both[1]["mean"]
        se = (both[0]["se"] ** 2 + both[1]["se"] ** 2) ** 0.5
        lines.append(_row("差", ("", f"{gap:+.2f}"), widths))
        lines.append("")
        lines.append(f"平均順位の差は {gap:+.2f} で、標準誤差は ±{se:.2f} である。")
    lines.append("")
    lines.append("平均損は、最良の葉の値と教師手の葉の値の差である。")
    return lines


def main(argv=None):
    """argvを省くとsys.argvを読む。hmwr diag defendは引数リストで呼ぶ。"""
    args = build_parser().parse_args(argv)
    try:
        rows = read_rows(args.tsv)
        if not rows:
            error(f"局面がない: {args.tsv}")
            return 3
        lines = report(aggregate(rows), rows, args.tsv)
    except (OSError, ValueError) as e:
        error(e)
        return 3
    text = "\n".join(lines)
    print(text)
    if args.log:
        with open(args.log, "a", encoding="utf-8") as fh:
            fh.write(text + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
