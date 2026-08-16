#!/usr/bin/env python3
"""活性ダンプからFT出力次元の置換を決める（ADR-0168）。

第1層は4列チャンクのうち全ゼロのものを飛ばす（ADR-0151群L）。飛ばせる数は
次元の並び順で変わるので、ゼロが同時に起きる次元を同じチャンクへ寄せると
第1層の回る回数が減る。出力した置換は `makenet --reorder` へ渡す。
並べ替えても積和の項の集合は変わらず、評価値はビット一致する。

入力は `--features himawari-engine/actdump` を付けたビルドが書くダンプで、
1サンプル `CONCAT / 8` バイトのビットマスクが並ぶ。ビットiは連結ベクトルの
次元iが非ゼロだったことを表す。

**渡すのは片視点の活性の次元数であって、FTの出力次元ではない。**出力の対を
掛ける構成（ADR-0171）では活性が `FT_OUT / 2` になる。置換もその単位で出て、
`makenet --reorder` がFT側を対で動かす。

終了コード: 0=成功、2=引数エラー、3=実行時エラー（ADR-0122）。
"""

import argparse
import sys

# バイト値から、立っているビットの位置を引く表。
NONZERO_BITS = [[i for i in range(8) if v >> i & 1] for v in range(256)]


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
        prog="ft-reorder.py",
        description="活性ダンプからFT出力次元の置換を決める（ADR-0168）。",
        epilog="ダンプは HIMAWARI_ACT_OUT で場所を指定し、actdump付きのビルドが書く。",
    )
    parser.add_argument("dump", help="活性ダンプ（*.bin）")
    parser.add_argument(
        "activations",
        type=int,
        help="片視点の活性次元（積ありなら FT_OUT/2。例 512）",
    )
    parser.add_argument("--out", help="置換の出力先。省略すると書かない")
    parser.add_argument(
        "--perm",
        help="既存の置換を当てて評価するだけにする（別局面での検証に使う）",
    )
    return parser


def load_masks(path, ft_out):
    """次元ごとのゼロマスクを作る。`ft_out` は片視点の活性次元。

    戻り値は `(視点0のマスク, 視点1のマスク, サンプル数)`。マスクの
    ビットnは「サンプルnでその次元がゼロだった」ことを表す。
    """
    concat = ft_out * 2
    per_sample = concat // 8
    with open(path, "rb") as f:
        raw = f.read()
    if len(raw) < per_sample:
        raise ValueError(
            f"ダンプが{len(raw)}バイトしかない（1サンプル{per_sample}バイト）"
        )
    n = len(raw) // per_sample
    columns = [bytearray((n + 7) // 8) for _ in range(concat)]
    for s in range(n):
        base = s * per_sample
        index, bit = s >> 3, 1 << (s & 7)
        for b in range(per_sample):
            v = raw[base + b]
            if v:
                head = b * 8
                for i in NONZERO_BITS[v]:
                    columns[head + i][index] |= bit
    full = (1 << n) - 1
    zero = [full ^ int.from_bytes(bytes(c), "little") for c in columns]
    return zero[:ft_out], zero[ft_out:], n


def chunk_stats(z0, z1, perm, n):
    """並び `perm` での全ゼロチャンク率と、1サンプルあたり回るチャンク数。"""
    chunks = len(perm) // 4
    allzero = 0
    for c in range(chunks):
        a, b, d, e = perm[4 * c : 4 * c + 4]
        allzero += (z0[a] & z0[b] & z0[d] & z0[e]).bit_count()
        allzero += (z1[a] & z1[b] & z1[d] & z1[e]).bit_count()
    total = n * chunks * 2
    rate = allzero / total
    return rate, chunks * 2 * (1 - rate)


def greedy_permutation(z0, z1):
    """ゼロが同時に起きる次元を4個ずつ寄せる。

    ゼロの多い次元を種にして、両視点あわせて同時ゼロが最大になる相手を
    3つ選ぶ。最適解は求めない（4個組への分割は組合せ爆発する）が、
    独立仮定の並びより寄っていれば足りる。
    """
    ft_out = len(z0)
    used = [False] * ft_out
    order = sorted(
        range(ft_out), key=lambda d: -(z0[d].bit_count() + z1[d].bit_count())
    )
    perm = []
    for seed in order:
        if used[seed]:
            continue
        used[seed] = True
        cur0, cur1 = z0[seed], z1[seed]
        perm.append(seed)
        for _ in range(3):
            best, best_score, best0, best1 = -1, -1, 0, 0
            for d in range(ft_out):
                if used[d]:
                    continue
                a0 = cur0 & z0[d]
                a1 = cur1 & z1[d]
                score = a0.bit_count() + a1.bit_count()
                if score > best_score:
                    best, best_score, best0, best1 = d, score, a0, a1
            if best < 0:
                raise ValueError("次元が4で割り切れない")
            used[best] = True
            perm.append(best)
            cur0, cur1 = best0, best1
    return perm


def read_perm(path, ft_out):
    """置換を読み、`0..ft_out` の順列であることを確かめる。"""
    with open(path) as f:
        perm = [int(t) for t in f.read().split()]
    if sorted(perm) != list(range(ft_out)):
        raise ValueError(f"{path}が0..{ft_out}の順列になっていない")
    return perm


def report(z0, z1, n, ft_out, perm):
    """並べ替え前後を並べて出す。"""
    zeros = sum(z.bit_count() for z in z0) + sum(z.bit_count() for z in z1)
    elem = zeros / (n * ft_out * 2)
    base = list(range(ft_out))
    rate0, running0 = chunk_stats(z0, z1, base, n)
    rate1, running1 = chunk_stats(z0, z1, perm, n)
    chunks = ft_out // 2
    print(f"{n}サンプル、片視点の活性={ft_out}、CONCAT={ft_out * 2}")
    print(f"要素のゼロ率: {elem:.4f}（独立仮定の全ゼロチャンク率 {elem**4:.4f}）")
    print(f"そのまま:   全ゼロチャンク率 {rate0:.4f}、回るチャンク {running0:.1f}/{chunks}")
    print(f"並べ替え後: 全ゼロチャンク率 {rate1:.4f}、回るチャンク {running1:.1f}/{chunks}")
    print(f"第1層の削減: {(1 - running1 / running0) * 100:.1f}%")
    for t in (1.0, 0.99, 0.95):
        dead = sum(
            1
            for d in range(ft_out)
            if (z0[d].bit_count() + z1[d].bit_count()) / (2 * n) >= t
        )
        print(f"  ゼロ率{t:.2f}以上の次元: {dead}/{ft_out}")


def run(args):
    z0, z1, n = load_masks(args.dump, args.activations)
    if args.perm:
        perm = read_perm(args.perm, args.activations)
    else:
        perm = greedy_permutation(z0, z1)
    report(z0, z1, n, args.activations, perm)
    if args.out:
        with open(args.out, "w") as f:
            f.write("\n".join(str(p) for p in perm) + "\n")
        print(f"置換を書いた: {args.out}")


def main():
    args = build_parser().parse_args()
    if args.activations <= 0 or args.activations % 4 != 0:
        error(f"活性の次元は4の倍数の正整数で指定する: {args.activations}")
        return 2
    try:
        run(args)
    except (OSError, ValueError) as e:
        error(e)
        return 3
    return 0


if __name__ == "__main__":
    sys.exit(main())
