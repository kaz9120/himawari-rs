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
from ..tools import (
    dead_dims,
    phase as phase_tool,
    rank_diag,
    replay as replay_tool,
    shadow_flips,
)


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser(
        "diag",
        help="単発の診断（互換は保たない）",
        description="1つの実験のために作った測定を置く。引数の互換は保たず、"
        "使わなくなったものは消す。",
    )
    ss = p.add_subparsers(dest="sub", metavar="<診断>")

    t = ss.add_parser(
        "replay",
        help="CSAの棋譜を、実戦の持ち時間のまま指し直す",
        description="単発の局面では出ないが対局の流れの中で出る問題を再現する。"
        "置換表・時間管理・ponderの状態は前の手から持ち越されるので、"
        "局面だけを渡す検討では見えない。自分の手番ごとに残り時間で go を送り、"
        "返るまでの時間と最終の反復を出す。時間制の対局と重ねて走らせない。",
    )
    t.add_argument("csa", metavar="CSA", help="棋譜")
    t.add_argument("--engine", metavar="パス", help="エンジン（既定 target/release/himawari）")
    t.add_argument("--eval-file", metavar="パス", help="評価関数（既定は EVAL_FILE）")
    t.add_argument("--player", default="Himawari", metavar="名前", help="自分の側の名前")
    t.add_argument("--threads", type=int, default=4, metavar="N")
    t.add_argument("--from", dest="start_ply", type=int, default=1, metavar="手数", help="この手から探索する")
    t.add_argument("--initial", type=int, default=300, metavar="秒", help="持ち時間")
    t.add_argument("--inc", type=int, default=10, metavar="秒", help="1手ごとの加算")
    t.set_defaults(func=replay)

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

    t = ss.add_parser(
        "shadow",
        help="影の利き属性の、1手あたりの反転数を測る",
        description="駒トークンに利きの数を属性として足す案のコストを測る。"
        "飛び駒の利きは遮りを無視した「影の利き」として数え、短い利きは厳密に数える。"
        "対局順のpsvを再生し、前後で同じマスに残った駒のうち属性が変わった数を"
        "粒度ごとに集計する。動いた駒と取られた駒は、属性がなくてもFTが触れるので"
        "数に入れない。",
    )
    t.add_argument("inputs", nargs="+", metavar="入力", help="対局順のpsv（data/raw/<名前>/*.bin）")
    t.add_argument("--limit", type=int, default=1000000, metavar="N", help="先頭N局面だけ読む")
    t.add_argument("--name", metavar="名前", help="ログの名前（既定は入力の置き場の名前）")
    t.set_defaults(func=shadow)


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


def _shadow_name(inputs: list[str]) -> str:
    """ログの名前を入力から決める。置き場の名前を使い、駄目なら stem を使う。"""
    first = Path(inputs[0])
    for candidate in (first.parent.name, first.stem):
        try:
            return paths.check_name(candidate)
        except paths.BadName:
            continue
    return "shadow"


def shadow(args: argparse.Namespace) -> int:
    """影の利き属性の、1手あたりの反転数を測る（ADR-0213）。"""
    name = paths.check_name(args.name) if args.name else _shadow_name(args.inputs)
    if args.dry_run:
        for path in args.inputs:
            print(f"[dry-run] 入力: {paths.rel(path)}")
        print(f"[dry-run] 先頭{args.limit:,}局面の反転数を測る")
        print(f"[dry-run] ログ: {paths.rel(paths.log('diag-shadow', name))}")
        return proc.OK
    for path in args.inputs:
        if not Path(path).is_file():
            raise proc.Fail(f"入力がない: {path}")
    log = paths.log("diag-shadow", name)
    argv = [*args.inputs, "--limit", str(args.limit), "--log", str(log)]
    code = shadow_flips.main(argv)
    if code == proc.OK:
        print(f"ログ: {paths.rel(log)}")
    return code


def replay(args: argparse.Namespace) -> int:
    """対局の流れを再現する。"""
    engine = args.engine or str(paths.release_bin("himawari"))
    eval_file = args.eval_file or config.get("EVAL_FILE")
    if args.dry_run:
        print(f"[dry-run] {paths.rel(engine)} で {args.csa} を{args.start_ply}手目から指し直す")
        return proc.OK
    for path, what in ((engine, "エンジン"), (eval_file, "評価関数"), (args.csa, "棋譜")):
        if not Path(path).is_file():
            raise proc.Fail(f"{what}がない: {path}")
    return replay_tool.replay(
        engine, eval_file, Path(args.csa),
        player=args.player, threads=args.threads, start_ply=args.start_ply, hash_mb=256,
        initial_ms=args.initial * 1000, inc_ms=args.inc * 1000,
    )  # fmt: skip
