"""挙動と速度の測定。

固定深さでノード数を比べる `verify`、NPSを測る `bench`、時間の内訳を見る
`profile` を持つ。いずれも実体は crates/tools のRustバイナリで、USIエンジンを
起動して測る仕事はそちらにある（ADR-0122）。

動作環境の最適なスレッド数を測る `threadtune` もここに置く。実体だけは
エンジン本体のモードだが、やることはNPSと自己対局による測定で、`bench` の
隣にあるのが読み手に分かりやすい（ADR-0203）。
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
        "マシンの温度や背景の負荷でNPSは数%動くため、1本ずつ別に測った値を"
        "比べない。評価関数をまたぐときは --nodes で打ち切る。",
    )
    p.add_argument("binaries", nargs="+", metavar="バイナリ")
    p.add_argument("--depth", type=int, metavar="N", help="探索の深さ（既定 19）")
    p.add_argument("--nodes", type=int, metavar="N", help="深さの代わりにノード数で打ち切る")
    p.add_argument("--runs", type=int, metavar="N", help="1本を何周測るか")
    p.add_argument("--positions", metavar="パス", help="局面リスト。既定は組み込みの4局面")
    p.add_argument("--eval-file", metavar="パス", help="評価関数")
    p.add_argument("--threads", type=int, metavar="N", help="探索スレッド数。2以上で深さ到達の計測になる")
    p.add_argument("--hash", type=int, metavar="MB", help="置換表の大きさ")
    p.add_argument("--log", metavar="名前", help="ログを残す名前")
    p.set_defaults(func=bench)

    p = sub.add_parser(
        "threadtune",
        help="動作環境の最適なスレッド数を測る",
        description="持続NPSで候補を絞ってから、現在の設定を起点に自己対局で"
        "隣の候補と比べる。放流と同じ電源モードとAC接続で、一晩かけて測る。"
        "候補の上限は論理コア−2にしてある。実戦では指し手を中継する"
        "クライアントが別プロセスで動くためである。",
    )
    p.add_argument("--eval", metavar="名前", help="評価関数（既定は現行のネット）")
    p.add_argument("--build", metavar="名前", help="data/bin のビルドで測る（既定はいまの木）")
    p.add_argument("--current", type=int, metavar="N", help="いま使っているスレッド数")
    p.add_argument("--hours", type=float, metavar="H", help="自己対局にかける時間（既定 4）")
    p.add_argument("--tc", metavar="持ち時間", help="自己対局の持ち時間（既定 10+0.1）")
    p.add_argument("--hash", type=int, metavar="MB", help="置換表の大きさ")
    p.add_argument("--max-threads", type=int, metavar="N", help="候補の上限")
    p.add_argument("--warmup-secs", type=int, metavar="秒", help="NPSを測る前の暖機")
    p.add_argument("--pairs", type=int, metavar="N", help="1組のペア数。--hours の代わりに使う")
    p.add_argument("--nps-only", action="store_true", help="NPSの表だけ出して終わる")
    p.add_argument("--log", metavar="名前", help="ログを残す名前（既定は評価関数の名前）")
    p.set_defaults(func=threadtune)


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
    if args.threads:
        extra += ["--threads", str(args.threads)]
    if args.hash:
        extra += ["--hash", str(args.hash)]
    if args.eval_file:
        extra += ["--eval-file", args.eval_file]
    return proc.run(
        proc.cargo_tool("bench", [*args.binaries, *extra]),
        dry_run=args.dry_run,
        env=config.measure_env(),
        log=paths.log("bench", args.log) if args.log else None,
    )


def threadtune(args: argparse.Namespace) -> int:
    """エンジン本体の threadtune モードを呼ぶ。

    足すのは3つだけである。評価関数を名前から解決すること、省いたときに
    現行のネットを使うこと、ログを data/logs/threadtune-<名前>.log へ
    決めることである。測り方の既定はエンジン側が持つので、渡された
    フラグだけを通す。
    """
    if args.eval:
        eval_file = str(paths.NETS / f"{paths.check_name(args.eval)}.hmwr")
    else:
        eval_file = config.get("EVAL_FILE")

    extra: list[str] = ["--eval", eval_file]
    for flag, value in (
        ("--hours", args.hours),
        ("--current", args.current),
        ("--tc", args.tc),
        ("--hash", args.hash),
        ("--max-threads", args.max_threads),
        ("--warmup-secs", args.warmup_secs),
        ("--pairs", args.pairs),
    ):
        if value is not None:
            extra += [flag, str(value)]
    if args.nps_only:
        extra.append("--nps-only")

    if args.build:
        engine = paths.BIN / paths.check_name(args.build)
        if not engine.is_file() and not args.dry_run:
            raise proc.Fail(f"ビルドがない: {paths.rel(engine)}")
        argv = [str(engine), "threadtune", *extra]
    else:
        argv = proc.cargo_tool("himawari", ["threadtune", *extra], package="himawari-usi")

    return proc.run(
        argv,
        dry_run=args.dry_run,
        env=config.measure_env(),
        log=paths.log("threadtune", args.log or Path(eval_file).stem),
    )
