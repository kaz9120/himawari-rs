"""評価関数の学習・評価・配布。

既定は測定で決まった結論に揃えてある。
変えるときはフラグで渡す。**環境変数を組み立てる必要はない。**
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import tempfile
from pathlib import Path

from .. import conditions, config, heartbeat, paths, proc
from .. import release as release_mod
from ..tools import ft_reorder

TRAINER = "training/train.py"

# 学習曲線のTSVの置き場。paths.RUNS の下に置く
CURVES = "net_shape"

# 既定の構成の正。Python側では数を持たず、ここから読む（#804）
BUILD_RS = paths.REPO / "crates" / "engine" / "build.rs"
DEFAULT_ARCH_RE = re.compile(r'^const DEFAULT_ARCH: &str = "([^"]+)";', re.MULTILINE)

# 構成を選ぶフラグの説明。ビルドを伴う操作で同じ文を使う
ARCH_HELP = (
    "ネットワークの構成。例 512x16x64（省くと既定構成）。"
    "この構成でビルドし直す。省いたまま既定以外のネットを渡すと、"
    "読み込みが effect accumulator mismatch で落ちる"
)

# 1局面40バイト固定
BYTES_PER_POSITION = 40
BATCH = 16384

# 既定の学習条件
FT_CLIP = "1.0"
BASE_FLAGS = ["--batch-loader", "--dense-ft", "--factorized"]

# 学習器の出力から状態ファイルの値を拾う。形式は training/train.py の print にある
TOTAL_STEPS_RE = re.compile(r"total_steps=([0-9]+)")
STEP_RE = re.compile(r"^step ([0-9]+) samples [0-9]+ loss ([0-9.]+)")
VALID_RE = re.compile(r"^\s*valid loss ([0-9.]+)")


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("net", help="評価関数を学習・評価・配布する")
    ss = p.add_subparsers(dest="sub", metavar="<操作>")

    t = ss.add_parser(
        "train",
        help="本番規模のネットを学習する",
        description="data/nets/<名前>.hmwr へ書き出し、"
        "training/checkpoints/<名前>/ へ途中経過を残す。"
        "検証データは学習データと同じ前処理に揃える。"
        "条件がずれると最良チェックポイントの選択が歪む。",
    )
    t.add_argument("name", help="ネット名。ログと学習の記録の名前にもなる")
    t.add_argument(
        "--data", required=True, metavar="名前", help="学習データ。data/train/<名前>.psv"
    )
    t.add_argument(
        "--valid", metavar="名前", help=f"検証データ（既定 {paths.rel(config.train_valid())}）"
    )
    t.add_argument(
        "--rank", metavar="名前", help="ランキング損失の兄弟群。data/train/<名前>.rankpsv"
    )
    t.add_argument("--init-ckpt", metavar="パス", help="継続学習の初期値")
    t.add_argument("--lr", metavar="値", help="学習率の頂点。継続学習の既定は1e-4")
    t.add_argument("--warmup", type=int, metavar="N", help="学習率を上げきるまでのステップ数")
    t.add_argument("--device", metavar="名前", help="mps か cpu（既定 mps）")
    t.add_argument("--seed", type=int, default=0, metavar="N", help="乱数種")
    t.add_argument(
        "--arch",
        metavar="構成",
        help="ネットワークの構成。例 512x16x64（既定は crates/engine/build.rs の DEFAULT_ARCH）。"
        "入っているpy拡張の構成と違えば、この構成で作り直す",
    )
    t.add_argument("--notes", metavar="文", help="学習の記録へ書く備考")
    t.add_argument(
        "--positions",
        type=int,
        metavar="N",
        help="学習データの先頭のこの件数だけを使う。大きなpsvの一部をコピーなしで学習する",
    )
    t.add_argument(
        "--extra",
        metavar="引数",
        help="学習器へ素通しする追加引数。ハイフンで始まる値は --extra=--flag と書く",
    )
    t.add_argument("--force", action="store_true", help="同じ名前のネットがあっても最初から学習し直す")
    t.add_argument(
        "--adopt",
        action="store_true",
        help="条件の記録がないチェックポイントを、この条件の続きとして引き継ぐ",
    )
    t.set_defaults(func=train)

    t = ss.add_parser(
        "eval",
        help="ネットの検証損失を測る",
        description="学習は回さず、書き出したネットを並べて測る。"
        "**採否は対局で決める。** 検証損失は初期値の系列が違うだけで動く。",
    )
    t.add_argument("nets", nargs="+", metavar="ネット")
    t.add_argument(
        "--valid",
        metavar="PSV",
        help=f"検証集合。複数はコンマ区切り（既定 {paths.rel(config.train_valid())}）",
    )
    t.add_argument("--device", default="cpu", metavar="名前", help="cpu か mps")
    t.set_defaults(func=evaluate)

    t = ss.add_parser(
        "reorder",
        help="活性ダンプからFT出力次元の並べ替えを決める",
        description="第1層は4列チャンクのうち全ゼロのものを飛ばす。"
        "飛ばせる数は次元の並び順で変わるので、ゼロが同時に起きる次元を"
        "同じチャンクへ寄せる。評価値はビット一致したまま速度だけが上がる。",
    )
    t.add_argument("dump", metavar="ダンプ", help="活性ダンプ（*.bin）")
    t.add_argument(
        "activations",
        type=int,
        metavar="次元",
        help="片視点の活性次元。対の積を取る構成ではFTの幅の半分になる（主256なら128）",
    )
    t.add_argument("--out", metavar="ファイル", help="並べ替えの出力先")
    t.add_argument("--perm", metavar="ファイル", help="既存の並べ替えを当てて評価する")
    t.add_argument(
        "--apply",
        metavar="ネット",
        help="決めた並べ替えをこのネットへ当て、<名前>_reorder.hmwr を書く。"
        "ダンプを取ったときと同じ、並べ替え前の書き出しを渡す。"
        "当てたあと、固定深さのノード数と評価値が並べ替え前と一致することを確かめる",
    )
    t.add_argument("--arch", metavar="構成", help=ARCH_HELP)
    t.set_defaults(func=reorder)

    t = ss.add_parser(
        "actdump",
        help="活性ダンプを取る（並べ替えの材料）",
        description="第1層が飛ばす全ゼロチャンクの分布を集める。"
        "並べ替えの当たり方はダンプを取った局面の分布で変わるので、"
        "局面リストはリポジトリが持つ。"
        "**並べ替えを決めるなら、並べ替え前のネットを --eval-file で渡す。**"
        "既定の評価関数は並べ替え済みなので、そのまま使うと二重に当たる。",
    )
    t.add_argument("name", metavar="名前", help="出力名。data/profile/act-<名前>.bin")
    t.add_argument("--positions", metavar="パス", help="局面リスト")
    limit = t.add_mutually_exclusive_group()
    limit.add_argument("--depth", type=int, metavar="N", help="探索の深さ（既定 16）")
    limit.add_argument(
        "--nodes",
        type=int,
        metavar="N",
        help="深さの代わりに、1局面をこのノード数で打ち切る。"
        "大差の局面で深さの探索が終わらないときに使う",
    )
    t.add_argument("--stride", type=int, metavar="N", help="何回の評価につき1つ記録するか")
    t.add_argument("--eval-file", metavar="パス", help="評価関数")
    t.add_argument("--arch", metavar="構成", help=ARCH_HELP)
    t.set_defaults(func=actdump)

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
    """psvの局面数。1局面40バイト固定である。"""
    return psv.stat().st_size // BYTES_PER_POSITION


def arch_env(arch: str | None) -> dict[str, str]:
    """構成をcargoへ渡す環境変数。省かれたら空で、build.rsの既定が効く。

    ビルドを伴う操作が構成を受け取らないと、既定の構成で `target/release` を
    上書きしてしまう。先に `hmwr build engine --arch` で作っても消えるので、
    読み込みが `effect accumulator mismatch` で落ちる（#1025）。
    """
    return {"HIMAWARI_ARCH": paths.check_arch(arch)} if arch else {}


def default_arch() -> str:
    """既定の構成。正は `crates/engine/build.rs` の `DEFAULT_ARCH` にある。"""
    m = DEFAULT_ARCH_RE.search(BUILD_RS.read_text(encoding="utf-8"))
    if not m:
        raise proc.Fail(f"既定の構成を読めない: {paths.rel(BUILD_RS)} に DEFAULT_ARCH がない")
    return paths.check_arch(m.group(1))


EXTENSION_STAMP = paths.REPO / "target" / "py-kp" / "installed.json"


def _source_fingerprint() -> str:
    """crates/ の中身を表す文字列。コミット済みの木のハッシュに、未コミットの差分のハッシュを足す。"""
    tree = proc.capture(["git", "rev-parse", "HEAD:crates"]).strip()
    diff = proc.capture(["git", "diff", "HEAD", "--", "crates"])
    if not diff:
        return tree
    return f"{tree}+{hashlib.sha256(diff.encode('utf-8')).hexdigest()[:12]}"


def _stamped() -> dict:
    try:
        return json.loads(EXTENSION_STAMP.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}


def _ensure_extension(arch: str | None = None, *, dry_run: bool) -> None:
    """指定の構成のpy拡張を入れる。入っていないか、構成か crates/ の中身が違えば作り直す。

    archを省くと既定の構成になる。拡張の有無だけを見ると、別の構成で作った
    拡張のまま学習が進む（#804）。構成の名前だけを見ると、拡張のコードが変わっても
    古い拡張のまま学習が進む。入れたときの crates/ の中身を控えて比べる。
    """
    want = paths.check_arch(arch) if arch else default_arch()
    if dry_run:
        return
    got = proc.capture(["python3", "-c", "import himawari; print(himawari.ARCH)"]).strip()
    source = _source_fingerprint()
    if got == want and _stamped().get("source") == source:
        return
    if got and got != want:
        print(f"py拡張の構成が違う: {got}（学習する構成 {want}）。作り直す")
    elif got:
        print("py拡張を入れた後に crates/ が変わったので作り直す")
    else:
        print("py拡張が入っていないので作る")
    wheels = paths.REPO / "target" / "wheels"
    proc.run(
        [
            "maturin", "build", "--release", "--quiet",
            "-m", "crates/py/Cargo.toml",
            "--out", str(wheels),
        ],
        env={"HIMAWARI_ARCH": want, "CARGO_TARGET_DIR": "target/py-kp"},
    )
    _install_wheel(wheels, want, dry_run=False)
    EXTENSION_STAMP.parent.mkdir(parents=True, exist_ok=True)
    EXTENSION_STAMP.write_text(json.dumps({"arch": want, "source": source}) + "\n", encoding="utf-8")


def train(args: argparse.Namespace) -> int:
    """本番規模の学習。既定は過去の実験の結論に揃えてある。"""
    name = paths.check_name(args.name)
    data = _train_file(args.data, ".psv")
    valid = _train_file(args.valid, ".psv") if args.valid else config.train_valid()
    rank = _train_file(args.rank, ".rankpsv") if args.rank else None

    if not args.dry_run:
        for path, label in ((data, "学習データ"), (valid, "検証データ"), (rank, "兄弟群")):
            if path is not None and not path.is_file():
                raise proc.Fail(f"{label}がない: {path}")

    arch = paths.check_arch(args.arch) if args.arch else default_arch()
    _ensure_extension(arch, dry_run=args.dry_run)

    out = paths.NETS / f"{name}.hmwr"
    argv = [
        "python3",
        TRAINER,
        "--data", paths.rel(data),
        "--valid", paths.rel(valid),
        "--out", str(out),
        *BASE_FLAGS,
        # 29.9億のpsvはRAMに載らない
        "--mmap",
        # i8で格納するので制約なしだと書き出しが落ちる
        "--ft-clip", FT_CLIP,
        "--device", args.device or "mps",
        "--seed", str(args.seed),
        "--checkpoint-dir", str(paths.CHECKPOINTS / name),
        "--log-file", _curve(name),
        "--registry", paths.rel(paths.registry()),
        "--name", name,
        "--notes", args.notes or f"{paths.rel(data)} で学習",
    ]

    lr = args.lr
    if args.init_ckpt:
        if not Path(args.init_ckpt).is_file() and not args.dry_run:
            raise proc.Fail(f"初期値のチェックポイントがない: {args.init_ckpt}")
        argv += ["--init-checkpoint", args.init_ckpt]
        # 前世代の表現を壊さない幅。3e-4では壊れる
        lr = lr or "1e-4"
        argv += _continual_args(data, args, dry_run=args.dry_run)
    if lr:
        argv += ["--peak-lr", lr]
    if rank is not None:
        argv += ["--rank-data", paths.rel(rank)]
    if args.positions:
        argv += ["--positions", str(args.positions)]
    if args.extra:
        argv += args.extra.split()

    if not args.dry_run:
        for directory in (paths.NETS, paths.RUNS / CURVES, paths.CHECKPOINTS / name):
            directory.mkdir(parents=True, exist_ok=True)

    # 備考は学習の記録のメモで、学習の条件ではない
    at = argv.index("--notes")
    # 構成は学習器の引数に出ないので、条件として前に足す。既定のときは足さず、
    # --arch を入れる前の記録と同じ行を保つ
    shown_arch = {"HIMAWARI_ARCH": arch} if arch != default_arch() else None
    line = proc.show(argv[:at] + argv[at + 2 :], shown_arch)
    record = paths.CHECKPOINTS / name / "train.cond"
    latest = paths.CHECKPOINTS / name / "latest.ckpt"

    if out.is_file() and not args.force and not args.dry_run:
        if conditions.recorded(record) == conditions.normalize(line):
            print(f"済み: {paths.rel(out)}（同じ条件で学習済み。何もしない）")
            return proc.OK
        raise proc.Fail(
            f"同じ名前のネットが既にある: {paths.rel(out)}\n"
            "名前を変えるか、--force で最初から学習し直す"
        )
    resuming = latest.is_file() and not args.force
    conditions.check(
        record, line, resuming=resuming, adopt=args.adopt, dry_run=args.dry_run,
        what="チェックポイント",
    )  # fmt: skip

    print(f"=== 学習: {name} ===")
    print(f"構成      : {arch}")
    print(f"学習データ: {paths.rel(data)}")
    print(f"検証データ: {paths.rel(valid)}")
    print(f"出力      : {paths.rel(out)}")
    if args.init_ckpt:
        print(f"初期値    : {args.init_ckpt}（継続学習、学習率 {lr}）")
    if resuming:
        # 止まった学習の続き。エポック内の位置まで戻る
        print(f"再開      : {paths.rel(latest)}")
        argv += ["--resume", str(latest)]
    log = paths.log("train", name)
    with heartbeat.running(
        "train", name, dry_run=args.dry_run, unit="step", log=log,
        detail={"data": paths.rel(data), "valid_data": paths.rel(valid)},
    ) as beat:  # fmt: skip
        return proc.run(argv, dry_run=args.dry_run, log=log, on_line=train_progress(beat))


def train_progress(beat):
    """学習器の出力行を状態ファイルへ反映する関数を返す。"""

    def on_line(line: str) -> None:
        if m := STEP_RE.match(line):
            beat.update(int(m.group(1)), detail={"loss": float(m.group(2))})
        elif m := VALID_RE.match(line):
            valid = float(m.group(1))
            best = beat.data["detail"].get("best_valid")
            detail = {"valid": valid}
            if best is None or valid < best:
                detail["best_valid"] = valid
            beat.update(detail=detail, force=True)
        elif m := TOTAL_STEPS_RE.search(line):
            beat.set_total(int(m.group(1)))

    return on_line


def _curve(name: str) -> str:
    """学習曲線のTSV。学習器へはリポジトリからの相対パスで渡す。"""
    return paths.rel(paths.RUNS / CURVES / f"{name}.tsv")


def _train_file(value: str, suffix: str) -> Path:
    """名前を data/train/ のファイルへ解決する。区切りか拡張子があればパスとして通す。

    置き場は `paths.TRAIN` から組む。文字列で組むとテストの退避
    （tests/conftest.py）が効かず、本物の `data/train/` を読む（#850）。
    学習器へは `paths.rel` で相対にして渡す。学習の記録の備考に載るので、
    マシンの置き場に依存させない。
    """
    if "/" in value or value.endswith(suffix):
        return Path(value)
    return paths.TRAIN / f"{paths.check_name(value)}{suffix}"


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

    条件をまたいで比べたいときだけ検証集合を複数渡す。教師データの分布を
    変える実験では評価基準も一緒に動く。
    """
    if args.valid:
        valids = [Path(v) for v in args.valid.split(",") if v]
    else:
        valids = [config.train_valid()]
    if not args.dry_run:
        for path in valids:
            if not path.is_file():
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
                "--valid", paths.rel(valid), *BASE_FLAGS, "--device", args.device,
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


