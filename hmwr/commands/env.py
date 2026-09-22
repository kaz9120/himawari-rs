"""マシンの設定と測定の既定条件を表示する。"""

from __future__ import annotations

import argparse

from .. import config, paths, proc


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("env", help="（旧）設定を表示する。hmwr status --config と同じ")
    p.set_defaults(func=show)


def show(args: argparse.Namespace) -> int:
    """測る前に条件を確かめるための表示。`hmwr status` に吸収した（ADR-0220）。
    1版は別名として残す。"""
    print("hmwr env は hmwr status --config に変わった。この別名は次の版で消える", flush=True)
    rows = config.summary()
    width = max(paths.display_width(k) for k, _ in rows) + 2
    for key, value in rows:
        print(f"{paths.pad(key, width)}{value}")
    return proc.OK
