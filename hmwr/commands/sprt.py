"""対局ゲート（SPRT）の起動・確認・待機。

判定が出るまで走らせ、完了は `data/sprt/<名前>.result` の有無で決まる。
プロセスの生死やセッションの継続に依存しない設計は ADR-0175 にある。
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from .. import config, paths, proc


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
        "--no-verify",
        dest="verify",
        action="store_false",
        help="機能検証を飛ばす。終盤にしか出ない機能を測るときだけ使う",
    )
    t.set_defaults(func=run, verify=True)

    t = ss.add_parser("show", help="途中経過や結果を出す。名前を省くと一覧")
    t.add_argument("name", nargs="?", help="実験名")
    t.add_argument("--all", action="store_true", help="完了した走行も並べる")
    t.set_defaults(func=show)

    t = ss.add_parser("wait", help="判定が出るまで待つ")
    t.add_argument("name", help="実験名")
    t.add_argument("--interval", type=int, default=60, metavar="秒", help="確認の間隔")
    t.set_defaults(func=wait)


def files(name: str) -> dict[str, Path]:
    """この名前で決まる置き場をまとめて返す。"""
    paths.check_name(name)
    return {
        "base": paths.BIN / f"base-{name}",
        "cand": paths.BIN / f"cand-{name}",
        "jsonl": paths.SPRT / f"{name}.jsonl",
        "result": paths.SPRT / f"{name}.result",
        "log": paths.log("sprt", name),
    }


def run(args: argparse.Namespace) -> int:
    """ビルド・機能検証・起動を順に行う。

    3つを別々に叩けると順番を飛ばせてしまう。機能検証を飛ばすと、探索に
    影響のない変更へ対局リソースを払うことになる（ADR-0074）。ここで
    順番を固定し、飛ばすには明示を求める。
    """
    f = files(args.name)

    if f["result"].is_file():
        print(f"判定済み: {paths.rel(f['result'])}")
        print(f["result"].read_text(encoding="utf-8"), end="")
        return proc.OK

    build = [proc.script("build-pair.sh"), args.name]
    if args.baseline:
        build.append(args.baseline)
    proc.run(build, dry_run=args.dry_run)

    if args.verify:
        code = proc.run(
            proc.cargo_tool("verify", [str(f["base"]), str(f["cand"])]),
            dry_run=args.dry_run,
            env=config.measure_env(),
            log=paths.log("verify", args.name),
            allowed=(proc.OK, proc.JUDGE),
        )
        if code == proc.JUDGE:
            print()
            print("全局面でノード数が一致した。この変更は探索に影響していない。")
            print("対局にかけても中立にしかならないので起動しない。")
            print("終盤にしか出ない機能なら、終盤局面を別に用意して測り直す。")
            print("それでも走らせるなら --no-verify を付ける。")
            return proc.JUDGE

    settings: list[str] = []
    if args.noninferiority:
        settings += ["SPRT_ELO0=-5", "SPRT_ELO1=0"]
    if args.tc:
        settings.append(f"SPRT_TC={args.tc}")
    for item in args.set or []:
        if "=" not in item:
            raise proc.Fail(f"--set はKEY=VALUEで書く: {item}", proc.USAGE)
        settings.append(item)

    proc.run(
        [
            proc.script("sprt-detach.py"),
            str(f["base"]),
            str(f["cand"]),
            args.name,
            *settings,
        ],
        dry_run=args.dry_run,
    )
    if not args.dry_run:
        print()
        print(f"経過: hmwr sprt show {args.name}")
        print(f"完了: {paths.rel(f['result'])} の出現を見る")
    return proc.OK


def show(args: argparse.Namespace) -> int:
    """途中経過を出す。名前を省くと走行を新しい順に並べる。"""
    if args.name:
        f = files(args.name)
        if not f["log"].is_file():
            raise proc.Fail(f"ログがない: {paths.rel(f['log'])}")
        return proc.run(
            [sys.executable, proc.script("sprt-summary.py"), str(f["log"]), args.name],
            dry_run=args.dry_run,
            allowed=(0, 1, 2),
        )
    return _list(args.all)


def _list(show_all: bool) -> int:
    """走行の一覧。完了は .result の有無で決まる（ADR-0175）。"""
    if not paths.SPRT.is_dir():
        print("走行はまだない")
        return proc.OK

    rows = []
    for jsonl in paths.SPRT.glob("*.jsonl"):
        name = jsonl.stem
        done = (paths.SPRT / f"{name}.result").is_file()
        games = sum(1 for _ in jsonl.open("rb"))
        rows.append((jsonl.stat().st_mtime, name, "完了" if done else "未完了", games))
    if not rows:
        print("走行はまだない")
        return proc.OK

    rows.sort(reverse=True)
    shown = rows if show_all else rows[:10]
    width = max(paths.display_width(r[1]) for r in shown) + 2
    print(paths.pad("名前", width) + paths.pad("状態", 8) + "局数")
    for _, name, state, games in shown:
        print(paths.pad(name, width) + paths.pad(state, 8) + str(games))
    if not show_all and len(rows) > len(shown):
        print(f"\n新しい順に{len(shown)}件。全{len(rows)}件を見るには --all")
    print("\n「未完了」は結果ファイルがない状態を指す。走っているとは限らない。")
    return proc.OK


def wait(args: argparse.Namespace) -> int:
    """判定が出るまで待つ。終了コードで判定を返す。"""
    f = files(args.name)
    return proc.run(
        [proc.script("watch-sprt.sh"), str(f["log"]), str(args.interval)],
        dry_run=args.dry_run,
        allowed=(0, 1, 2),
    )
