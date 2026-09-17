"""`hmwr data` の操作を、`psv` のサブコマンドへの対応表から作る（ADR-0208）。

**1操作は表の1行である。** 名前からのパス解決・既定値・ログ・完了印は
ここの共通の実装が足す。部品を足すコストを1行にすることが、`psv` を
直接叩かせないための本体になる。

完了印は `<出力>.done` に置く。出力は `.part` へ書いてから改名するので、
出力があれば最後まで書けている。完了印には走らせたコマンドを控え、同じ名前で
条件の違う再実行を止める。
"""

from __future__ import annotations

import argparse
import json
import time
from dataclasses import dataclass
from pathlib import Path

from . import config, paths, proc

# 静止化の並列数。**並列の出力はjobs固定で決定論になる**（逐次とは一致しない）
# ので、既定を1か所で持つ。ADR-0200〜0206のチェーン11回はすべて8だった
QUIET_JOBS = 8
SHUFFLE_SEED = 1

# 既定値の印。評価関数は実行時に config から引く
EVAL = object()


@dataclass(frozen=True)
class Opt:
    """psvへそのまま渡すオプション。綴りはhmwrとpsvで同じにする。"""

    flag: str
    help: str
    metavar: str = "N"
    kind: type = int
    default: object = None  # Noneなら、指定がない限りpsvへ渡さない
    required: bool = False


@dataclass(frozen=True)
class Op:
    name: str  # hmwr data の操作名。ログの領域名にもなる
    psv: str  # psv のサブコマンド
    help: str
    description: str
    opts: tuple[Opt, ...] = ()
    suffix: str = ".psv"  # 出力の拡張子
    min_inputs: int = 1
    max_inputs: int | None = 1
    raw: bool = False  # --raw <データセット> で生データを入力にできる
    split_jobs: bool = False  # 入力を区間で割って並列に走らせ、結合する


EVAL_OPT = Opt("--eval-file", "評価関数（既定は EVAL_FILE）", "パス", str, EVAL)
LIMIT_OPT = Opt("--limit", "先頭のこの件数だけ処理する")
HASH_OPT = Opt("--hash", "置換表の大きさ", "MB")

OPS: tuple[Op, ...] = (
    Op(
        "shuffle",
        "shuffle",
        "全体をシャッフルする",
        "2パスのバケット法で動き、入力の大きさに制限はない。"
        "一時ファイルは出力と同じ場所に作る。",
        opts=(Opt("--seed", "乱数種", default=SHUFFLE_SEED),),
        raw=True,
    ),
    Op(
        "split",
        "head",
        "先頭から区間を切り出す",
        "学習用と検証用の切り出しに使う。検証用を先頭から取り、学習用は "
        "--skip で検証用の件数を飛ばして取る。",
        opts=(
            Opt("--count", "切り出す件数", required=True),
            Opt("--skip", "先頭から飛ばす件数"),
        ),
    ),
    Op(
        "mix",
        "shuffle",
        "複数の教師を混ぜてシャッフルする",
        "混合の比率は入力の件数で決まる。比率を変えるなら、先に split で"
        "件数を揃える。",
        opts=(Opt("--seed", "乱数種", default=SHUFFLE_SEED),),
        min_inputs=2,
        max_inputs=None,
    ),
    Op(
        "quiet",
        "quiet",
        "教師局面を静止局面へ置き換える",
        "評価関数が探索中に見るのは静止局面だが、公開データは"
        "取り合いの途中の局面へ収束後の評価値を付けて配られている。"
        "そのずれを消す。並列8で6,000万局面に12分かかる。"
        "**学習データを静止化したら、検証集合も同じ設定で静止化する。**",
        opts=(
            Opt("--max-plies", "進める手数の上限", default=1),
            EVAL_OPT,
            Opt("--jobs", "並列数。変えると出力が変わる", default=QUIET_JOBS),
            LIMIT_OPT,
            HASH_OPT,
        ),
    ),
    Op(
        "rank",
        "rank",
        "兄弟局面の葉の群を作る",
        "ランキング損失の教師になる。psv rank は単スレッドなので、"
        "--jobs で入力を区間に割って並列に走らせ、順に結合する。"
        "区間はレコードの絶対位置で切る。",
        opts=(Opt("--skip", "先頭から飛ばす件数"), LIMIT_OPT, EVAL_OPT, HASH_OPT),
        suffix=".rankpsv",
        split_jobs=True,
    ),
)


