"""コマンド行の解析と振り分け。

各領域のモジュールが自分のサブコマンドを登録するので、ここは並べるだけで
領域ごとの事情を知らない。
"""

from __future__ import annotations

import argparse
import sys

from . import paths, proc
from .commands import MODULES

DESCRIPTION = """\
himawari-rs の開発コマンド。

ビルド・測定・学習・データ処理をここから行う。オプションはフラグで渡す。
ログの置き場は data/logs/<領域>-<名前>.log へ自動で決まる。
"""

EPILOG = """\
どのコマンドでも --dry-run が使える。走るはずのコマンドを表示して終わるので、
時間のかかる操作や外から見える操作は先に下見できる。

終了コード: 0=成功、1=判定結果、2=引数エラー、3=実行時エラー。
"""


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="hmwr",
        description=DESCRIPTION,
        epilog=EPILOG,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="実行せず、走るはずのコマンドを表示する",
    )
    sub = parser.add_subparsers(dest="command", metavar="<コマンド>")
    for module in MODULES:
        module.add_parser(sub)
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    if not getattr(args, "func", None):
        # サブコマンドが要る領域で操作を省いた場合は、その領域のヘルプを出す
        target = _subparser_for(parser, getattr(args, "command", None)) or parser
        target.print_help()
        return proc.USAGE

    try:
        return args.func(args)
    except paths.BadName as e:
        print(f"エラー: {e}", file=sys.stderr)
        return proc.USAGE
    except proc.Fail as e:
        print(f"エラー: {e}", file=sys.stderr)
        return e.code
    except KeyboardInterrupt:
        print("\n中断した", file=sys.stderr)
        return proc.RUNTIME


def _subparser_for(
    parser: argparse.ArgumentParser, name: str | None
) -> argparse.ArgumentParser | None:
    if not name:
        return None
    for action in parser._actions:
        choices = getattr(action, "choices", None)
        if isinstance(choices, dict) and name in choices:
            return choices[name]
    return None
