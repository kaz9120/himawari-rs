"""継続的インテグレーションの結果を待つ。

待機ループを直接ツールへ渡すと、複合コマンドのため権限の許可規則で拾えず、
そのたびに確認を求められる（ADR-0098）。読むだけの操作としてここに置く。
**マージはしない。破壊的な操作を読み取り専用のコマンドへ混ぜない。**
"""

from __future__ import annotations

import argparse
import time

from .. import proc

# 待ちきれずに戻る上限。CIが数十分で終わらないなら別の問題がある
MAX_WAIT = 60 * 60


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("ci", help="CIの結果を見る")
    ss = p.add_subparsers(dest="sub", metavar="<操作>")

    t = ss.add_parser("wait", help="CIが確定するまで待つ")
    t.add_argument("pr", metavar="PR番号")
    t.add_argument("--interval", type=int, default=20, metavar="秒", help="確認の間隔")
    t.set_defaults(func=wait)


def wait(args: argparse.Namespace) -> int:
    """成功か失敗が確定するまで待つ。終了コードで結果を返す。"""
    if args.dry_run:
        print(f"[dry-run] PR#{args.pr} のCIを待つ")
        return proc.OK

    waited = 0
    while waited < MAX_WAIT:
        out = proc.capture(["gh", "pr", "checks", str(args.pr)])
        state = _state(out)
        if state is not None:
            print(f"PR#{args.pr} CI {'pass' if state else 'fail'}")
            return proc.OK if state else proc.JUDGE
        time.sleep(args.interval)
        waited += args.interval

    raise proc.Fail(f"PR#{args.pr} のCIが{MAX_WAIT}秒で確定しなかった")


def _state(output: str) -> bool | None:
    """出力から成否を読む。まだ動いているならNoneを返す。"""
    rows = [line for line in output.splitlines() if line.strip()]
    if not rows:
        return None
    if any("\tpending" in line or "\tin_progress" in line for line in rows):
        return None
    if any("\tfail" in line for line in rows):
        return False
    if all("\tpass" in line or "\tskipping" in line for line in rows):
        return True
    return None
