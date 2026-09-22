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
import struct
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

from . import config, paths, proc
from .tools import focus_labels

# 静止化の並列数。**並列の出力はjobs固定で決定論になる**（逐次とは一致しない）
# ので、既定を1か所で持つ。ADR-0200〜0206のチェーン11回はすべて8だった
QUIET_JOBS = 8
SHUFFLE_SEED = 1

# 既定値の印。評価関数は実行時に config から引く
EVAL = object()

# DL系モデルによる付け直し（ADR-0215）。モデルは配布元のライセンスに従って
# 手元に置く。パスワードつきの配布なので、取得の手順はIssue #556にある
DL_MODEL = "data/models/dlshogi/model-dr2_exhi.onnx"
DL_SCALE = 600.0


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
        "一時ファイルは出力と同じ場所に作る。--limit を渡すと、入力の先頭"
        "この件数だけを読む。生データの先頭だけを元にするときに使う。",
        opts=(Opt("--seed", "乱数種", default=SHUFFLE_SEED), LIMIT_OPT),
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
        "そのずれを消す。並列8で6,000万局面に2〜12分かかる。"
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
    Op(
        "oversample",
        "oversample",
        "該当する型の局面を複製して重くする",
        "教師の最善手から型を判定し、該当する局面を末尾へ（--times−1）回"
        "書き足す。判定に教師手が要るので、静止化の前に通す。出力の並びは"
        "元の全件のあとに複製が続くので、学習の前に shuffle を掛ける。",
        opts=(
            Opt("--kind", "重くする型", "型", str, "defense"),
            Opt("--times", "該当局面を何倍にするか", default=3),
            LIMIT_OPT,
        ),
    ),
)


