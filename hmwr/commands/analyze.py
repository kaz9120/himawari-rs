"""総当たり戦とプロファイルの集計。

どちらも「外部が出した結果を読んで表にする」仕事で、実行そのものは
別のツールが持つ。
"""

from __future__ import annotations

import argparse
from pathlib import Path

from .. import config, paths, proc
from ..tools import league as league_tool
from ..tools import profile as profile_tool


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("league", help="総当たり戦を回す・集計する")
    ss = p.add_subparsers(dest="sub", metavar="<操作>")

    t = ss.add_parser("run", help="総当たり戦を回す")
    t.add_argument("engines", nargs="+", metavar="バイナリ")
    t.add_argument("--games", type=int, metavar="N", help="1組あたりの対局数")
    t.add_argument("--out", metavar="JSONL", help="棋譜の出力先")
    t.set_defaults(func=league_run)

    t = ss.add_parser(
        "summary",
        help="棋譜から相対Eloを集計する",
        description="対局中の集計は途中で止まると残らない。棋譜さえあれば"
        "後から同じ推定ができる。",
    )
    t.add_argument("jsonl", metavar="棋譜")
    t.add_argument("--anchor", metavar="参加者", help="この参加者を0に揃える")
    t.set_defaults(func=league_summary)

    p = sub.add_parser("profile", help="時間の内訳を測る・集計する")
    ss = p.add_subparsers(dest="sub", metavar="<操作>")

    t = ss.add_parser("record", help="プロファイルを取る")
    t.add_argument("binary", metavar="バイナリ")
    t.add_argument("--depth", type=int, metavar="N", help="探索の深さ")
    t.add_argument("--out", metavar="ファイル", help="出力先")
    t.set_defaults(func=profile_record)

    t = ss.add_parser(
        "report",
        help="self時間の上位を出す",
        description="関数単位とソース行単位の2つを出す。行番号には"
        "デバッグ情報が要る。",
    )
    t.add_argument("profile", metavar="プロファイル")
    t.add_argument("binary", nargs="?", metavar="バイナリ", help="デバッグ情報付き")
    t.add_argument("--top", type=int, default=20, metavar="N", help="上位何件")
    t.set_defaults(func=profile_report)


def league_run(args: argparse.Namespace) -> int:
    extra: list[str] = []
    if args.games:
        extra += ["--games", str(args.games)]
    out = args.out or str(paths.SPRT / "league.jsonl")
    return proc.run(
        proc.cargo_tool("league", [*args.engines, "--out", out, *extra]),
        dry_run=args.dry_run,
        env=config.measure_env(),
        log=paths.log("league", "run"),
    )


def league_summary(args: argparse.Namespace) -> int:
    argv = [args.jsonl]
    if args.anchor:
        argv += ["--anchor", args.anchor]
    if args.dry_run:
        print(f"[dry-run] 相対Eloを集計する: {args.jsonl}")
        return proc.OK
    return league_tool.main(argv)


def profile_record(args: argparse.Namespace) -> int:
    extra: list[str] = []
    if args.depth:
        extra += ["--depth", str(args.depth)]
    if args.out:
        extra += ["--out", args.out]
    return proc.run(
        proc.cargo_tool("profile", [args.binary, *extra]),
        dry_run=args.dry_run,
        env=config.measure_env(),
    )


def profile_report(args: argparse.Namespace) -> int:
    argv = [args.profile]
    if args.binary:
        argv.append(args.binary)
    argv += ["--top", str(args.top)]
    if args.dry_run:
        print(f"[dry-run] プロファイルを集計する: {args.profile}")
        return proc.OK
    if not Path(args.profile).is_file():
        raise proc.Fail(f"プロファイルがない: {args.profile}")
    return profile_tool.main(argv)
