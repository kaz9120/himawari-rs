"""定跡の生成・統計・配布。"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

from .. import config, paths, proc
from .. import release as release_mod
from ..tools import floodgate

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
    t.add_argument(
        "--games",
        nargs="+",
        metavar="ディレクトリ",
        help=f"棋譜の置き場（既定 data/raw/floodgate/ の{floodgate.FIRST_YEAR}年から今年まで）",
    )
    t.add_argument("--out", metavar="DB", help="定跡ファイル（既定 data/book/main.db）")
    t.add_argument("--depth", type=int, default=28, metavar="N", help="探索の深さ")
    t.add_argument("--max-positions", type=int, default=50, metavar="N", help="追加の上限")
    t.set_defaults(func=seed)

    t = ss.add_parser(
        "seeds",
        help="floodgateの全対局から局面の出現率を数え、mineの種にする",
        description="data/book/<名前>.seeds へ書く。両対局者のレートが下限以上の対局だけを数える。",
    )
    t.add_argument("name", metavar="名前")
    t.add_argument("--years", required=True, metavar="A-B", help="対象の年。例 2025-2026")
    t.add_argument("--min-rate", type=float, default=3500, metavar="R", help="両対局者のレートの下限（既定 3500）")
    t.add_argument("--max-ply", type=int, default=40, metavar="N", help="数える手数の上限（既定 40）")
    t.add_argument("--min-count", type=int, default=2, metavar="N", help="この回数未満の局面は書かない（既定 2）")
    t.add_argument("--min-moves", type=int, default=60, metavar="N", help="これより短い対局は数えない（既定 60）")
    t.add_argument("--tag", default="2026-04-10", metavar="タグ", help="アーカイブのミラーのタグ")
    t.add_argument("--jobs", type=int, default=8, metavar="N", help="並列数")
    t.set_defaults(func=seeds)

    t = ss.add_parser(
        "openings",
        help="種から浅い手数の開始局面集を作る",
        description="種のうち指定の手数の局面を、出現率に比例して重複ありで引き、"
        "openings/<出力名>.txt へ書く。定跡の効果を測る対局に使う。"
        "既定のSPRTは24手目から始まるので、定跡が引かれない。",
    )
    t.add_argument("name", metavar="出力名")
    t.add_argument("--seeds", required=True, metavar="名前", help="data/book/<名前>.seeds")
    t.add_argument("--ply", type=int, default=6, metavar="N", help="開始局面の手数（既定 6）")
    t.add_argument("--count", type=int, default=20000, metavar="N", help="局面数（既定 20000）")
    t.add_argument("--seed", type=int, default=1, metavar="N", help="乱数の種（既定 1）")
    t.set_defaults(func=openings)

    t = ss.add_parser(
        "mine",
        help="到達確率の高い局面から順に定跡を掘る",
        description="data/book/<名前>.db へ書き、切り離して走る。同じコマンドで続きから掘る。"
        "止めるには data/book/<名前>.db.stop を置く。",
    )
    t.add_argument("name", metavar="名前")
    t.add_argument("--seeds", metavar="名前", help="種。data/book/<名前>.seeds（hmwr book seeds の出力）")
    t.add_argument("--eval", metavar="ネット", help="評価関数。data/nets/<名前>.hmwr（既定 現行）")
    t.add_argument("--depth", type=int, default=30, metavar="N", help="探索の深さ（既定 30）")
    t.add_argument(
        "--nodes", type=int, default=300_000_000, metavar="N",
        help="1局面のノード上限（既定 3億、平均の約6倍。0は無制限）",
    )
    t.add_argument("--width", type=int, default=3, metavar="N", help="記録する候補手数（既定 3）")
    t.add_argument("--threads", type=int, default=10, metavar="N", help="探索スレッド数（既定 10）")
    t.add_argument("--hash", type=int, default=4096, metavar="MB", help="置換表（既定 4096）")
    t.add_argument("--max-ply", type=int, default=40, metavar="N", help="掘る手数の上限（既定 40）")
    t.add_argument("--max-positions", type=int, default=10000, metavar="N", help="定跡全体の局面数の上限")
    t.add_argument("--margin", type=int, default=100, metavar="CP", help="最善手との差の上限（既定 100）")
    t.add_argument("--temp", type=float, default=40, metavar="CP", help="手の分布の温度（既定 40）")
    t.add_argument("--min-prob", type=float, default=1e-4, metavar="P", help="到達確率の下限（既定 0.0001）")
    t.add_argument("--foreground", action="store_true", help="切り離さず走らせる")
    t.set_defaults(func=mine)

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
    binary = paths.release_bin("book")
    if not binary.is_file():
        raise proc.Fail(f"{paths.rel(binary)} がない。先に cargo build --release を実行する")
    return str(binary)


def seed(args: argparse.Namespace) -> int:
    db = _db_path(args)
    games = args.games or [str(d) for d in floodgate.games_dirs(paths.RAW / "floodgate")]
    db.parent.mkdir(parents=True, exist_ok=True)
    argv = [
        _tool() if not args.dry_run else "target/release/book", "seed",
        "--games", *games,
        "--out", str(db),
        "--eval", config.get("EVAL_FILE", "（未設定）"),
        "--depth", str(args.depth),
        "--max-positions", str(args.max_positions),
    ]
    return proc.run(argv, dry_run=args.dry_run, log=paths.log("book", "seed"))


def _years(spec: str) -> list[int]:
    a, _, b = spec.partition("-")
    try:
        lo, hi = int(a), int(b or a)
    except ValueError:
        raise proc.Fail(f"--years は A-B の形で渡す: {spec}") from None
    if lo > hi:
        raise proc.Fail(f"--years の順が逆: {spec}")
    return list(range(lo, hi + 1))


def seeds(args: argparse.Namespace) -> int:
    paths.check_name(args.name)
    out = paths.BOOK / f"{args.name}.seeds"
    years = _years(args.years)
    if args.dry_run:
        print(f"[dry-run] {years} の棋譜から {paths.rel(out)} を書く"
              f"（レート{args.min_rate}以上、{args.max_ply}手まで、{args.min_count}回以上）")
        return proc.OK
    from ..tools import book_seeds

    out.parent.mkdir(parents=True, exist_ok=True)
    try:
        got = book_seeds.count(
            years, paths.RAW / "floodgate_archive", out, tag=args.tag, max_ply=args.max_ply,
            min_rate=args.min_rate, min_moves=args.min_moves, min_count=args.min_count,
            jobs=args.jobs,
        )
    except ValueError as e:
        raise proc.Fail(str(e)) from None
    print(f"{got['games']:,}局から種{got['seeds']:,}局面を {paths.rel(out)} へ書いた")
    return proc.OK


def openings(args: argparse.Namespace) -> int:
    paths.check_name(args.name)
    src = paths.BOOK / f"{args.seeds}.seeds"
    out = paths.REPO / "openings" / f"{args.name}.txt"
    if args.dry_run:
        print(f"[dry-run] {paths.rel(src)} の{args.ply}手目の局面から{args.count}局面を {paths.rel(out)} へ書く")
        return proc.OK
    if not src.is_file():
        raise proc.Fail(f"種がない: {paths.rel(src)}")
    rows = pick_openings(src.read_text(encoding="utf-8"), args.ply, args.count, args.seed)
    if not rows:
        raise proc.Fail(f"{args.ply}手目の局面が種に無い")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text("".join(f"{r}\n" for r in rows), encoding="utf-8")
    print(f"{len(rows):,}局面（異なり{len(set(rows)):,}）を {paths.rel(out)} へ書いた")
    return proc.OK


def pick_openings(text: str, ply: int, count: int, seed: int) -> list[str]:
    """種のうち手数plyの局面を、出現率に比例して重複ありでcount個引く。"""
    import random

    cands, weights = [], []
    for line in text.splitlines():
        prob, _, sfen = line.partition(" ")
        f = sfen.split()
        if len(f) == 5 and f[0] == "sfen" and f[4] == str(ply + 1):
            cands.append(sfen)
            weights.append(float(prob))
    if not cands:
        return []
    return random.Random(seed).choices(cands, weights=weights, k=count)


def mine(args: argparse.Namespace) -> int:
    paths.check_name(args.name)
    out = paths.BOOK / f"{args.name}.db"
    net = paths.NETS / f"{args.eval}.hmwr" if args.eval else Path(config.get("EVAL_FILE", ""))
    argv = [
        _tool() if not args.dry_run else "target/release/book", "mine",
        "--out", str(out),
        "--eval", str(net),
        "--depth", str(args.depth),
        "--nodes", str(args.nodes),
        "--width", str(args.width),
        "--threads", str(args.threads),
        "--hash", str(args.hash),
        "--max-ply", str(args.max_ply),
        "--max-positions", str(args.max_positions),
        "--margin", str(args.margin),
        "--temp", str(args.temp),
        "--min-prob", str(args.min_prob),
    ]
    if args.seeds:
        seeds_file = paths.BOOK / f"{args.seeds}.seeds"
        if not seeds_file.is_file() and not args.dry_run:
            raise proc.Fail(f"種がない: {paths.rel(seeds_file)}（先に hmwr book seeds {args.seeds}）")
        argv += ["--seeds", str(seeds_file)]
    if not net.is_file() and not args.dry_run:
        raise proc.Fail(f"評価関数がない: {net}")
    log = paths.log("book", args.name)
    if args.foreground or args.dry_run:
        return proc.run(argv, dry_run=args.dry_run, log=log)
    log.parent.mkdir(parents=True, exist_ok=True)
    with open(log, "ab") as fh, open(os.devnull, "rb") as devnull:
        child = subprocess.Popen(
            argv, cwd=str(paths.REPO), stdout=fh, stderr=fh, stdin=devnull,
            start_new_session=True,
        )
    print(f"起動した pid={child.pid}（新しいセッション）")
    print(f"経過: tail -f {paths.rel(log)}")
    print(f"停止: touch {paths.rel(out)}.stop")
    return proc.OK


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
    # アセットは固定名で配る。手元の名前はノートに残す
    asset_name = config.BOOK_ASSET

    notes = [
        "## 定跡", "",
        "| 項目 | 値 |", "|---|---|",
        f"| アセット | `{asset_name}` |",
        f"| 手元の名前 | `{db.name}` |",
        "| 形式 | やねうら王の定跡（`#YANEURAOU-DB2016`） |",
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
        f"gh release download {tag} -p '{asset_name}' -D data/book/",
        "```", "",
        "USIオプション `BookFile` にパスを指定する。既定は定跡なし。",
        "実行ファイルの隣に置いただけでは読まない。",
        "`BookDepth` で定跡を引く手数の上限を決める（既定40）。", "",
        "生成は非決定的である。同じコマンドでも内容が変わるため、",
        "再現ではなくこの成果物を使う。",
    ]

    print(f"タグ    : {tag}")
    print(f"アセット: {asset_name}（{positions}局面、{size}、手元の名前 {db.name}）")
    with tempfile.TemporaryDirectory() as tmp:
        asset = Path(tmp) / asset_name
        shutil.copy2(db, asset)
        return release_mod.create(
            tag,
            f"{tag}: {asset_name} ({positions}局面)",
            "\n".join(notes) + "\n",
            [asset],
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