def psv_bin() -> Path:
    return paths.release_bin("psv")


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

    t = ss.add_parser("stats", help="局面数・評価値の分布・勝敗を表示する")
    t.add_argument("name", metavar="名前", help="data/train/<名前>.psv")
    t.add_argument("--limit", type=int, metavar="N", help="先頭のこの件数だけ読む")
    t.set_defaults(func=stats)

    t = ss.add_parser(
        "rm",
        help="hmwr data が作った中間ファイルを消す",
        description="完了印のある出力だけを消す。完了印に作り方が残っているので、"
        "消しても同じコマンドで作り直せる。由来の記録がないファイルには触らない。",
    )
    t.add_argument(
        "names", nargs="+", metavar="名前", help="data/train/<名前>の.psv・.rankpsv・.focus"
    )
    t.set_defaults(func=remove)

    t = ss.add_parser(
        "openings",
        help="教師データから開始局面集を作る",
        description="手数の条件を満たす局面を先頭から拾い、openings/<出力名>.txt へ"
        "1行1局面のSFENで書く。入力はシャッフル済みのものを使う。",
    )
    t.add_argument("name", metavar="出力名", help="openings/<出力名>.txt へ書く")
    t.add_argument("--in", dest="inputs", action="append", default=[], metavar="入力名")
    t.add_argument("--count", type=int, required=True, metavar="N", help="拾う局面数")
    t.add_argument("--min-ply", type=int, default=0, metavar="N", help="この手数以上の局面だけ拾う")
    t.add_argument("--skip", type=int, default=0, metavar="N", help="先頭から飛ばす件数")
    t.add_argument("--force", action="store_true", help="出力が既にあっても作り直す")
    t.set_defaults(func=openings)

    t = ss.add_parser(
        "relabel",
        help="DL系モデルの推論1回の評価値でscoreを付け直す",
        description="局面・指し手・勝敗・手数は残し、scoreだけをモデルの勝率から"
        "戻した値へ書き換える。勝率→評価値の変換は `cp = scale × logit(p)` で、"
        "scaleを600から下げることは推論側の FV_SCALE を上げることに当たる。"
        "本エンジンの探索で付け直す `psv relabel` と違い、こちらは探索しない。",
    )
    t.add_argument("name", metavar="出力名", help="data/train/<出力名>.psv へ書く")
    t.add_argument("--in", dest="inputs", action="append", default=[], metavar="入力名")
    t.add_argument(
        "--labeler",
        default="dlshogi",
        choices=("dlshogi", "rescale"),
        help="裏側のモデルの種類（既定 dlshogi）。rescale は推論せず、既存のscoreを "
        "scale/600 倍に縮める。スケールの水準を振るとき推論を1回で済ませる",
    )
    t.add_argument(
        "--model",
        default=DL_MODEL,
        metavar="パス",
        help=f"モデルのファイル（既定 {DL_MODEL}）",
    )
    t.add_argument(
        "--scale",
        type=float,
        default=DL_SCALE,
        metavar="S",
        help=f"勝率→評価値の変換のスケール（既定 {DL_SCALE:g}）",
    )
    t.add_argument(
        "--device",
        choices=("cpu", "coreml", "cuda"),
        help="推論のデバイス。省くと使えるものを選ぶ",
    )
    t.add_argument("--batch", type=int, default=4096, metavar="N", help="推論のバッチ（既定 4096）")
    t.add_argument("--limit", type=int, metavar="N", help="先頭のこの件数だけ処理する")
    t.add_argument(
        "--in-place",
        action="store_true",
        help="出力名のpsvをその場で書き換える。--in は要らない。書き換える前のscoreは "
        "<名前>.psv.scores-before.i16 へ控え、進み具合を <名前>.psv.relabel.json に記録して"
        "途中から続けられる。大きなpsvで空きが無いときに使う",
    )
    t.add_argument("--start", type=int, default=0, metavar="N", help="--in-place の範囲の先頭（既定 0）")
    t.add_argument("--count", type=int, metavar="N", help="--in-place の範囲の件数（既定は末尾まで）")
    t.add_argument("--force", action="store_true", help="出力が既にあっても作り直す")
    t.set_defaults(func=relabel)

    t = ss.add_parser(
        "focus",
        help="対局順の生データから焦点の熱地図つき局面集を作る",
        description="ある局面から先のk手で駒が動いたマスと取られたマスを"
        "9×9の熱地図にし、盤上の駒ごとの関与フラグを付ける。"
        "入力は対局順のままの生データに限る。シャッフル済みの教師は続きを"
        "持たないので使えない。出力は202バイト固定長で、"
        "data/train/<出力名>.focus へ書く。",
    )
    t.add_argument("name", metavar="出力名", help=f"data/train/<出力名>{focus_labels.SUFFIX} へ書く")
    t.add_argument(
        "--raw",
        required=True,
        metavar="データセット",
        help="data/raw/<データセット>/ の生データ（*.bin）を名前順に読む",
    )
    t.add_argument("--count", type=int, required=True, metavar="N", help="切り出す局面数")
    t.add_argument(
        "--plies",
        type=int,
        default=focus_labels.PLIES,
        metavar="N",
        help=f"先を見る手数（既定 {focus_labels.PLIES}）",
    )
    t.add_argument("--force", action="store_true", help="出力が既にあっても作り直す")
    t.set_defaults(func=focus)


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


# --- 確認 --------------------------------------------------------------


def stats(args: argparse.Namespace) -> int:
    source = paths.TRAIN / f"{paths.check_name(args.name)}.psv"
    if not source.is_file() and not args.dry_run:
        raise proc.Fail(f"入力のpsvがない: {paths.rel(source)}")
    argv = [str(psv_bin()), "stats", "--in", paths.rel(source)]
    if args.limit is not None:
        argv += ["--limit", str(args.limit)]
    return proc.run(argv, dry_run=args.dry_run)


# --- 片付け ------------------------------------------------------------