def reorder(args: argparse.Namespace) -> int:
    """活性ダンプから並べ替えを決める。--apply ならネットへ当てて検証まで行う。"""
    net = Path(args.apply) if args.apply else None
    env = arch_env(args.arch)
    out = args.out
    if net and not out:
        out = str(paths.PROFILE / f"perm_{net.name.split('.')[0]}.txt")
    if args.dry_run:
        print(f"[dry-run] 並べ替えを決める: {args.dump}（活性 {args.activations}）→ {out}")
        if net:
            print(f"[dry-run] {paths.rel(net)} へ当てて {paths.rel(reordered_path(net))} を書き、評価値の一致を確かめる")
        return proc.OK
    if not Path(args.dump).is_file():
        raise proc.Fail(f"活性ダンプがない: {args.dump}")
    if net and not net.is_file():
        raise proc.Fail(f"ネットがない: {args.apply}")
    argv = [args.dump, str(args.activations)]
    if out:
        argv += ["--out", out]
    if args.perm:
        argv += ["--perm", args.perm]
    code = ft_reorder.main(argv)
    if code != proc.OK or not net:
        return code
    return apply_reorder(net, Path(out), env=env)


def reordered_path(net: Path) -> Path:
    return paths.NETS / f"{net.name.split('.')[0]}_reorder.hmwr"


