#!/usr/bin/env python3
"""影の利き属性の、1手あたりの反転数を測る（ADR-0213）。

駒トークンに「そのマスへの利きの数」を属性として足す案のコストは、
1手で属性が変わったトークンの数で決まる。飛び駒（香・角・飛と馬龍の
飛び部分）の利きは遮りを無視した「影の利き」として数える。遮る駒に
依存しないので、マスごとの影の利きの数はトークン集合の線形関数になり、
差分更新が成立する。短い利き（歩・桂・銀・金・玉と馬龍の1マス部分）は
厳密のままである。

数えるのは、前後の局面で同じマスに同じ駒が残っていて、属性だけが
変わったトークンである。動いた駒と取られた駒は、属性がなくてもFTの
更新で触れるので数に入れない。

属性の粒度は3通りを同時に集計する。

- `both9`: 自分の利き数（影込み）0/1/2+ × 相手の利き数 0/1/2+ の9通り
- `split`: 短い利きと影の利きを分け、自他それぞれ0/1/2+に丸めた81通り
- `own3`: 自分の利き数（影込み）0/1/2+ だけの3通り

「自分」は駒の持ち主から見た側とする。同じマスでも先手の駒と後手の駒で
属性は変わる。

入力は対局順のpsv（40バイト固定長）で、連続する局面かどうかはgamePlyが
+1になっていることと、盤面の違いが1〜2マスに収まることで判定する。
対局の切れ目はまたがない。

駒コードと盤のマス番号はcshogiに合わせる。マス番号は `(筋 - 1) * 9 + 段`
で、筋は1から9、段はaを0とする。先手の前進は段が減る向きになる。

終了コード: 0=成功、2=引数エラー、3=実行時エラー（ADR-0122）。
"""

import argparse
import sys
import unicodedata

PSV_BYTES = 40
SFEN_BYTES = 32
PLY_OFFSET = 36

SQUARES = 81
RANKS = 9

# 駒コード。先手は1〜14、後手は+16（cshogiのBPAWN〜WPROM_ROOK）
PAWN, LANCE, KNIGHT, SILVER, BISHOP, ROOK, GOLD, KING = 1, 2, 3, 4, 5, 6, 7, 8
PROM_PAWN, PROM_LANCE, PROM_KNIGHT, PROM_SILVER = 9, 10, 11, 12
HORSE, DRAGON = 13, 14
WHITE = 16

# 先手から見た向き。dfは筋が増える向き、drは段が増える向き（先手の前は-1）
GOLD_STEPS = ((0, -1), (1, -1), (-1, -1), (1, 0), (-1, 0), (0, 1))
KING_STEPS = GOLD_STEPS + ((1, 1), (-1, 1))
DIAGONALS = ((1, 1), (1, -1), (-1, 1), (-1, -1))
ORTHOGONALS = ((0, 1), (0, -1), (1, 0), (-1, 0))

# 1マスの利き（厳密に数える分）
STEPS = {
    PAWN: ((0, -1),),
    KNIGHT: ((1, -2), (-1, -2)),
    SILVER: ((0, -1), (1, -1), (-1, -1), (1, 1), (-1, 1)),
    GOLD: GOLD_STEPS,
    KING: KING_STEPS,
    PROM_PAWN: GOLD_STEPS,
    PROM_LANCE: GOLD_STEPS,
    PROM_KNIGHT: GOLD_STEPS,
    PROM_SILVER: GOLD_STEPS,
    HORSE: ORTHOGONALS,
    DRAGON: DIAGONALS,
}
# 飛び利き（遮りを無視して盤端まで数える分）
RAYS = {
    LANCE: ((0, -1),),
    BISHOP: DIAGONALS,
    ROOK: ORTHOGONALS,
    HORSE: DIAGONALS,
    DRAGON: ORTHOGONALS,
}

# 反転数の分布を出すときの区切り
BUCKETS = ((0, 0), (1, 1), (2, 2), (3, 3), (4, 4), (5, 5), (6, 10), (11, None))
HIST_MAX = 128
GRAINS = ("both9", "split", "own3")
# 現行のFT更新が1手あたりに触れる行数（ADR-0213の読み方の基準）
FT_ROWS = "2〜3"


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
        prog="shadow-flips.py",
        description="影の利き属性の、1手あたりの反転数を測る（ADR-0213）。",
    )
    parser.add_argument("inputs", nargs="+", help="対局順のpsv（40バイト固定長）")
    parser.add_argument("--limit", type=int, default=1000000, help="先頭N局面だけ読む")
    parser.add_argument("--log", help="表を書き出す先。省くと標準出力だけに出す")
    return parser


def _squares(file_, rank):
    """盤内なら通し番号を返す。外なら None を返す。"""
    if 0 <= file_ < RANKS and 0 <= rank < RANKS:
        return file_ * RANKS + rank
    return None


