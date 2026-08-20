"""対局・計測・配布のためのビルド。"""

from __future__ import annotations

import argparse

from .. import config, paths, proc


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("build", help="エンジンをビルドする")
    ss = p.add_subparsers(dest="sub", metavar="<種類>")

    t = ss.add_parser(
        "pair",
        help="比較用の2本を同じ条件で作る",
        description="比較元（既定 origin/main）と現在のHEADから、"
        "data/bin/base-<名前> と cand-<名前> を作る。"
        "未コミットの変更があると中断する。",
    )
    t.add_argument("name", help="実験名")
    t.add_argument("--baseline", metavar="REF", help="比較元のref（既定 origin/main）")
    t.set_defaults(func=pair)

    t = ss.add_parser(
        "pgo",
        help="配布・対局用の単体ビルドを作る",
        description="計測用ビルド・ベンチ走行・最適化ビルドの3段で作る。"
        "NPSが+10%前後上がる。比較用のペアには使わない。",
    )
    t.add_argument("--out", metavar="パス", help="出力先（既定 data/bin/himawari-pgo）")
    t.set_defaults(func=pgo)

    t = ss.add_parser(
        "engine",
        help="エンジン本体をビルドする",
        description="計測と同じフラグでビルドする。",
    )
    t.add_argument("--arch", metavar="構成", help="例 512x16x64。省くと既定構成")
    t.set_defaults(func=engine)


def pair(args: argparse.Namespace) -> int:
    argv = [proc.script("build-pair.sh"), paths.check_name(args.name)]
    if args.baseline:
        argv.append(args.baseline)
    return proc.run(argv, dry_run=args.dry_run)


def pgo(args: argparse.Namespace) -> int:
    argv = [proc.script("build-pgo.sh")]
    if args.out:
        argv.append(args.out)
    return proc.run(argv, dry_run=args.dry_run)


def engine(args: argparse.Namespace) -> int:
    env = {"RUSTFLAGS": config.rustflags()}
    if args.arch:
        env["HIMAWARI_ARCH"] = args.arch
    return proc.run(
        ["cargo", "build", "--release", "-p", "himawari-usi"],
        dry_run=args.dry_run,
        env=env,
    )
