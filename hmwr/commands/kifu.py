"""実戦棋譜の回収と分析。

公開対局場の棋譜を定期的に回収し、分析と定跡追加を決まった手順で回す。
分析と定跡追加は入力集合・エンジン・評価関数・探索条件の純関数で、
定跡追加は冪等である（ADR-0152）。
"""

from __future__ import annotations

import argparse
import datetime
from pathlib import Path

from .. import config, paths, proc
from ..tools import floodgate
from . import book as book_cmd


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("kifu", help="実戦の棋譜を回収・分析する")
    ss = p.add_subparsers(dest="sub", metavar="<操作>")

    t = ss.add_parser("fetch", help="対局場から棋譜を回収する")
    t.add_argument("--player-url", metavar="URL", help="対局者ページ")
    t.add_argument("--out", metavar="ディレクトリ", help="置き場")
    t.add_argument("--max-files", type=int, metavar="N", help="取得の上限")
    t.set_defaults(func=fetch)

    t = ss.add_parser(
        "report",
        help="棋譜を分析してレポートを書く",
        description="実戦の評価値の系列から、崩壊のきっかけになった局面を"
        "拾う。再解析より安い診断材料になる。",
    )
    t.add_argument("games", nargs="?", metavar="ディレクトリ", help="棋譜の置き場")
    t.add_argument("--out", metavar="ファイル", help="レポートの出力先")
    t.set_defaults(func=report)

    t = ss.add_parser(
        "cycle",
        help="回収・分析・定跡追加・網羅率を続けて回す",
        description="定期実行の手順を固定する。定跡追加は1局面あたり"
        "深さ28で約34秒かかるので、1回の追加数を絞って残りは次回へ回す。",
    )
    t.add_argument("--year", type=int, default=2026, metavar="年", help="対象年")
    t.add_argument("--seed-max", type=int, default=50, metavar="N", help="1回の定跡追加数")
    t.set_defaults(func=cycle)


def _games_dir(year: int) -> Path:
    return paths.RAW / "floodgate" / str(year)


def fetch(args: argparse.Namespace) -> int:
    argv: list[str] = []
    if args.player_url:
        argv += ["--player-url", args.player_url]
    if args.out:
        argv += ["--out", args.out]
    if args.max_files:
        argv += ["--max-files", str(args.max_files)]
    if args.dry_run:
        argv.append("--dry-run")
    return floodgate.main(argv)


def report(args: argparse.Namespace) -> int:
    games = Path(args.games) if args.games else _games_dir(2026)
    out = Path(args.out) if args.out else _default_report()
    binary = paths.REPO / "target" / "release" / "kifu"
    if not binary.is_file() and not args.dry_run:
        raise proc.Fail(f"{paths.rel(binary)} がない。先に cargo build --release を実行する")
    return proc.run(
        [
            str(binary),
            str(paths.REPO / "target" / "release" / "himawari"),
            str(games),
            "--eval-file", config.get("EVAL_FILE", "（未設定）"),
            "--out", str(out),
        ],
        dry_run=args.dry_run,
        log=paths.log("kifu", "report"),
    )


def _default_report() -> Path:
    stamp = datetime.date.today().strftime("%Y%m%d")
    return paths.LOGS / f"floodgate-report-{stamp}.md"


def cycle(args: argparse.Namespace) -> int:
    """回収・分析・定跡追加・網羅率の4段を順に回す。"""
    games = _games_dir(args.year)
    report_path = _default_report()

    print(f"=== 実戦棋譜のサイクル: {args.year}年 ===")

    print("1/4: 回収")
    fetch(argparse.Namespace(player_url=None, out=None, max_files=None, dry_run=args.dry_run))

    if not games.is_dir() and not args.dry_run:
        raise proc.Fail(f"棋譜がない: {paths.rel(games)}")

    print(f"2/4: 分析レポート → {paths.rel(report_path)}")
    proc.run(
        ["cargo", "build", "--release", "--quiet"],
        dry_run=args.dry_run,
        env={"RUSTFLAGS": config.rustflags()},
    )
    report(
        argparse.Namespace(games=str(games), out=str(report_path), dry_run=args.dry_run)
    )

    print(f"3/4: 定跡追加（最大{args.seed_max}局面。冪等なので続きから足す）")
    book_cmd.seed(
        argparse.Namespace(
            games=str(games),
            out=None,
            depth=28,
            max_positions=args.seed_max,
            dry_run=args.dry_run,
        )
    )

    print("4/4: 網羅率")
    book_cmd.stats(argparse.Namespace(out=None, dry_run=args.dry_run))

    print()
    print(f"レポート: {paths.rel(report_path)}")
    return proc.OK