def _build_tables():
    """駒コードとマスから、利きの先のマスを引く表を作る。

    後手は先手の向きを180度回す。1マスの利きの集合は左右対称なので、
    dfとdrの符号を同時に反転すれば足りる。
    """
    short, shadow = {}, {}
    for kind in range(PAWN, DRAGON + 1):
        for white in (False, True):
            code = kind + WHITE if white else kind
            sign = -1 if white else 1
            for sq in range(SQUARES):
                file_, rank = divmod(sq, RANKS)
                near = []
                for df, dr in STEPS.get(kind, ()):
                    target = _squares(file_ + sign * df, rank + sign * dr)
                    if target is not None:
                        near.append(target)
                far = []
                for df, dr in RAYS.get(kind, ()):
                    f, r = file_ + sign * df, rank + sign * dr
                    while (target := _squares(f, r)) is not None:
                        far.append(target)
                        f, r = f + sign * df, r + sign * dr
                short[code * SQUARES + sq] = tuple(near)
                shadow[code * SQUARES + sq] = tuple(far)
    return short, shadow


SHORT, SHADOW = _build_tables()


def counts(pieces):
    """マスごとの利きの数を、色と種別（短い・影）で4本に分けて数える。"""
    black_short = [0] * SQUARES
    black_shadow = [0] * SQUARES
    white_short = [0] * SQUARES
    white_shadow = [0] * SQUARES
    for sq, code in enumerate(pieces):
        if not code:
            continue
        if code > WHITE:
            near, far = white_short, white_shadow
        else:
            near, far = black_short, black_shadow
        key = code * SQUARES + sq
        for target in SHORT[key]:
            near[target] += 1
        for target in SHADOW[key]:
            far[target] += 1
    return black_short, black_shadow, white_short, white_shadow


def _cap(n):
    """0/1/2+ に丸める。"""
    return n if n < 2 else 2


def attributes(pieces):
    """盤上の駒の属性を、粒度ごとに81マスの配列で返す。空マスは0を置く。"""
    b_short, b_shadow, w_short, w_shadow = counts(pieces)
    both9 = [0] * SQUARES
    split = [0] * SQUARES
    own3 = [0] * SQUARES
    for sq, code in enumerate(pieces):
        if not code:
            continue
        if code > WHITE:
            near, far = w_short[sq], w_shadow[sq]
            opp_near, opp_far = b_short[sq], b_shadow[sq]
        else:
            near, far = b_short[sq], b_shadow[sq]
            opp_near, opp_far = w_short[sq], w_shadow[sq]
        own = _cap(near + far)
        opp = _cap(opp_near + opp_far)
        both9[sq] = own * 3 + opp
        split[sq] = ((_cap(near) * 3 + _cap(far)) * 3 + _cap(opp_near)) * 3 + _cap(
            opp_far
        )
        own3[sq] = own
    return both9, split, own3


def flips(prev_pieces, prev_attrs, pieces, attrs):
    """同じマスに残った駒のうち、属性が変わった数を粒度ごとに数える。"""
    out = [0, 0, 0]
    for sq in range(SQUARES):
        code = pieces[sq]
        if not code or code != prev_pieces[sq]:
            continue
        for k in range(3):
            if attrs[k][sq] != prev_attrs[k][sq]:
                out[k] += 1
    return out


def is_successor(prev_pieces, prev_ply, pieces, ply):
    """後の局面が、前の局面の1手後かを判定する。

    gamePlyが+1であることに加えて、盤面の違いが1〜2マスであることを見る。
    打つ手は1マス、動かす手と成りは2マスしか変わらない。対局の切れ目で
    たまたまgamePlyが揃っても、盤面の違いで弾ける。
    """
    if ply != prev_ply + 1:
        return False
    diff = 0
    for a, b in zip(prev_pieces, pieces):
        if a != b:
            diff += 1
            if diff > 2:
                return False
    return diff >= 1


def _records(paths, limit):
    """psvを先頭から読み、(盤面, gamePly) を順に返す。"""
    import numpy as np
    from cshogi import Board

    board = Board()
    read = 0
    for path in paths:
        with open(path, "rb") as fh:
            while read < limit:
                want = min(limit - read, 4096)
                raw = fh.read(want * PSV_BYTES)
                if len(raw) < PSV_BYTES:
                    break
                block = np.frombuffer(raw, dtype=np.uint8)
                n = len(block) // PSV_BYTES
                block = block[: n * PSV_BYTES].reshape(n, PSV_BYTES)
                for row in block:
                    board.set_psfen(row[:SFEN_BYTES].copy())
                    ply = int(row[PLY_OFFSET]) | (int(row[PLY_OFFSET + 1]) << 8)
                    yield board.pieces, ply
                read += n
        if read >= limit:
            break