def remove(args: argparse.Namespace) -> int:
    """完了印のある出力だけを消す。消す前に全部の対象を確かめる。"""
    targets: list[tuple[Path, Path]] = []
    for name in args.names:
        paths.check_name(name)
        if args.dry_run:
            # 消す対象は、手順の前のステップが実行時に作る。予行では存在を問わない
            print(
                f"[dry-run] rm {paths.rel(paths.TRAIN / name)} の"
                ".psv・.rankpsv・.focus と、その完了印"
            )
            continue
        found = [
            p
            for suffix in (".psv", ".rankpsv", focus_labels.SUFFIX)
            if (p := paths.TRAIN / f"{name}{suffix}").exists()
        ]
        if not found:
            raise proc.Fail(f"消す対象がない: {name}")
        for out in found:
            done = out.with_name(out.name + ".done")
            if not done.is_file():
                raise proc.Fail(
                    f"由来の記録がないので消さない: {paths.rel(out)}\n"
                    "hmwr data が作ったファイルだけを消せる"
                )
            targets.append((out, done))

    for out, done in targets:
        size = out.stat().st_size
        out.unlink()
        done.unlink()
        print(f"消した: {paths.rel(out)}（{size:,}バイト）")
    return proc.OK


# --- 開始局面集 --------------------------------------------------------

PSV_BYTES = 40
PLY_OFFSET = 36  # game_ply（u16、リトルエンディアン）


def openings(args: argparse.Namespace) -> int:
    """手数の条件で局面を拾い、SFENの列挙へ書き出す。"""
    if len(args.inputs) != 1:
        raise proc.Fail(f"--in は1個要る（{len(args.inputs)}個渡された）", proc.USAGE)
    source = paths.TRAIN / f"{paths.check_name(args.inputs[0])}.psv"
    out = paths.REPO / "openings" / f"{paths.check_name(args.name)}.txt"

    if args.dry_run:
        print(
            f"[dry-run] {paths.rel(source)} の{args.skip}件目から、"
            f"{args.min_ply}手目以降を{args.count}局面拾う"
        )
        print(f"[dry-run] {paths.rel(psv_bin())} dump で復元し {paths.rel(out)} へ書く")
        return proc.OK
    if not source.is_file():
        raise proc.Fail(f"入力のpsvがない: {paths.rel(source)}")
    if out.exists() and not args.force:
        raise proc.Fail(f"出力が既にある: {paths.rel(out)}\n名前を変えるか、--force で作り直す")

    picked = bytearray()
    with open(source, "rb") as fh:
        fh.seek(args.skip * PSV_BYTES)
        while len(picked) < args.count * PSV_BYTES:
            chunk = fh.read(PSV_BYTES * 65536)
            if not chunk:
                break
            for i in range(0, len(chunk) - PSV_BYTES + 1, PSV_BYTES):
                (ply,) = struct.unpack_from("<H", chunk, i + PLY_OFFSET)
                if ply >= args.min_ply:
                    picked += chunk[i : i + PSV_BYTES]
                    if len(picked) >= args.count * PSV_BYTES:
                        break
    found = len(picked) // PSV_BYTES
    if found < args.count:
        raise proc.Fail(f"条件を満たす局面が足りない（{found}/{args.count}）")

    with tempfile.NamedTemporaryFile(suffix=".psv") as tmp:
        tmp.write(picked)
        tmp.flush()
        dumped, err = proc.capture_both(
            [str(psv_bin()), "dump", "--in", tmp.name, "--limit", str(args.count)]
        )
    lines = [f"sfen {row.split(' | ')[0]}" for row in dumped.splitlines() if " | " in row]
    if len(lines) != args.count:
        raise proc.Fail(f"局面を復元できなかった（{len(lines)}/{args.count}）: {err.strip()}")
    out.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"完了: {paths.rel(out)}（{len(lines)}局面）")
    return proc.OK


# --- DL系モデルによる付け直し ------------------------------------------