def apply_reorder(net: Path, perm: Path, *, env: dict[str, str] | None = None) -> int:
    """並べ替えをネットへ当て、評価値が変わっていないことを確かめる。

    並べ替えは評価値を変えないので、変わったら渡した書き出しか置換を
    取り違えている（以前、.hmwr と .hmwr.best を取り違えた）。

    envは構成を選ぶ環境変数で、makenet・エンジン・verifyの3つへ同じものを
    渡す。1つでも欠けると、その呼び出しが既定の構成でビルドし直して
    `target/release` を置き換える（#1025）。
    """
    from .build import cargo_build

    env = env or {}
    out = reordered_path(net)
    proc.run(
        # 来歴の文字列にパスが残るので、リポジトリからの相対で渡す
        proc.cargo_tool(
            "makenet",
            ["--reorder", paths.rel(perm), "--from", paths.rel(net), "--out", paths.rel(out)],
        ),
        env={"RUSTFLAGS": config.rustflags(), **env},
    )
    # actdumpは計測用のfeature付きでエンジンを作るので、通常のビルドへ戻してから測る
    cargo_build(dry_run=False, env=env, args=["-p", "himawari-usi", "--bin", "himawari"])
    engine = str(paths.release_bin("himawari"))
    tables = []
    for n in (net, out):
        text = proc.capture(
            proc.cargo_tool("verify", [engine, "--eval-file", str(n)]), env=env
        )
        tables.append([line for line in text.splitlines() if line.startswith("| ")])
    if not tables[0] or tables[0] != tables[1]:
        print("\n".join(tables[0]))
        print("\n".join(tables[1]))
        raise proc.Fail(
            f"並べ替えの前後で探索が一致しない: {paths.rel(net)} と {paths.rel(out)}。"
            "ダンプを取ったときの書き出しと、渡したネットが同じか確かめる"
        )
    print("\n".join(tables[0]))
    print(f"一致: 固定深さのノード数・評価値・最善手が並べ替えの前後で同じ（{len(tables[0]) - 1}局面）")
    print(f"書いた: {paths.rel(out)}")
    return proc.OK


