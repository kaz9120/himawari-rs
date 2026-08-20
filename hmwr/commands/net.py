"""評価関数の学習・評価・配布。"""

from __future__ import annotations

import argparse

from .. import paths, proc


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("net", help="評価関数を学習・評価・配布する")
    ss = p.add_subparsers(dest="sub", metavar="<操作>")

    t = ss.add_parser(
        "train",
        help="本番規模のネットを学習する",
        description="data/nets/<名前>.hmwr へ書き出し、"
        "training/checkpoints/<名前>/ へ途中経過を残す。"
        "検証データは学習データと同じ前処理に揃える。"
        "土俵がずれると最良チェックポイントの選択が歪む。",
    )
    t.add_argument("name", help="ネット名。ログと台帳の名前にもなる")
    t.add_argument("--data", required=True, metavar="PSV", help="学習データ")
    t.add_argument("--valid", metavar="PSV", help="検証データ")
    t.add_argument("--init-ckpt", metavar="パス", help="継続学習の初期値")
    t.add_argument("--lr", metavar="値", help="学習率の頂点。継続学習の既定は1e-4")
    t.add_argument("--warmup", type=int, metavar="N", help="学習率を上げきるまでのステップ数")
    t.add_argument("--device", metavar="名前", help="mps か cpu（既定 mps）")
    t.add_argument("--seed", type=int, metavar="N", help="乱数種（既定 0）")
    t.add_argument("--notes", metavar="文", help="実験台帳へ書く備考")
    t.add_argument(
        "--extra",
        metavar="引数",
        help="学習器へ素通しする追加引数。ハイフンで始まる値は --extra=--flag と書く",
    )
    t.set_defaults(func=train)

    t = ss.add_parser(
        "eval",
        help="ネットの検証損失を測る",
        description="学習は回さず、書き出したネットを並べて測る。"
        "**採否は対局で決める。** 検証損失は初期値の系列が違うだけで動く。",
    )
    t.add_argument("nets", nargs="+", metavar="ネット")
    t.add_argument("--valid", metavar="PSV", help="検証集合。複数はコンマ区切り")
    t.set_defaults(func=evaluate)

    t = ss.add_parser(
        "release",
        help="ネットをGitHub Releaseで配る",
        description="既定では作らない。走るはずのコマンドとノートを出して終わる。"
        "実際に作るには --apply を付ける。",
    )
    t.add_argument("file", metavar="ネット")
    t.add_argument("version", type=int, metavar="番号")
    t.add_argument("--notes", metavar="文", help="リリースノートへの追記")
    t.add_argument("--apply", action="store_true", help="実際に作る")
    t.set_defaults(func=release)


def train(args: argparse.Namespace) -> int:
    """フラグを学習スクリプトの環境変数へ畳む。"""
    env: dict[str, str] = {}
    if args.init_ckpt:
        env["TRAIN_INIT_CKPT"] = args.init_ckpt
    if args.lr:
        env["TRAIN_PEAK_LR"] = args.lr
    if args.warmup:
        env["TRAIN_WARMUP"] = str(args.warmup)
    if args.device:
        env["TRAIN_DEVICE"] = args.device
    if args.seed is not None:
        env["TRAIN_SEED"] = str(args.seed)
    if args.notes:
        env["TRAIN_NOTES"] = args.notes
    if args.extra:
        env["TRAIN_EXTRA_ARGS"] = args.extra

    argv = [proc.script("train-net.sh"), paths.check_name(args.name), args.data]
    if args.valid:
        argv.append(args.valid)
    return proc.run(argv, dry_run=args.dry_run, env=env)


def evaluate(args: argparse.Namespace) -> int:
    env = {"EVAL_VALIDS": args.valid} if args.valid else {}
    return proc.run(
        [proc.script("eval-net.sh"), *args.nets],
        dry_run=args.dry_run,
        env=env,
    )


def release(args: argparse.Namespace) -> int:
    argv = [proc.script("release-net.sh"), args.file, str(args.version)]
    if args.notes:
        argv.append(args.notes)
    if args.apply:
        argv.append("--apply")
    return proc.run(argv, dry_run=args.dry_run)