def measure(paths, limit):
    """psvを対局順に読み、1手あたりの反転数の分布を粒度ごとに集める。"""
    hist = [[0] * (HIST_MAX + 1) for _ in GRAINS]
    positions = 0
    moves = 0
    breaks = 0
    prev_pieces = prev_attrs = None
    prev_ply = -1
    for pieces, ply in _records(paths, limit):
        attrs = attributes(pieces)
        positions += 1
        if prev_pieces is not None:
            if is_successor(prev_pieces, prev_ply, pieces, ply):
                moves += 1
                for k, n in enumerate(flips(prev_pieces, prev_attrs, pieces, attrs)):
                    hist[k][min(n, HIST_MAX)] += 1
            else:
                breaks += 1
        prev_pieces, prev_attrs, prev_ply = pieces, attrs, ply
    return {"hist": hist, "positions": positions, "moves": moves, "breaks": breaks}


def summary(hist):
    """ヒストグラムから平均と分位点を出す。分位点は順位で取る。"""
    total = sum(hist)
    if total == 0:
        return None
    mean = sum(n * c for n, c in enumerate(hist)) / total

    def rank(q):
        want = q * total
        seen = 0
        for n, c in enumerate(hist):
            seen += c
            if seen >= want:
                return n
        return len(hist) - 1

    return {"mean": mean, "p50": rank(0.5), "p90": rank(0.9), "p99": rank(0.99)}


def bucket_shares(hist):
    """分布を区切りごとの割合（%）にする。"""
    total = sum(hist)
    shares = []
    for lo, hi in BUCKETS:
        end = len(hist) if hi is None else hi + 1
        shares.append(100.0 * sum(hist[lo:end]) / total if total else 0.0)
    return shares


def _width(text):
    """端末での表示幅。全角を2桁と数える。"""
    return sum(2 if unicodedata.east_asian_width(c) in "WF" else 1 for c in text)


def _cell(text, width, left=False):
    """表示幅で揃えた桁を作る。"""
    pad = " " * max(width - _width(text), 0)
    return text + pad if left else pad + text


def _row(name, cells, widths):
    return _cell(name, widths[0], left=True) + "".join(
        _cell(text, w) for text, w in zip(cells, widths[1:])
    )


def report(result, paths, limit):
    """表を行のリストで返す。標準出力とログへ同じものを出す。"""
    lines = []
    lines.append("=== 影の利き属性の1手あたりの反転数（ADR-0213） ===")
    for path in paths:
        lines.append(f"入力  : {path}")
    lines.append(f"上限  : {limit:,} 局面")
    lines.append(f"局面数: {result['positions']:,}")
    lines.append(
        f"遷移数: {result['moves']:,}（対局の切れ目 {result['breaks']:,} 件は除く）"
    )
    lines.append("")
    lines.append(
        "数えたのは、前後で同じマスに同じ駒が残っていて属性が変わったトークンである。"
    )
    lines.append("動いた駒と取られた駒は、属性がなくてもFTが触れるので数に入れない。")
    lines.append("")
    widths = (10, 8, 8, 8, 8)
    lines.append(_row("粒度", ("平均", "中央値", "p90", "p99"), widths))
    for k, grain in enumerate(GRAINS):
        s = summary(result["hist"][k])
        if s is None:
            lines.append(_row(grain, ("（遷移なし）",), widths))
            continue
        cells = (f"{s['mean']:.2f}", str(s["p50"]), str(s["p90"]), str(s["p99"]))
        lines.append(_row(grain, cells, widths))
    lines.append(f"参考: 現行のFT更新は1手あたり{FT_ROWS}行である。")
    lines.append("")
    lines.append("反転数の分布（%）")
    labels = [
        str(lo) if hi == lo else (f"{lo}-{hi}" if hi else f"{lo}+") for lo, hi in BUCKETS
    ]
    dist_widths = (10,) + (7,) * len(labels)
    lines.append(_row("粒度", labels, dist_widths))
    for k, grain in enumerate(GRAINS):
        shares = [f"{v:.1f}" for v in bucket_shares(result["hist"][k])]
        lines.append(_row(grain, shares, dist_widths))
    return lines


def main(argv=None):
    """argvを省くとsys.argvを読む。hmwr diag shadowは引数リストで呼ぶ。"""
    args = build_parser().parse_args(argv)
    if args.limit <= 0:
        error(f"--limit は1以上で指定する: {args.limit}")
        return 2
    try:
        result = measure(args.inputs, args.limit)
        lines = report(result, args.inputs, args.limit)
    except (OSError, ValueError) as e:
        error(e)
        return 3
    except ImportError as e:
        error(f"cshogiとnumpyが要る: {e}")
        return 3
    text = "\n".join(lines)
    print(text)
    if args.log:
        with open(args.log, "a", encoding="utf-8") as fh:
            fh.write(text + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