def make_labeler(args: argparse.Namespace):
    """裏側のモデルを作る。テストでは差し替える。"""
    from .tools import dl_relabel

    if args.labeler == "rescale":
        return dl_relabel.RescaleLabeler()
    model = paths.REPO / args.model
    if not model.is_file():
        raise proc.Fail(
            f"モデルがない: {paths.rel(model)}\n"
            "配布元からライセンスに同意して取得し、この場所へ置く（Issue #556）"
        )
    return dl_relabel.DlshogiLabeler(model, args.device)


def relabel_in_place(args: argparse.Namespace) -> int:
    """出力名のpsvをその場で書き換える。進み具合は relabel.json が持つ。"""
    from .tools import dl_relabel

    if args.inputs:
        raise proc.Fail("--in-place では --in を渡さない。書き換える名前だけを渡す", proc.USAGE)
    target = paths.TRAIN / f"{paths.check_name(args.name)}.psv"
    model = "" if args.labeler == "rescale" else f" --model {args.model}"
    record = {"labeler": args.labeler, "model": args.model if args.labeler != "rescale" else None,
              "scale": args.scale}
    log = paths.log("relabel", args.name)
    if args.dry_run:
        rng = f"[{args.start}, {args.start + args.count})" if args.count else f"[{args.start}, 末尾)"
        print(f"[dry-run] relabel --in-place --labeler {args.labeler}{model} --scale {args.scale:g}"
              f" {paths.rel(target)} の {rng} をその場で書き換える")
        print(f"[dry-run] 元のscore: {paths.rel(dl_relabel.sidecar_path(target))}")
        print(f"[dry-run] 進み具合: {paths.rel(dl_relabel.progress_path(target))}")
        print(f"[dry-run] ログ: {paths.rel(log)}")
        return proc.OK
    if not target.is_file():
        raise proc.Fail(f"psvがない: {paths.rel(target)}")
    total = target.stat().st_size // dl_relabel.PSV_BYTES
    count = args.count or (total - args.start)
    labeler = make_labeler(args)
    print(f"=== data relabel（その場）: {args.name} [{args.start}, {args.start + count}) ===")
    device = getattr(labeler, "device", None)
    if device:
        print(f"デバイス: {device}")
    with open(log, "a", encoding="utf-8") as fh:

        def report(line: str) -> None:
            stamp = time.strftime("%Y-%m-%dT%H:%M:%S")
            print(line)
            fh.write(f"{stamp} {line}\n")
            fh.flush()

        report(f"開始: in-place {record} start={args.start} count={count}")
        try:
            state = dl_relabel.relabel_in_place(
                target, labeler, scale=args.scale, start=args.start, count=count,
                record=record, batch=args.batch, report=report,
            )
        except ValueError as e:
            raise proc.Fail(str(e)) from e
        report(f"終了: {state['done']:,}局面を書き換えた")
    print(f"完了: {paths.rel(target)}（元のscoreは {paths.rel(dl_relabel.sidecar_path(target))}）")
    return proc.OK


