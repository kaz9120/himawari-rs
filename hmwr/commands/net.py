"""評価関数の学習・評価・配布。

既定は測定で決まった結論に揃えてある（ADR-0135・0138・0065・0066）。
変えるときはフラグで渡す。**環境変数を組み立てる必要はない。**
"""

from __future__ import annotations

import argparse
import re
from pathlib import Path

from .. import config, paths, proc

ARCH_RE = re.compile(r"^\d+x\d+(x\d+){0,2}$")
TRAINER = "training/train.py"
REGISTRY = "training/runs/registry.tsv"
RUNS = "training/runs/net_shape"

# 1局面40バイト固定（ADR-0038）
BYTES_PER_POSITION = 40
BATCH = 16384

# 既定の学習条件。数値の根拠はADR-0135にある
DEFAULT_VALID = "data/train/valid_385M.psv"
DEFAULT_EVAL_VALID = "data/train/valid_385M_q1.psv"
FT_CLIP = "1.0"
BASE_FLAGS = ["--batch-loader", "--dense-ft", "--factorized"]


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("net", help="評価関数を学習・評価・配布する")
    ss = p.add_subparsers(dest="sub", metavar="<操作>")

    t = ss.add_parser(
        "train",
        help="本番規模のネットを学習する",
        description="data/nets/<名前>.hmwr へ書き出し、"
        "training/checkpoints/<名前>/ へ途中経過を残す。"
        "検証データは学習データと同じ前処理に揃える。"
        "土俵がずれると最良チェックポイントの選択が歪む。",
    )
    t.add_argument("name", help="ネット名。ログと台帳の名前にもなる")
    t.add_argument("--data", required=True, metavar="PSV", help="学習データ")
    t.add_argument("--valid", metavar="PSV", help=f"検証データ（既定 {DEFAULT_VALID}）")
    t.add_argument("--init-ckpt", metavar="パス", help="継続学習の初期値")
    t.add_argument("--lr", metavar="値", help="学習率の頂点。継続学習の既定は1e-4")
    t.add_argument("--warmup", type=int, metavar="N", help="学習率を上げきるまでのステップ数")
    t.add_argument("--device", metavar="名前", help="mps か cpu（既定 mps）")
    t.add_argument("--seed", type=int, default=0, metavar="N", help="乱数種")
    t.add_argument("--notes", metavar="文", help="実験台帳へ書く備考")
    t.add_argument(
        "--extra",
        metavar="引数",
        help="学習器へ素通しする追加引数。ハイフンで始まる値は --extra=--flag と書く",
    )
    t.set_defaults(func=train)

    t = ss.add_parser(
        "shapes",
        help="ネットワーク構成ごとに小さく学習して比べる",
        description="学習側の次元はPyO3拡張から読むので、構成ごとに拡張を"
        "ビルドし直して順に回す。並列に回すと拡張が上書きし合い、"
        "次元の違う構成で学習してしまう。",
    )
    t.add_argument("specs", nargs="+", metavar="構成", help="例 256x16 512x16x32")
    t.add_argument("--data", metavar="PSV", help="学習データ（既定 train_300M.psv）")
    t.add_argument("--valid", metavar="PSV", help="検証データ")
    t.add_argument("--tag", default="train", metavar="名前", help="出力名の頭")
    t.add_argument("--seed", type=int, default=0, metavar="N", help="乱数種")
    t.add_argument("--device", default="cpu", metavar="名前", help="cpu か mps")
    t.add_argument("--mmap", action="store_true", help="学習データをmmapで開く")
    t.add_argument("--init-net", metavar="ネット", help="初期値のネット")
    t.add_argument("--freeze-ft", action="store_true", help="FTを凍結する")
    t.add_argument("--distill-net", metavar="ネット", help="蒸留の教師")
    t.add_argument("--lambda-distill", metavar="値", help="蒸留の重み")
    t.add_argument("--effect-head", metavar="種類", help="利き予測のヘッド")
    t.add_argument("--lambda-effect", metavar="値", help="利き損失の重み")
    t.add_argument("--lambda-value", metavar="値", help="評価値損失の重み")
    t.add_argument("--lr", metavar="値", help="学習率の頂点")
    t.add_argument("--ft-clip", default=FT_CLIP, metavar="値", help="FTクリップ（0で無効）")
    t.add_argument(
        "--generate",
        type=int,
        metavar="N",
        help="教師データの代わりに局面をその場で作る。1エポックあたりの局面数",
    )
    t.set_defaults(func=shapes)

    t = ss.add_parser(
        "eval",
        help="ネットの検証損失を測る",
        description="学習は回さず、書き出したネットを並べて測る。"
        "**採否は対局で決める。** 検証損失は初期値の系列が違うだけで動く。",
    )
    t.add_argument("nets", nargs="+", metavar="ネット")
    t.add_argument("--valid", metavar="PSV", help="検証集合。複数はコンマ区切り")
    t.add_argument("--device", default="cpu", metavar="名前", help="cpu か mps")
    t.set_defaults(func=evaluate)

    t = ss.add_parser(
        "release",
        help="ネットをGitHub Releaseで配る",
        description="既定では作らない。走るはずのコマンドとノートを出して終わる。"
        "実際に作るには --apply を付ける。",
    )
    t.add_argument("file", metavar="ネット")
    t.add_argument("version", type=int, metavar="番号")
    t.add_argument("--notes", metavar="文", help="リリースノートへの追記")
    t.add_argument("--apply", action="store_true", help="実際に作る")
    t.set_defaults(func=release)


