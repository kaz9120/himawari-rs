#!/usr/bin/env python3
"""リーグ戦の棋譜（jsonl）から相対Eloを集計する（ADR-0128）。

`league` は実行しながら集計するので、途中で止まると結果表が残らない。
棋譜さえあれば後から同じ推定ができるように、集計だけを切り出す。
中断したリーグ戦の途中経過を見るのにも使う。

使い方:
  scripts/league-summary.py <棋譜.jsonl> [--anchor <名前>]
"""

import argparse
import collections
import json
import math
import sys


def read_games(path):
    """棋譜を読み、(a, b) ごとの [勝ち, 引き分け, 負け] を返す。aから見た数。"""
    table = collections.defaultdict(lambda: [0, 0, 0])
    players = []
    n = 0
    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            g = json.loads(line)
            a, b = g["a"], g["b"]
            for name in (a, b):
                if name not in players:
                    players.append(name)
            if g["winner"] == "draw":
                idx = 1
            elif g["winner"] == g["a_side"]:
                idx = 0
            else:
                idx = 2
            table[(a, b)][idx] += 1
            n += 1
    return players, table, n


def solve_elo(players, table, max_iterations=10_000, epsilon=1e-4, max_step=50.0):
    """勝敗表からEloを最尤で解く。league.rs の solve_elo と同じ手順。"""
    rating = {p: 0.0 for p in players}
    # (i, j) -> [勝ち, 分け, 負け] を双方向に展開する
    pair = collections.defaultdict(lambda: [0, 0, 0])
    for (a, b), (w, d, l) in table.items():
        pair[(a, b)][0] += w
        pair[(a, b)][1] += d
        pair[(a, b)][2] += l
        pair[(b, a)][0] += l
        pair[(b, a)][1] += d
        pair[(b, a)][2] += w

    for _ in range(max_iterations):
        moved = 0.0
        for i in players:
            actual = expected = derivative = 0.0
            for j in players:
                if i == j:
                    continue
                w, d, l = pair[(i, j)]
                games = w + d + l
                if games == 0:
                    continue
                actual += w + 0.5 * d
                p = 1.0 / (1.0 + 10 ** ((rating[j] - rating[i]) / 400.0))
                expected += games * p
                derivative += games * p * (1.0 - p) * math.log(10) / 400.0
            if derivative <= 0.0:
                continue
            step = max(-max_step, min(max_step, (actual - expected) / derivative))
            rating[i] += step
            moved = max(moved, abs(step))
        if moved < epsilon:
            break
    return rating, pair


def stderr_of(i, players, pair, rating):
    """標準誤差の目安（Elo）。対戦数だけから出す。"""
    information = 0.0
    for j in players:
        if i == j:
            continue
        games = sum(pair[(i, j)])
        if games == 0:
            continue
        p = 1.0 / (1.0 + 10 ** ((rating[j] - rating[i]) / 400.0))
        d = math.log(10) / 400.0
        information += games * p * (1.0 - p) * d * d
    return float("inf") if information <= 0 else 1.0 / math.sqrt(information)


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("jsonl", help="league が書いた棋譜")
    ap.add_argument("--anchor", help="この参加者を0に揃える。省略時は平均が0")
    args = ap.parse_args()

    players, table, n = read_games(args.jsonl)
    if not players:
        sys.exit("棋譜が空")
    rating, pair = solve_elo(players, table)

    offset = rating[args.anchor] if args.anchor else sum(rating.values()) / len(rating)
    for p in rating:
        rating[p] -= offset

    order = sorted(players, key=lambda p: -rating[p])
    print(f"=== {n}局 ===")
    print()
    print("| 参加者 | Elo | ±2SE | 得点 | 対局 |")
    print("|---|---|---|---|---|")
    for p in order:
        points = games = 0.0
        for j in players:
            if p == j:
                continue
            w, d, l = pair[(p, j)]
            points += w + 0.5 * d
            games += w + d + l
        se = stderr_of(p, players, pair, rating)
        print(f"| {p} | {rating[p]:+.1f} | {2 * se:.1f} | {points:.1f}/{games:.0f} | {games:.0f} |")

    print()
    print("勝敗表（行から見た +勝 =分 -負）")
    print("| |" + "".join(f" {p} |" for p in order))
    print("|---|" + "---|" * len(order))
    for i in order:
        cells = []
        for j in order:
            if i == j:
                cells.append(" — |")
            else:
                w, d, l = pair[(i, j)]
                cells.append(f" +{w} ={d} -{l} |")
        print(f"| {i} |" + "".join(cells))


if __name__ == "__main__":
    main()