# 活性ダンプの既定
ACTDUMP_POSITIONS = "openings/actdump_positions.txt"
ACTDUMP_DEPTH = 16
# actdump.rs の TARGET と揃える。片方だけ動かすと間引きの見積もりが狂う
ACTDUMP_TARGET = 12500
# 深さ16・終盤局面での実測は1局面あたり24万回だが、局面ごとに上下する。
# 見積もりが上振れすると目標数に届かずダンプが書かれないので、8割で見込む。
# 深さを変えたら --stride で渡す
ACTDUMP_EVALS_PER_POSITION = 190_000
# ノード数で打ち切るときの、1ノードあたりの評価回数の見込み。pool1_2ep の
# 深さ16のダンプは84局面・9,533万ノードで3,800万回の評価だった（0.40）。
# 上と同じく8割で見込む
ACTDUMP_EVALS_PER_NODE = 0.32


def actdump_go(depth: int | None, nodes: int | None) -> str:
    """1局面に送るgoのコマンド。ノード数を指定したらそちらで打ち切る。"""
    if nodes:
        return f"go nodes {nodes}"
    return f"go depth {depth or ACTDUMP_DEPTH}"


def actdump_stride(positions: int, nodes: int | None) -> int:
    """全局面をちょうど使い切る当たりの間引き。"""
    per_position = (
        int(nodes * ACTDUMP_EVALS_PER_NODE) if nodes else ACTDUMP_EVALS_PER_POSITION
    )
    return max(1, positions * per_position // ACTDUMP_TARGET)


def actdump(args: argparse.Namespace) -> int:
    """actdump付きのビルドで局面を探索させ、活性ダンプを書く。

    局面リストを固定するのがこのコマンドの主目的である。手打ちのUSI入力で
    取ると、どの局面で決めた並べ替えかが残らない。

    新しい並べ替えを決めるときは、並べ替え前のネットを渡す。既定の評価関数は
    並べ替え済みなので、そこで決めた置換を当てると二重適用になる。
    """
    name = paths.check_name(args.name)
    positions = Path(args.positions or paths.REPO / ACTDUMP_POSITIONS)
    out = paths.PROFILE / f"act-{name}.bin"
    go = actdump_go(args.depth, args.nodes)
    eval_file = args.eval_file or config.get("EVAL_FILE")
    env = arch_env(args.arch)

    if not positions.is_file() and not args.dry_run:
        raise proc.Fail(f"局面リストがない: {paths.rel(positions)}")

    print(f"=== 活性ダンプ: {name} ===")
    print(f"局面    : {paths.rel(positions)}")
    print(f"探索    : {go}")
    print(f"評価関数: {eval_file}")
    if args.arch:
        print(f"構成    : {args.arch}")
    print(f"出力    : {paths.rel(out)}")

    from .build import cargo_build

    cargo_build(
        dry_run=args.dry_run,
        env=env,
        args=[
            "-p", "himawari-usi", "--bin", "himawari",
            "--features", "himawari-engine/actdump",
        ],
    )
    if args.dry_run:
        print(f"[dry-run] 局面を探索させて {paths.rel(out)} を書く")
        return proc.OK

    lines = [
        line.strip()
        for line in positions.read_text().splitlines()
        if line.strip() and not line.startswith("#")
    ]
    if not lines:
        raise proc.Fail(f"局面リストが空だ: {paths.rel(positions)}")
    # 間引きが狭いと、リストの先頭だけで目標数に達してリストの残りが
    # 使われない。既定は「全局面をちょうど使い切る」当たりへ置く
    stride = args.stride or actdump_stride(len(lines), args.nodes)
    print(f"間引き  : {stride}（{len(lines)}局面ぶんの見積もり）")

    usi = ["usi", "setoption name Threads value 1",
           f"setoption name EvalFile value {eval_file}", "isready"]
    for position in lines:
        usi += ["usinewgame", f"position {position}", go]
    usi.append("quit")

    out.parent.mkdir(parents=True, exist_ok=True)
    engine = paths.release_bin("himawari")
    code = proc.run(
        [str(engine)],
        env={"HIMAWARI_ACT_OUT": str(out), "HIMAWARI_ACT_STRIDE": str(stride)},
        log=paths.log("actdump", name),
        stdin_text="\n".join(usi) + "\n",
    )
    if code == proc.OK and not out.is_file():
        raise proc.Fail(
            f"ダンプが書かれなかった: {paths.rel(out)}\n"
            f"目標サンプル数に届いていない。--stride を {stride} より小さくするか、"
            "局面か深さ（ノード数）を増やす"
        )
    return code


# 評価関数ファイルの形式。版によって次元の数が変わる。
# 版11・12は利きの特徴数と幅の2つを寸法の後ろに持つ
NDIMS_BY_VERSION = {2: 3, 3: 4, 5: 4, 7: 4, 9: 4, 4: 5, 6: 5, 8: 5, 10: 5, 11: 6, 12: 7}


def read_lineage(path: Path) -> str:
    """評価関数ファイルから学習来歴を読む。

    どのデータでどう学習したネットかを、配布物の説明に必ず載せる。
    ファイル自身が持っているので、手で書き写さない。
    """
    raw = path.read_bytes()
    version = int.from_bytes(raw[8:12], "little")
    ndims = NDIMS_BY_VERSION.get(version)
    if ndims is None:
        raise proc.Fail(f"未対応のフォーマット版: {version}（Himawari NNUE形式か確かめる）")
    length_at = 12 + ndims * 4
    length = int.from_bytes(raw[length_at : length_at + 4], "little")
    if not 0 < length <= 4096:
        raise proc.Fail("学習来歴の長さを読めない（Himawari NNUE形式か確かめる）")
    start = length_at + 4
    return raw[start : start + length].decode("utf-8", errors="replace")


def release(args: argparse.Namespace) -> int:
    """学習済みネットをGitHub Releaseとして配る。

    ネットはリポジトリで管理しないため、学習を回したマシンから直接作る。
    タグは net-v<番号> で、エンジン本体とは別系統になる。
    """
    path = Path(args.file)
    if not path.is_file():
        raise proc.Fail(f"ネットファイルがない: {path}")
    release_mod.check_version(args.version)

    tag = f"net-v{args.version}"
    release_mod.check_prereqs(tag, dry_run=args.dry_run)

    lineage = read_lineage(path)
    # アセットは固定名で配る。版はタグと学習来歴が持つ。
    # 学習時の名前はノートと題に残し、どの実験のネットかを辿れるようにする
    asset_name = config.NET_ASSET
    train_name = path.name.removesuffix(".best")
    size = release_mod.file_size(path)

    notes = [
        "## 学習来歴", "", "```", lineage, "```", "",
        "## ファイル", "",
        "| 項目 | 値 |", "|---|---|",
        f"| アセット | `{asset_name}` |",
        f"| 学習時の名前 | `{train_name}` |",
        f"| サイズ | {size} |",
        "| 形式 | Himawari NNUE |",
    ]
    if args.notes:
        notes += ["", "## 補足", "", args.notes]
    notes += [
        "", "## 使い方", "",
        f"`{asset_name}` を実行ファイルと同じフォルダに置く。USIオプション `EvalFile` が空なら、"
        "エンジンはそこを読む。更新は上書きコピーで済む。",
        "", "開発機へ学習時の名前で取るときは、次のようにする。", "", "```",
        f"gh release download {tag} -p '{asset_name}' -O data/nets/{train_name}",
        "```",
    ]

    print(f"タグ    : {tag}")
    print(f"アセット: {asset_name}（{size}、学習時の名前 {train_name}）")
    print(f"学習来歴: {lineage}")

    with tempfile.TemporaryDirectory() as tmp:
        asset = Path(tmp) / asset_name
        shutil.copy2(path, asset)
        return release_mod.create(
            tag,
            f"{tag}: {train_name}",
            "\n".join(notes) + "\n",
            [asset],
            apply=args.apply,
            dry_run=args.dry_run,
        )