# --- train -------------------------------------------------------------


def positions(psv: Path) -> int:
    """psvの局面数。1局面40バイト固定である（ADR-0038）。"""
    return psv.stat().st_size // BYTES_PER_POSITION


def train(args: argparse.Namespace) -> int:
    """本番規模の学習。既定はADRの結論に揃えてある。"""
    name = paths.check_name(args.name)
    data = Path(args.data)
    valid = Path(args.valid or DEFAULT_VALID)

    if not args.dry_run:
        for path, label in ((data, "学習データ"), (valid, "検証データ")):
            if not path.is_file():
                raise proc.Fail(f"{label}がない: {path}")

    out = paths.NETS / f"{name}.hmwr"
    argv = [
        "python3",
        TRAINER,
        "--data", str(data),
        "--valid", str(valid),
        "--out", str(out),
        *BASE_FLAGS,
        # 29.9億のpsvはRAMに載らない（ADR-0065）
        "--mmap",
        # i8で格納するので制約なしだと書き出しが落ちる（ADR-0138）
        "--ft-clip", FT_CLIP,
        "--device", args.device or "mps",
        "--seed", str(args.seed),
        "--checkpoint-dir", f"training/checkpoints/{name}",
        "--log-file", f"{RUNS}/{name}.tsv",
        "--registry", REGISTRY,
        "--name", name,
        "--notes", args.notes or f"{data} で学習",
    ]

    lr = args.lr
    if args.init_ckpt:
        if not Path(args.init_ckpt).is_file() and not args.dry_run:
            raise proc.Fail(f"初期値のチェックポイントがない: {args.init_ckpt}")
        argv += ["--init-checkpoint", args.init_ckpt]
        # 前世代の表現を壊さない幅。3e-4では壊れる（ADR-0145）
        lr = lr or "1e-4"
        argv += _continual_args(data, args, dry_run=args.dry_run)
    if lr:
        argv += ["--peak-lr", lr]
    if args.extra:
        argv += args.extra.split()

    for directory in (paths.NETS, paths.REPO / RUNS, paths.CHECKPOINTS / name):
        directory.mkdir(parents=True, exist_ok=True)

    print(f"=== 学習: {name} ===")
    print(f"学習データ: {paths.rel(data)}")
    print(f"検証データ: {paths.rel(valid)}")
    print(f"出力      : {paths.rel(out)}")
    if args.init_ckpt:
        print(f"初期値    : {args.init_ckpt}（継続学習、学習率 {lr}）")
    return proc.run(argv, dry_run=args.dry_run, log=paths.log("train", name))


