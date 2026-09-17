"""単発の診断。1本のADRのために作った測定の置き場（ADR-0208）。

**ここは安定した表面ではない。** 引数の互換は保たず、使ったADRが閉じて
90日たったら消してよい。消したものはgitの履歴とADRから掘り起こせる。
診断を足すときはここに置き、2本目のADRで使われたら正規の領域への昇格を
検討する。
"""

from __future__ import annotations

import argparse
from pathlib import Path

from .. import config, paths, proc
from ..tools import dead_dims, phase as phase_tool, rank_diag


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser(
        "diag",
        help="単発の診断（互換は保たない）",
        description="1つの実験のために作った測定を置く。引数の互換は保たず、"
        "使わなくなったものは消す。",
    )
    ss = p.add_subparsers(dest="sub", metavar="<診断>")

    t = ss.add_parser(
        "rank",
        help="ランキング損失のヒンジ発火を分ける",
        description="正例の葉が負例の葉より良いかを群ごとに測り、"
        "発火を「順序が逆」と「マージン不足」へ分ける。"
        "前者はαを上げる線、後者はδを動かす線につながる。",
    )
    t.add_argument("weights", metavar="重み", help="チェックポイントかネット")
    t.add_argument("rank_data", metavar="群", help="psv rank が書いた *.rankpsv")
    t.add_argument("--margin", type=float, metavar="X", help="ヒンジのマージン")
    t.add_argument("--groups", type=int, metavar="N", help="測る群の数")
    t.add_argument("--seed", type=int, metavar="N", help="群を引く乱数の種")
    t.add_argument("--threads", type=int, metavar="N", help="torchのスレッド数")
    t.set_defaults(func=rank)

    t = ss.add_parser(
        "dead",
        help="FT出力の対が死ぬ原因をa側とb側に分けて測る",
        description="対の積は片側がゼロなら結果もゼロになる。"
        "積が死ぬ原因が片側の死なのか、両側が同時に発火しないのかを分ける。"
        "活性は学習器と同じf32で測るので、量子化後の値を見る reorder とは"
        "ゼロ率が変わる。",
    )
    t.add_argument("weights", metavar="重み", help="チェックポイントかネット")
    t.add_argument("valid", metavar="PSV", help="測る局面")
    t.add_argument("--batch", type=int, metavar="N", help="バッチの大きさ")
    t.add_argument("--threads", type=int, metavar="N", help="torchのスレッド数")
    t.set_defaults(func=dead)

    t = ss.add_parser(
        "phase",
        help="進行度の指標の候補を、評価の系統誤差で比べる",
        description="psv phase で局面ごとの指標と静的評価をTSVへ書き、"
        "指標ごとに4クラスへ切ってクラス別の補正が損失を下げる量を比べる。"
        "補正は出力のアフィン（2パラメータ）と、L2活性の線形ヘッド"
        "（最終段だけを分岐する案と同じ容量）の2段で測る。"
        "TSVは data/profile/phase-<名前>.tsv に残る。",
    )
    t.add_argument("psv", metavar="PSV", help="測る局面")
    t.add_argument("--eval-file", metavar="パス", help="評価関数（既定は EVAL_FILE）")
    t.add_argument("--limit", type=int, metavar="N", help="先頭N局面だけ測る")
    t.add_argument("--lambda", type=float, dest="lambda_", metavar="X", help="目標の混合比（既定0.7）")
    t.add_argument("--seed", type=int, metavar="N", help="乱数分割の種")
    t.set_defaults(func=phase)


def rank(args: argparse.Namespace) -> int:
    """ランキング損失のヒンジ発火の内訳を測る。"""
    if args.dry_run:
        print(f"[dry-run] ヒンジの発火を分ける: {args.weights} × {args.rank_data}")
        return proc.OK
    for path, what in ((args.weights, "重み"), (args.rank_data, "群")):
        if not Path(path).is_file():
            raise proc.Fail(f"{what}がない: {path}")
    argv = [args.weights, args.rank_data]
    for name in ("margin", "groups", "seed", "threads"):
        value = getattr(args, name, None)
        if value is not None:
            argv += [f"--{name}", str(value)]
    return rank_diag.main(argv)


def dead(args: argparse.Namespace) -> int:
    """FT出力の対が死ぬ原因を測る。"""
    if args.dry_run:
        print(f"[dry-run] 対の死に方を測る: {args.weights} × {args.valid}")
        return proc.OK
    for path, what in ((args.weights, "重み"), (args.valid, "局面")):
        if not Path(path).is_file():
            raise proc.Fail(f"{what}がない: {path}")
    argv = [args.weights, args.valid]
    if args.batch:
        argv += ["--batch", str(args.batch)]
    if args.threads:
        argv += ["--threads", str(args.threads)]
    return dead_dims.main(argv)


def phase(args: argparse.Namespace) -> int:
    """進行度の指標の候補を、クラス別の系統誤差で比べる（ADR-0198）。"""
    psv = Path(args.psv)
    if not psv.is_file() and not args.dry_run:
        raise proc.Fail(f"局面がない: {args.psv}")
    eval_file = args.eval_file or config.get("EVAL_FILE")
    if not eval_file and not args.dry_run:
        raise proc.Fail("評価関数がない。--eval-file で渡す")
    name = paths.check_name(psv.name.removesuffix(".psv"))
    tsv = paths.PROFILE / f"phase-{name}.tsv"
    tsv.parent.mkdir(parents=True, exist_ok=True)

    print(f"=== 進行度の指標: {name} ===")
    print(f"局面    : {paths.rel(psv)}")
    print(f"評価関数: {paths.rel(eval_file or '（未設定）')}")
    print(f"TSV     : {paths.rel(tsv)}")
    argv = ["--in", str(psv), "--out", str(tsv), "--eval-file", eval_file or "（未設定）"]
    if args.limit is not None:
        argv += ["--limit", str(args.limit)]
    code = proc.run(
        proc.cargo_tool("psv", ["phase", *argv]),
        dry_run=args.dry_run,
        log=paths.log("phase", name),
    )
    if code != proc.OK:
        return code
    if args.dry_run:
        print(f"[dry-run] 指標ごとの系統誤差を集計する: {paths.rel(tsv)}")
        return proc.OK
    # L2活性の線形ヘッド（ADR-0137の容量）も同じネットで測る
    tool_argv = [str(tsv), "--weights", eval_file, "--psv", str(psv)]
    if args.lambda_ is not None:
        tool_argv += ["--lambda", str(args.lambda_)]
    if args.seed is not None:
        tool_argv += ["--seed", str(args.seed)]
    return phase_tool.main(tool_argv)