def psv_bin() -> Path:
    return paths.REPO / "target" / "release" / "psv"


def dest(flag: str) -> str:
    return flag.lstrip("-").replace("-", "_")


# --- 登録 --------------------------------------------------------------


def add_parsers(ss: argparse._SubParsersAction) -> None:
    for op in OPS:
        t = ss.add_parser(op.name, help=op.help, description=op.description)
        t.add_argument("name", metavar="出力名", help=f"data/train/<出力名>{op.suffix} へ書く")
        t.add_argument(
            "--in",
            dest="inputs",
            action="append",
            default=[],
            metavar="入力名",
            help="data/train/<入力名>.psv を読む"
            + ("" if op.max_inputs == 1 else "。複数回渡せる"),
        )
        if op.raw:
            t.add_argument(
                "--raw",
                metavar="データセット",
                help="data/raw/<データセット>/ の生データ（*.bin）を入力にする",
            )
        for o in op.opts:
            default_note = ""
            if o.default is not None and o.default is not EVAL:
                default_note = f"（既定 {o.default}）"
            t.add_argument(
                o.flag,
                type=o.kind,
                metavar=o.metavar,
                required=o.required,
                help=o.help + default_note,
            )
        if op.split_jobs:
            t.add_argument(
                "--jobs", type=int, default=1, metavar="N", help="並列数。--limit が要る（既定 1）"
            )
        t.add_argument(
            "--force", action="store_true", help="出力が既にあっても作り直す"
        )
        t.set_defaults(func=lambda args, op=op: run(op, args))


# --- 実行 --------------------------------------------------------------


def _inputs(op: Op, args: argparse.Namespace) -> list[Path]:
    raw = getattr(args, "raw", None)
    if raw:
        if args.inputs:
            raise proc.Fail("--in と --raw は同時に渡せない", proc.USAGE)
        raw_dir = paths.RAW / paths.check_name(raw)
        found = sorted(raw_dir.glob("*.bin"))
        if not found:
            if args.dry_run:
                return [raw_dir / "*.bin"]
            raise proc.Fail(f"生データがない: {paths.rel(raw_dir)}")
        return found

    n = len(args.inputs)
    if n < op.min_inputs or (op.max_inputs is not None and n > op.max_inputs):
        want = (
            f"{op.min_inputs}個"
            if op.max_inputs == op.min_inputs
            else f"{op.min_inputs}個以上"
        )
        raise proc.Fail(f"--in は{want}要る（{n}個渡された）", proc.USAGE)
    found = [paths.TRAIN / f"{paths.check_name(x)}.psv" for x in args.inputs]
    if not args.dry_run:
        for p in found:
            if not p.is_file():
                raise proc.Fail(f"入力のpsvがない: {paths.rel(p)}")
    return found


def _opt_args(op: Op, args: argparse.Namespace, *, skip: tuple[str, ...] = ()) -> list[str]:
    out: list[str] = []
    for o in op.opts:
        if o.flag in skip:
            continue
        value = getattr(args, dest(o.flag))
        if value is None:
            value = o.default
        if value is EVAL:
            value = config.get("EVAL_FILE")
            if not value:
                if not args.dry_run:
                    raise proc.Fail("評価関数がない。--eval-file で渡す")
                value = "（未設定）"
        if value is not None:
            out += [o.flag, str(value)]
    return out