def relabel(args: argparse.Namespace) -> int:
    """scoreだけをDL系モデルの値へ書き換える。完了印の扱いは他の操作と同じ。"""
    from .tools import dl_relabel

    if args.in_place:
        return relabel_in_place(args)
    if len(args.inputs) != 1:
        raise proc.Fail(f"--in は1個要る（{len(args.inputs)}個渡された）", proc.USAGE)
    source = paths.TRAIN / f"{paths.check_name(args.inputs[0])}.psv"
    out = paths.TRAIN / f"{paths.check_name(args.name)}.psv"
    part = out.with_name(out.name + ".part")
    done = out.with_name(out.name + ".done")
    model = "" if args.labeler == "rescale" else f" --model {args.model}"
    record = [
        f"relabel --labeler {args.labeler}{model} --scale {args.scale:g}"
        f" --in {paths.rel(source)}"
        + (f" --limit {args.limit}" if args.limit is not None else "")
    ]
    log = paths.log("relabel", args.name)

    if args.dry_run:
        print(f"[dry-run] {record[0]} → {paths.rel(part)}")
        print(f"[dry-run] mv {paths.rel(part)} {paths.rel(out)}")
        print(f"[dry-run] ログ: {paths.rel(log)}")
        return proc.OK
    if not source.is_file():
        raise proc.Fail(f"入力のpsvがない: {paths.rel(source)}")
    if out.exists() and not args.force:
        return _already(out, done, record)

    labeler = make_labeler(args)
    out.parent.mkdir(parents=True, exist_ok=True)
    done.unlink(missing_ok=True)
    print(f"=== data relabel: {args.name} ===")
    device = getattr(labeler, "device", None)
    if device:
        print(f"デバイス: {device}")
    with open(log, "a", encoding="utf-8") as fh:

        def report(line: str) -> None:
            stamp = time.strftime("%Y-%m-%dT%H:%M:%S")
            print(line)
            fh.write(f"{stamp} {line}\n")
            fh.flush()

        report(f"開始: {record[0]}")
        stats = dl_relabel.relabel(
            source, part, labeler, scale=args.scale, batch=args.batch, limit=args.limit, report=report
        )
        report(
            f"終了: {stats['positions']:,}局面 {stats['positions_per_second']:,}局面/秒 "
            f"相関 {stats['corr_old_new']} 平均|score| {stats['mean_abs_old']}→{stats['mean_abs_new']}"
        )
    part.replace(out)
    done.write_text(
        json.dumps(
            {
                "commands": record,
                "bytes": out.stat().st_size,
                "seconds": stats["seconds"],
                "stats": stats,
                "finished": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n"
    )
    print(f"完了: {paths.rel(out)}（{out.stat().st_size:,}バイト）")
    return proc.OK


# --- 焦点の熱地図 ------------------------------------------------------


def focus(args: argparse.Namespace) -> int:
    """対局順の生データから、先k手の焦点を付けた局面集を作る（ADR-0213）。"""
    name = paths.check_name(args.name)
    dataset = paths.check_name(args.raw)
    raw_dir = paths.RAW / dataset
    out = paths.TRAIN / f"{name}{focus_labels.SUFFIX}"
    part = out.with_name(out.name + ".part")
    done = out.with_name(out.name + ".done")
    record = [f"focus --raw {dataset} --count {args.count} --plies {args.plies}"]
    log = paths.log("focus", name)
    sources = sorted(raw_dir.glob("*.bin"))

    if args.dry_run:
        print(f"[dry-run] {record[0]} → {paths.rel(part)}")
        print(f"[dry-run] 入力: {paths.rel(raw_dir)}/*.bin（{len(sources)}ファイル）")
        print(f"[dry-run] mv {paths.rel(part)} {paths.rel(out)}")
        print(f"[dry-run] ログ: {paths.rel(log)}")
        return proc.OK
    if not sources:
        raise proc.Fail(f"生データがない: {paths.rel(raw_dir)}")
    if out.exists() and not args.force:
        return _already(out, done, record)

    out.parent.mkdir(parents=True, exist_ok=True)
    done.unlink(missing_ok=True)
    print(f"=== data focus: {name} ===")
    with open(log, "a", encoding="utf-8") as fh:

        def report(line: str) -> None:
            print(line)
            fh.write(f"{time.strftime('%Y-%m-%dT%H:%M:%S')} {line}\n")
            fh.flush()

        report(f"開始: {record[0]}")
        stats = focus_labels.write(
            sources, part, count=args.count, plies=args.plies, report=report
        )
        if stats.rows < args.count:
            raise proc.Fail(f"切り出せた局面が足りない（{stats.rows}/{args.count}）")
        focus_labels.report_stats(stats, plies=args.plies, report=report)

    part.replace(out)
    done.write_text(
        json.dumps(
            {
                "commands": record,
                "bytes": out.stat().st_size,
                "seconds": round(stats.seconds),
                "stats": stats.summary(),
                "finished": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n"
    )
    print(f"完了: {paths.rel(out)}（{out.stat().st_size:,}バイト）")
    return proc.OK
