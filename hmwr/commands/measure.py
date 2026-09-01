"""挙動と速度の測定。

固定深さでノード数を比べる `verify`、NPSを測る `bench`、時間の内訳を見る
`profile` を持つ。いずれも実体は crates/tools のRustバイナリで、USIエンジンを
起動して測る仕事はそちらにある（ADR-0122）。
"""

from __future__ import annotations

import argparse
from pathlib import Path

from .. import config, paths, proc


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser(
        "verify",
        help="固定深さのノード数を比べ、挙動が変わったかを見る",
        description="実験名を1つ渡すと data/bin/base-<名前> と cand-<名前> を"
        "比べる。バイナリを直接並べてもよい。"
        "全局面でノード数が一致したら終了コード1を返す。"
        "その変更は探索に影響しておらず、対局にかけても中立にしかならない。",
    )
    p.add_argument("targets", nargs="+", metavar="<名前 | バイナリ...>")
    p.add_argument("--depth", type=int, metavar="N", help="探索の深さ（既定 13）")
    p.add_argument("--positions", metavar="パス", help="局面リスト。既定は組み込みの4局面")
    p.add_argument("--eval-file", metavar="パス", help="評価関数")
    p.add_argument("--log", metavar="名前", help="ログを残す名前")
    p.set_defaults(func=verify)

    p = sub.add_parser(
        "bench",
        help="固定深さでNPSを測る",
        description="2本以上を並べると交互に測る。"
        "機体の温度や背景の負荷でNPSは数%動くため、1本ずつ別に測った値を"
        "比べない。評価関数をまたぐときは --nodes で打ち切る。",
    )
    p.add_argument("binaries", nargs="+", metavar="バイナリ")
    p.add_argument("--depth", type=int, metavar="N", help="探索の深さ（既定 19）")
    p.add_argument("--nodes", type=int, metavar="N", help="深さの代わりにノード数で打ち切る")
    p.add_argument("--runs", type=int, metavar="N", help="1本を何周測るか")
    p.add_argument("--positions", metavar="パス", help="局面リスト。既定は組み込みの4局面")
    p.add_argument("--eval-file", metavar="パス", help="評価関数")
    p.add_argument("--log", metavar="名前", help="ログを残す名前")
    p.set_defaults(func=bench)


def verify(args: argparse.Namespace) -> int:
    given = args.targets
    if len(given) == 1 and not Path(given[0]).is_file():
        name = paths.check_name(given[0])
        base, cand = paths.BIN / f"base-{name}", paths.BIN / f"cand-{name}"
        for path in (base, cand):
            if not path.is_file() and not args.dry_run:
                raise proc.Fail(
                    f"バイナリがない: {paths.rel(path)}\n"
                    f"hmwr build pair {name} で作る"
                )
        targets = [str(base), str(cand)]
        log = paths.log("verify", name)
    else:
        targets = given
        log = paths.log("verify", args.log) if args.log else None

    extra: list[str] = []
    if args.depth:
        extra += ["--depth", str(args.depth)]
    if args.positions:
        extra += ["--positions", args.positions]
    if args.eval_file:
        extra += ["--eval-file", args.eval_file]
    return proc.run(
        proc.cargo_tool("verify", [*targets, *extra]),
        dry_run=args.dry_run,
        env=config.measure_env(),
        log=log,
        allowed=(proc.OK, proc.JUDGE),
    )


def bench(args: argparse.Namespace) -> int:
    extra: list[str] = []
    if args.depth:
        extra += ["--depth", str(args.depth)]
    if args.nodes:
        extra += ["--nodes", str(args.nodes)]
    if args.runs:
        extra += ["--runs", str(args.runs)]
    if args.positions:
        extra += ["--positions", args.positions]
    if args.eval_file:
        extra += ["--eval-file", args.eval_file]
    return proc.run(
        proc.cargo_tool("bench", [*args.binaries, *extra]),
        dry_run=args.dry_run,
        env=config.measure_env(),
        log=paths.log("bench", args.log) if args.log else None,
    )
