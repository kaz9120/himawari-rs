"""対局ゲート（SPRT）の入口。`match` の別名として残している（ADR-0208）。

対局の実体は `match.py` にある。ここが持つのは2つの段取りだけである。
`sprt run` はペアの作成・機能検証・起動を順に行い、`sprt net` はビルドを
固定して評価関数だけを差し替える。どちらも `match run` の対局者の
組み立て方の違いで、移行が済んだら消す。
"""

from __future__ import annotations

import argparse
from pathlib import Path

from .. import config, paths, proc
from . import build, match
from .match import Player, Spec, files, settings

def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("sprt", help="対局で棋力を検定する")
    ss = p.add_subparsers(dest="sub", metavar="<操作>")

    t = ss.add_parser(
        "run",
        help="ペアを作り、機能検証を通してから起動する",
        description="ビルド・機能検証・起動を順に行う。判定が出るまで走り、"
        "落ちても棋譜から再開する。すでに判定済みなら結果を返して終わる。",
    )
    t.add_argument("name", help="実験名")
    t.add_argument("--baseline", metavar="REF", help="比較元のref（既定 origin/main）")
    t.add_argument(
        "--noninferiority",
        action="store_true",
        help="非劣性で測る（elo0=-5、elo1=0）",
    )
    t.add_argument("--tc", metavar="持ち時間", help="例 60+0.6（既定 10+0.1）")
    t.add_argument(
        "--set",
        action="append",
        metavar="KEY=VALUE",
        help="測定条件を直接渡す（繰り返し可）",
    )
    t.add_argument(
        "--max-pairs",
        type=int,
        metavar="N",
        help="このペア数で打ち切る。H1だけを採択するゲート用途の見送り上限",
    )
    t.add_argument(
        "--no-verify",
        dest="verify",
        action="store_false",
        help="機能検証を飛ばす。終盤にしか出ない機能を測るときだけ使う",
    )
    t.add_argument(
        "--foreground",
        action="store_true",
        help="切り離さずその場で走らせる。判定まで戻らない",
    )
    # 切り離した子プロセスが自分を呼び直すための内部フラグ
    t.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    t.set_defaults(func=run, verify=True)

    t = ss.add_parser(
        "net",
        help="ビルドを固定し、評価関数だけを差し替えて測る",
        description="同じビルドで対局し、評価関数を片側ずつ指定する。"
        "ネットとビルドの次元は揃える。",
    )
    t.add_argument("base", metavar="baselineネット")
    t.add_argument("cand", metavar="candidateネット")
    t.add_argument("name", help="実験名")
    t.add_argument("--bin", metavar="パス", help="対局に使うビルド")
    t.add_argument(
        "--cand-bin",
        metavar="パス",
        help="candidate側だけ別のビルドを使う。入力特徴の違う構成の比較用",
    )
    t.add_argument("--noninferiority", action="store_true", help="非劣性で測る")
    t.add_argument("--tc", metavar="持ち時間", help="例 60+0.6")
    t.add_argument("--set", action="append", metavar="KEY=VALUE", help="測定条件")
    t.add_argument(
        "--max-pairs",
        type=int,
        metavar="N",
        help="このペア数で打ち切る。H1だけを採択するゲート用途の見送り上限",
    )
    t.add_argument("--foreground", action="store_true", help="切り離さず走らせる")
    t.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    t.set_defaults(func=run_net)

    match.add_show_wait(ss)


# --- 起動 --------------------------------------------------------------


def run(args: argparse.Namespace) -> int:
    """ビルド・機能検証・起動を順に行う。

    3つを別々に叩けると順番を飛ばせてしまう。機能検証を飛ばすと、探索に
    影響のない変更へ対局リソースを払うことになる（ADR-0074）。ここで
    順番を固定し、飛ばすには明示を求める。
    """
    f = files(args.name)
    spec = Spec(
        args.name, Player(str(f["base"])), Player(str(f["cand"])), env=settings(args)
    )

    if args.worker:
        # 切り離された子。判定が出るまで回す
        return match.until_decision(spec, dry_run=False)

    if match.decided(args.name):
        return proc.OK

    code = build.make_pair(
        args.name,
        baseline=args.baseline or "origin/main",
        dry_run=args.dry_run,
    )
    if code == proc.JUDGE:
        # 2本が同一だった。対局しても差は出ない
        return proc.JUDGE

    if args.verify and not _verified(args):
        return proc.JUDGE

    return match.start(args, spec, ["sprt", "run", args.name])


def run_net(args: argparse.Namespace) -> int:
    """評価関数だけを差し替えて測る。ビルドは同じものを両側に使う。"""
    engine = args.bin or _default_bin(args.name)
    spec = Spec(
        args.name,
        Player(engine, match.abs_path(args.base)),
        Player(args.cand_bin or engine, match.abs_path(args.cand)),
        env=settings(args),
    )

    if args.worker:
        return match.until_decision(spec, dry_run=False)

    if match.decided(args.name):
        return proc.OK

    for net in (args.base, args.cand):
        if not Path(net).is_file() and not args.dry_run:
            raise proc.Fail(f"ネットがない: {net}")

    rest = ["sprt", "net", args.base, args.cand, args.name]
    if args.cand_bin:
        rest += ["--cand-bin", args.cand_bin]
    if args.bin:
        rest += ["--bin", args.bin]
    return match.start(args, spec, rest)


def _verified(args: argparse.Namespace) -> bool:
    """機能検証を通す。全局面で一致したらFalseを返し、起動を止める。"""
    f = files(args.name)
    code = proc.run(
        proc.cargo_tool("verify", [str(f["base"]), str(f["cand"])]),
        dry_run=args.dry_run,
        env=config.measure_env(),
        log=paths.log("verify", args.name),
        allowed=(proc.OK, proc.JUDGE),
    )
    if code != proc.JUDGE:
        return True
    print()
    print("全局面でノード数が一致した。この変更は探索に影響していない。")
    print("対局にかけても中立にしかならないので起動しない。")
    print("終盤にしか出ない機能なら、終盤局面を別に用意して測り直す。")
    print("それでも走らせるなら --no-verify を付ける。")
    return False


def _default_bin(name: str) -> str:
    """対局に使うビルド。実験ごとに固定したいので data/bin を先に見る。"""
    fixed = paths.BIN / f"base-{name}"
    if fixed.is_file():
        return str(fixed)
    return str(paths.REPO / "target" / "release" / "himawari")
