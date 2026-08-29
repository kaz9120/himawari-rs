"""教師データの取得と前処理。

公開データセットを取り、学習用と検証用のpsvを作る。開発機を移すときは
これで再現できる。hao_depth9は固定の381ファイル、その他のデータセットは
HuggingFaceのAPIでファイル一覧とサイズを引いて取得・検査する。
"""

from __future__ import annotations

import argparse
import json
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from .. import config, paths, proc

BASE_URL = "https://huggingface.co/datasets/nodchip/shogi_hao_depth9/resolve/main"
PREFIX = "kifu.tag=train.depth=9.num_positions=1000000000"
START_TIMES = ("1695340981", "1695606850", "1695872823")
INDEXES = tuple(f"{i:03d}" for i in range(127))

# 取得できるデータセット。hao以外は生psv（.bin）をAPIの一覧で取る
DATASETS = {
    "hao": {"repo": "nodchip/shogi_hao_depth9", "dir": "hao_depth9"},
    "tanuki2024": {
        "repo": "nodchip/tanuki-.nnue-pytorch-2024-07-30.1",
        "dir": "tanuki2024",
    },
    "entering-king": {
        "repo": "nodchip/shogi_suisho5_depth9_entering_king",
        "dir": "entering_king",
    },
}

# 検証データの供給元。学習データからは除く
VALID_START_TIME = "1695340981"
VALID_INDEX = "023"
VALID_COUNT = 200000

# 途中で切れた取得を見分ける下限
MIN_SIZE = 100 * 1024 * 1024
DEFAULT_JOBS = 4


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("data", help="教師データを取得・加工する")
    ss = p.add_subparsers(dest="sub", metavar="<操作>")

    t = ss.add_parser(
        "fetch",
        help="公開データセットを取得してpsvを作る",
        description="生データ116GBと加工後120GBで、空きが236GB要る。"
        "取得は再実行できる。妥当なサイズのファイルは飛ばす。",
    )
    t.add_argument(
        "stage",
        nargs="?",
        default="all",
        choices=["download", "verify", "prepare", "all"],
        metavar="<段>",
        help="download / verify / prepare / all（既定 all）",
    )
    t.add_argument("--jobs", type=int, default=DEFAULT_JOBS, metavar="N", help="並列数")
    t.add_argument("--raw-dir", metavar="パス", help="生データの置き場")
    t.add_argument("--train-dir", metavar="パス", help="加工後の置き場")
    t.add_argument(
        "--dataset",
        default="hao",
        choices=sorted(DATASETS),
        help="取得するデータセット（既定 hao）。prepareはhaoだけが持つ",
    )
    t.add_argument(
        "--limit-files",
        type=int,
        metavar="N",
        help="名前順の先頭Nファイルだけ取得する。スラブ処理と部分検証用",
    )
    t.set_defaults(func=fetch)

    t = ss.add_parser(
        "quiet",
        help="教師局面を静止局面へ置き換える",
        description="評価関数が探索中に見るのは静止局面だが、公開データは"
        "取り合いの途中の局面へ収束後の評価値を付けて配られている。"
        "そのずれを消す。29.9億で7.0時間、3億で50分かかる。"
        "**学習データを静止化したら、検証集合も同じ設定で静止化する。**",
    )
    t.add_argument("input", metavar="入力psv")
    t.add_argument("output", metavar="出力psv")
    t.add_argument("--name", metavar="名前", help="ログ名。省くと出力名から作る")
    t.add_argument("--max-plies", type=int, default=1, metavar="N", help="進める手数の上限")
    t.add_argument("--limit", type=int, metavar="N", help="先頭のこの件数だけ処理する")
    t.add_argument("--eval-file", metavar="パス", help="評価関数")
    t.set_defaults(func=quiet)


# --- fetch -------------------------------------------------------------


def _raw_dir(args: argparse.Namespace) -> Path:
    if args.raw_dir:
        return Path(args.raw_dir)
    return paths.RAW / DATASETS[args.dataset]["dir"]


def _train_dir(args: argparse.Namespace) -> Path:
    return Path(args.train_dir) if args.train_dir else paths.TRAIN


def file_name(start_time: str, index: str) -> str:
    return f"{PREFIX}.start_time={start_time}.thread_index={index}.bin"


def all_names() -> list[str]:
    return [file_name(st, i) for st in START_TIMES for i in INDEXES]


def _manifest(dataset: str, limit: int | None) -> list[tuple[str, int | None]]:
    """(ファイル名, 期待サイズ)の一覧。haoは固定、他はAPIから引く。

    期待サイズNoneは「下限MIN_SIZEだけ検査する」を意味する。
    """
    if dataset == "hao":
        entries: list[tuple[str, int | None]] = [(n, None) for n in all_names()]
    else:
        repo = DATASETS[dataset]["repo"]
        url = f"https://huggingface.co/api/datasets/{repo}/tree/main"
        with urllib.request.urlopen(url, timeout=60) as r:
            tree = json.load(r)
        entries = sorted(
            (f["path"], (f.get("lfs") or {}).get("size") or f["size"])
            for f in tree
            if f["path"].endswith(".bin")
        )
        if not entries:
            raise proc.Fail(f"{repo} に.binファイルがない")
    return entries[:limit] if limit else entries


