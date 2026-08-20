"""定跡の生成・統計・配布。"""

from __future__ import annotations

import argparse
from pathlib import Path

from .. import config, paths, proc
from .. import release as release_mod

DB_HEADER = "#YANEURAOU-DB"


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("book", help="定跡を作る・配る")
    ss = p.add_subparsers(dest="sub", metavar="<操作>")

    t = ss.add_parser(
        "seed",
        help="実戦の棋譜から定跡へ局面を足す",
        description="1局面あたり深さ28で約34秒かかる。冪等なので、"
        "何度回しても取得済みの局面は増えない。",
    )
    t.add_argument("--games", metavar="ディレクトリ", help="棋譜の置き場")
    t.add_argument("--out", metavar="DB", help="定跡ファイル（既定 data/book/main.db）")
    t.add_argument("--depth", type=int, default=28, metavar="N", help="探索の深さ")
    t.add_argument("--max-positions", type=int, default=50, metavar="N", help="追加の上限")
    t.set_defaults(func=seed)

    t = ss.add_parser("stats", help="定跡の網羅率を出す")
    t.add_argument("--out", metavar="DB", help="定跡ファイル（既定 data/book/main.db）")
    t.set_defaults(func=stats)

    t = ss.add_parser(
        "release",
        help="定跡をGitHub Releaseで配る",
        description="生成は非決定的なので、コマンドを残しても同じものは"
        "再現できない。成果物そのものを保存する。"
        "既定では作らない。実際に作るには --apply を付ける。",
    )
    t.add_argument("file", metavar="DB")
    t.add_argument("version", type=int, metavar="番号")
    t.add_argument("--gen-log", metavar="ログ", help="生成時のログ。条件をノートへ載せる")
    t.add_argument("--notes", metavar="文", help="リリースノートへの追記")
    t.add_argument("--apply", action="store_true", help="実際に作る")
    t.set_defaults(func=release)


def _db_path(args: argparse.Namespace) -> Path:
    return Path(args.out) if args.out else paths.BOOK / "main.db"


def _tool() -> str:
    binary = paths.REPO / "target" / "release" / "book"
    if not binary.is_file():
        raise proc.Fail(f"{paths.rel(binary)} がない。先に cargo build --release を実行する")
    return str(binary)


def seed(args: argparse.Namespace) -> int:
    db = _db_path(args)
    games = args.games or str(paths.RAW / "floodgate" / "2026")
    db.parent.mkdir(parents=True, exist_ok=True)
    argv = [
        _tool() if not args.dry_run else "target/release/book", "seed",
        "--games", games,
        "--out", str(db),
        "--eval", config.get("EVAL_FILE", "（未設定）"),
        "--depth", str(args.depth),
        "--max-positions", str(args.max_positions),
    ]
    return proc.run(argv, dry_run=args.dry_run, log=paths.log("book", "seed"))


def stats(args: argparse.Namespace) -> int:
    argv = [
        _tool() if not args.dry_run else "target/release/book",
        "stats",
        "--out",
        str(_db_path(args)),
    ]
    return proc.run(argv, dry_run=args.dry_run)


def release(args: argparse.Namespace) -> int:
    db = Path(args.file)
    if not db.is_file():
        raise proc.Fail(f"定跡ファイルがない: {db}")
    release_mod.check_version(args.version)

    with db.open(encoding="utf-8", errors="replace") as fh:
        if not fh.readline().startswith(DB_HEADER):
            raise proc.Fail(f"定跡の形式が違う: {paths.rel(db)}")

    tag = f"book-v{args.version}"
    release_mod.check_prereqs(tag, dry_run=args.dry_run)

    positions = sum(
        1
        for line in db.open(encoding="utf-8", errors="replace")
        if line.startswith("sfen")
    )
    size = release_mod.file_size(db)
    gen = _read_gen_log(args.gen_log)

    notes = [
        "## 定跡", "",
        "| 項目 | 値 |", "|---|---|",
        f"| アセット | `{db.name}` |",
        f"| 局面数 | {positions} |",
        f"| サイズ | {size} |",
    ]
    if gen.get("BookGen"):
        notes += ["", "## 生成条件", "", "```", gen["BookGen"], "```"]
    if gen.get("EvalFile"):
        notes += ["", "## 生成に使った評価関数", "", "```", gen["EvalFile"], "```"]
    if args.notes:
        notes += ["", "## 補足", "", args.notes]
    notes += [
        "", "## 使い方", "", "```",
        f"gh release download {tag} -D data/book/",
        "```", "",
        "USIオプション `BookFile` にパスを指定する。既定は定跡なし。",
        "`BookDepth` で定跡を引く手数の上限を決める（既定24）。", "",
        "生成は非決定的である。同じコマンドでも内容が変わるため、",
        "再現ではなくこの成果物を使う。",
    ]

    print(f"タグ    : {tag}")
    print(f"アセット: {db.name}（{positions}局面、{size}）")
    return release_mod.create(
        tag,
        f"{tag}: {db.name} ({positions}局面)",
        "\n".join(notes) + "\n",
        [db],
        apply=args.apply,
        dry_run=args.dry_run,
    )


def _read_gen_log(path: str | None) -> dict[str, str]:
    """生成ログから条件を拾う。無ければ空で返す。"""
    if not path or not Path(path).is_file():
        return {}
    found: dict[str, str] = {}
    for line in Path(path).read_text(encoding="utf-8", errors="replace").splitlines():
        for key in ("BookGen:", "EvalFile:"):
            if line.startswith(key) and key.rstrip(":") not in found:
                found[key.rstrip(":")] = line[len(key) :].strip()
    return found