def _commands(op: Op, args: argparse.Namespace, part: Path) -> tuple[list[list[str]], list[Path]]:
    """走らせるpsvのコマンド列と、結合する区間の出力を返す。

    入力はリポジトリからの相対パスで渡す（実行時のcwdはリポジトリ）。
    コンマで連結した引数は `proc.show` が相対にできず、完了印の記録が
    マシンの置き場に依存してしまう。
    """
    head = [str(psv_bin()), op.psv, "--in", ",".join(paths.rel(p) for p in _inputs(op, args))]
    jobs = getattr(args, "jobs", 1) if op.split_jobs else 1
    if not op.split_jobs or jobs <= 1:
        return [[*head, "--out", str(part), *_opt_args(op, args)]], []

    if args.limit is None:
        raise proc.Fail("--jobs で割るには --limit が要る", proc.USAGE)
    base = args.skip or 0
    per = -(-args.limit // jobs)
    rest = _opt_args(op, args, skip=("--skip", "--limit"))
    commands, pieces = [], []
    for k in range(jobs):
        count = min(per, args.limit - k * per)
        if count <= 0:
            break
        piece = part.with_name(f"{part.name}{k:03d}")
        pieces.append(piece)
        commands.append(
            [*head, "--out", str(piece), "--skip", str(base + k * per), "--limit", str(count), *rest]
        )
    return commands, pieces


def _concat(pieces: list[Path], out: Path) -> None:
    with open(out, "wb") as dst:
        for piece in pieces:
            with open(piece, "rb") as src:
                while chunk := src.read(1 << 24):
                    dst.write(chunk)
    for piece in pieces:
        piece.unlink()


def run(op: Op, args: argparse.Namespace) -> int:
    out = paths.TRAIN / f"{paths.check_name(args.name)}{op.suffix}"
    part = out.with_name(out.name + ".part")
    done = out.with_name(out.name + ".done")
    commands, pieces = _commands(op, args, part)
    record = [proc.show(c) for c in commands]
    log = paths.log(op.name, args.name)

    if args.dry_run:
        for c in commands:
            proc.run(c, dry_run=True)
        if pieces:
            print(f"[dry-run] {len(pieces)}区間を順に結合する → {paths.rel(part)}")
        print(f"[dry-run] mv {paths.rel(part)} {paths.rel(out)}")
        print(f"[dry-run] ログ: {paths.rel(log)}")
        return proc.OK

    if out.exists() and not args.force:
        return _already(out, done, record)
    if not psv_bin().is_file():
        raise proc.Fail(f"{paths.rel(psv_bin())} がない。先に cargo build --release を実行する")

    out.parent.mkdir(parents=True, exist_ok=True)
    done.unlink(missing_ok=True)
    print(f"=== data {op.name}: {args.name} ===")
    started = time.time()
    if pieces:
        proc.run_all(commands, log=log)
        _concat(pieces, part)
    else:
        proc.run(commands[0], log=log)
    part.replace(out)
    done.write_text(
        json.dumps(
            {
                "commands": record,
                "bytes": out.stat().st_size,
                "seconds": round(time.time() - started),
                "finished": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n"
    )
    print(f"完了: {paths.rel(out)}（{out.stat().st_size:,}バイト）")
    return proc.OK


def _already(out: Path, done: Path, record: list[str]) -> int:
    """出力が既にあるときの扱い。同じ条件なら済み、違えば止める。"""
    if not done.is_file():
        raise proc.Fail(
            f"出力が既にあり、由来の記録がない: {paths.rel(out)}\n"
            "名前を変えるか、--force で作り直す"
        )
    before = json.loads(done.read_text()).get("commands")
    if before != record:
        raise proc.Fail(
            f"同じ名前で条件の違う出力がある: {paths.rel(out)}\n"
            f"前回: {before}\n今回: {record}\n"
            "名前を変えるか、--force で作り直す"
        )
    print(f"済み: {paths.rel(out)}（同じ条件で作成済み。何もしない）")
    return proc.OK