def fetch(args: argparse.Namespace) -> int:
    raw, train = _raw_dir(args), _train_dir(args)
    stages = ("download", "verify", "prepare") if args.stage == "all" else (args.stage,)
    if "prepare" in stages and args.dataset != "hao":
        if args.stage == "all":
            stages = ("download", "verify")
        else:
            raise proc.Fail(
                "prepareはhao専用。他のデータセットの前処理は着手時のADRの手順で行う"
            )
    manifest = _manifest(args.dataset, args.limit_files)
    for stage in stages:
        if stage == "download":
            _download(args.dataset, manifest, raw, args.jobs, dry_run=args.dry_run)
        elif stage == "verify":
            if not _verify(manifest, raw) and not args.dry_run:
                raise proc.Fail("欠落またはサイズ不足のファイルがある")
        else:
            _prepare(raw, train, dry_run=args.dry_run)
    return proc.OK


def _download(
    dataset: str,
    manifest: list[tuple[str, int | None]],
    raw: Path,
    jobs: int,
    *,
    dry_run: bool,
) -> None:
    total = sum(s for _, s in manifest if s)
    size_note = f"、計{total / 2**30:.1f}GiB" if total else ""
    print(f"対象 {len(manifest)} ファイル{size_note}、並列 {jobs}、置き場 {paths.rel(raw)}")
    if dry_run:
        print(f"[dry-run] curl で {len(manifest)} ファイルを取得する")
        return

    repo = DATASETS[dataset]["repo"]
    base = f"https://huggingface.co/datasets/{repo}/resolve/main"
    raw.mkdir(parents=True, exist_ok=True)
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        list(pool.map(lambda e: _fetch_one(base, e[0], e[1], raw), manifest))


def _fetch_one(base: str, name: str, size: int | None, raw: Path) -> None:
    """1ファイル取る。妥当なサイズで既にあれば飛ばす。"""
    path = raw / name
    if path.is_file():
        have = path.stat().st_size
        if have == size or (size is None and have >= MIN_SIZE):
            return
        print(f"再取得（サイズ不一致 {have}B）: {name}")
        path.unlink()

    # HuggingFaceは = をエンコードしたパスで配る
    url = f"{base}/{name.replace('=', '%3D')}"
    part = path.with_suffix(path.suffix + ".part")
    ok = proc.succeeds(
        ["curl", "-fsSL", "--retry", "3", "--retry-delay", "5", "-o", str(part), url]
    )
    if not ok:
        raise proc.Fail(f"取得に失敗した: {name}")
    part.replace(path)
    print(f"取得: {name}")


def _verify(manifest: list[tuple[str, int | None]], raw: Path) -> bool:
    bad = 0
    for name, size in manifest:
        path = raw / name
        if not path.is_file():
            print(f"欠落: {name}")
            bad += 1
            continue
        have = path.stat().st_size
        if (size is not None and have != size) or (size is None and have < MIN_SIZE):
            print(f"サイズ不一致 {have}B（期待{size}B）: {name}")
            bad += 1
    print(f"検査 {len(manifest)} ファイル、異常 {bad} 件")
    return bad == 0


def _prepare(raw: Path, train: Path, *, dry_run: bool) -> None:
    """検証データを切り出し、残りをシャッフルして学習データにする。"""
    psv = paths.REPO / "target" / "release" / "psv"
    if not psv.is_file() and not dry_run:
        raise proc.Fail(f"{paths.rel(psv)} がない。先に cargo build --release を実行する")
    train.mkdir(parents=True, exist_ok=True)

    valid_src = raw / file_name(VALID_START_TIME, VALID_INDEX)
    print(f"検証データを切り出す（{VALID_COUNT}局面）")
    proc.run(
        [
            str(psv), "head",
            "--in", str(valid_src),
            "--out", str(train / "valid_385M.psv"),
            "--count", str(VALID_COUNT),
        ],
        dry_run=dry_run,
    )

    print("学習データをシャッフルする（検証データの供給元を除く）")
    excluded = valid_src.name
    sources = sorted(p for p in raw.glob("*.bin") if p.name != excluded)
    if not sources and not dry_run:
        raise proc.Fail(f"生データがない: {paths.rel(raw)}")
    proc.run(
        [
            str(psv), "shuffle",
            "--in", ",".join(str(p) for p in sources) or "（生データ）",
            "--out", str(train / "train_2990M.psv"),
            "--seed", "42",
        ],
        dry_run=dry_run,
    )
    proc.run(
        [str(psv), "stats", "--in", str(train / "train_2990M.psv"), "--limit", "1"],
        dry_run=dry_run,
        allowed=(0, 1, 2, 3),
    )


# --- quiet -------------------------------------------------------------


def quiet(args: argparse.Namespace) -> int:
    """教師局面をqsearchの静止局面へ置き換える。"""
    source, out = Path(args.input), Path(args.output)
    if not source.is_file() and not args.dry_run:
        raise proc.Fail(f"入力のpsvがない: {source}")

    eval_file = args.eval_file or config.get("EVAL_FILE")
    if not eval_file and not args.dry_run:
        raise proc.Fail("評価関数がない。--eval-file で渡す")

    psv = paths.REPO / "target" / "release" / "psv"
    if not psv.is_file() and not args.dry_run:
        raise proc.Fail(f"{paths.rel(psv)} がない。先に cargo build --release を実行する")

    name = args.name or out.name.removesuffix(".psv")
    paths.check_name(name)
    out.parent.mkdir(parents=True, exist_ok=True)

    print(f"=== 教師局面の静止化: {name} ===")
    print(f"入力    : {paths.rel(source)}")
    print(f"出力    : {paths.rel(out)}")
    print(f"上限手数: {args.max_plies}")

    argv = [
        str(psv), "quiet",
        "--in", str(source),
        "--out", str(out),
        "--max-plies", str(args.max_plies),
        "--eval-file", eval_file or "（未設定）",
    ]
    if args.limit is not None:
        argv += ["--limit", str(args.limit)]
    return proc.run(argv, dry_run=args.dry_run, log=paths.log("quiet", name))