def _continual_args(data: Path, args: argparse.Namespace, *, dry_run: bool) -> list[str]:
    """継続学習の刻みを学習データの規模から決める。

    warmupは総ステップの4%にする。固定値だと規模で意味が変わり、824万局面
    （503ステップ）で決めた20は1億局面（6,100ステップ）では0.3%になる。
    学習率を上げきるまでの区間が短いほど前世代の表現が壊れやすい。
    """
    if dry_run and not data.is_file():
        return ["--warmup-steps", "20", "--valid-interval", "50"]
    steps = positions(data) // BATCH
    warmup = args.warmup or max(steps * 4 // 100, 20)
    interval = max(steps // 20, 50)
    print(f"総ステップ {steps}、warmup {warmup}、検証間隔 {interval}")
    return ["--warmup-steps", str(warmup), "--valid-interval", str(interval)]


# --- shapes ------------------------------------------------------------


def shapes(args: argparse.Namespace) -> int:
    """構成ごとに拡張をビルドし直して学習する。"""
    for spec in args.specs:
        if not ARCH_RE.match(spec):
            raise proc.Fail(
                f"構成の書き方が違う: {spec}（<FT>x<L1>[x<L2>[x<L3>]]）", proc.USAGE
            )
    if args.effect_head and not args.lambda_effect:
        raise proc.Fail("--effect-head には --lambda-effect が要る", proc.USAGE)

    common = _shape_common_args(args)
    source = f"生成{args.generate}局面" if args.generate else (args.data or "data/train/train_300M.psv")

    print(f"=== 構成ごとの学習（{len(args.specs)}件） ===")
    print(f"学習データ: {source}")
    print(f"デバイス: {args.device}、名前の頭: {args.tag}、種: {args.seed}")

    wheels = paths.REPO / "target" / "wheels-shape"
    for spec in args.specs:
        print(f"拡張をビルド: {spec}")
        proc.run(
            [
                "maturin", "build", "--release", "--quiet",
                "-m", "crates/py/Cargo.toml",
                "--out", str(wheels),
            ],
            dry_run=args.dry_run,
            env={"HIMAWARI_ARCH": spec, "CARGO_TARGET_DIR": f"target/shape/{spec}"},
        )
        _install_wheel(wheels, spec, dry_run=args.dry_run)

        name = f"{args.tag}-{spec}-s{args.seed}"
        print(f"学習: {spec}")
        proc.run(
            [
                "python3", TRAINER,
                *common,
                "--out", f"data/nets/{name}.hmwr",
                *BASE_FLAGS,
                "--device", args.device,
                "--seed", str(args.seed),
                "--log-file", f"{RUNS}/{name}.tsv",
                "--registry", REGISTRY,
                "--name", f"{args.tag}_{spec}_s{args.seed}",
                "--notes", f"構成の比較: {spec}、種 {args.seed}、{source}",
            ],
            dry_run=args.dry_run,
            log=paths.log("shapes", name),
        )

    print()
    print(f"検証損失を比べる: column -t -s $'\\t' {REGISTRY} | grep '{args.tag}_'")
    return proc.OK


def _shape_common_args(args: argparse.Namespace) -> list[str]:
    """構成比較で共通の引数を組み立てる。"""
    out: list[str] = []
    if args.generate:
        # 生成した局面は使い捨てなので検証集合が要らない（ADR-0133）
        out += ["--generate", str(args.generate)]
    else:
        out += [
            "--data", args.data or "data/train/train_300M.psv",
            "--valid", args.valid or DEFAULT_VALID,
        ]
    if args.mmap:
        out.append("--mmap")
    if args.init_net:
        out += ["--init-net", args.init_net]
        if args.freeze_ft:
            out.append("--freeze-ft")
    if args.distill_net:
        out += ["--distill-net", args.distill_net]
        if args.lambda_distill:
            out += ["--lambda-distill", args.lambda_distill]
    if args.effect_head:
        out += ["--effect-head", args.effect_head, "--lambda-effect", args.lambda_effect]
    if args.lambda_value:
        out += ["--lambda-value", args.lambda_value]
    if args.lr:
        out += ["--peak-lr", args.lr]
    if args.ft_clip not in ("0", "0.0"):
        out += ["--ft-clip", args.ft_clip]
    return out


def _install_wheel(wheels: Path, spec: str, *, dry_run: bool) -> None:
    """作った拡張を入れ、構成が合っているかを確かめる。"""
    if dry_run:
        print(f"[dry-run] pip install（{spec} の拡張）")
        return
    built = sorted(wheels.glob("*.whl"), key=lambda p: p.stat().st_mtime, reverse=True)
    if not built:
        raise proc.Fail(f"拡張が作られていない: {paths.rel(wheels)}")
    proc.run(
        ["python3", "-m", "pip", "install", "--force-reinstall", "--no-deps",
         "--quiet", str(built[0])],
        dry_run=False,
    )
    got = proc.capture(["python3", "-c", "import himawari; print(himawari.ARCH)"]).strip()
    if got != spec:
        raise proc.Fail(f"拡張の構成が合わない: {got}（期待 {spec}）")


# --- eval --------------------------------------------------------------


def evaluate(args: argparse.Namespace) -> int:
    """書き出したネットを検証集合で測って表にする。

    土俵を跨いで比べたいときだけ検証集合を複数渡す。教師データの分布を
    変える実験では物差しも一緒に動く（ADR-0136）。
    """
    valids = [v for v in (args.valid or DEFAULT_EVAL_VALID).split(",") if v]
    if not args.dry_run:
        for path in valids:
            if not Path(path).is_file():
                raise proc.Fail(f"検証データがない: {path}")
        for net in args.nets:
            if not Path(net).is_file():
                raise proc.Fail(f"ネットがない: {net}")

    print(f"=== 検証損失の測定（{len(args.nets)}件） ===")
    header = f"{paths.pad('ネット', 52)}{paths.pad('検証集合', 34)}loss"
    print(header)

    for net in args.nets:
        # 拡張子で初期値の読み方を選ぶ。.hmwr は量子化済み、.ckpt はf32
        flag = "--init-checkpoint" if net.endswith((".ckpt", ".pt")) else "--init-net"
        for valid in valids:
            argv = [
                "python3", TRAINER, "--eval-only", flag, net,
                "--valid", valid, *BASE_FLAGS, "--device", args.device,
            ]
            if args.dry_run:
                print(f"[dry-run] {proc.show(argv)}")
                continue
            out, err = proc.capture_both(argv)
            loss = _last_loss(out)
            if loss is None:
                detail = err.strip().splitlines()[-3:] if err.strip() else []
                raise proc.Fail(
                    "\n".join([f"測れない: {net} / {valid}", *detail])
                )
            print(
                paths.pad(Path(net).name, 52)
                + paths.pad(Path(valid).name, 34)
                + loss
            )
    return proc.OK


def _last_loss(output: str) -> str | None:
    """学習器の出力（タブ区切り）から損失の列を取る。"""
    for line in reversed(output.splitlines()):
        parts = line.split("\t")
        if len(parts) >= 3 and parts[2].strip():
            return parts[2].strip()
    return None


# --- release -----------------------------------------------------------


def release(args: argparse.Namespace) -> int:
    argv = [proc.script("release-net.sh"), args.file, str(args.version)]
    if args.notes:
        argv.append(args.notes)
    if args.apply:
        argv.append("--apply")
    return proc.run(argv, dry_run=args.dry_run)
