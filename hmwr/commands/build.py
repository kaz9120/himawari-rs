"""対局・計測・配布のためのビルド。

手で組み立てると条件がぶれる。実際に起きた事故が3つある。片側だけ
`-C target-cpu=native` を付け忘れて最適化条件の違う2本を比べたこと、
退避した変更の戻し忘れで変更前後を取り違えたこと、競合を解決しないまま
中途半端な木からビルドしたことである（ADR-0081）。手順をここに固定する。
"""

from __future__ import annotations

import argparse
import filecmp
import re
import shutil
from pathlib import Path

from .. import config, paths, proc

ARCH_RE = re.compile(r"^\d+x\d+(x\d+){0,2}$")
ENGINE = "target/release/himawari"


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("build", help="エンジンをビルドする")
    ss = p.add_subparsers(dest="sub", metavar="<種類>")

    t = ss.add_parser(
        "pair",
        help="比較用の2本を同じ条件で作る",
        description="比較元（既定 origin/main）と現在のHEADから、"
        "data/bin/base-<名前> と cand-<名前> を作る。"
        "未コミットの変更があると中断する。どちらを候補にしたか"
        "後から辿れなくなるためである。",
    )
    t.add_argument("name", help="実験名")
    t.add_argument("--baseline", metavar="REF", default="origin/main", help="比較元のref")
    t.set_defaults(func=pair)

    t = ss.add_parser(
        "pgo",
        help="配布・対局用の単体ビルドを作る",
        description="計測用ビルド・学習走行・最適化ビルドの3段で作る。"
        "NPSが+10%前後上がる。比較用のペアには使わない。"
        "両側を同条件（最適化なし）で作るほうが公平である。",
    )
    t.add_argument("--out", metavar="パス", help="出力先（既定 data/bin/himawari-pgo）")
    t.add_argument("--depth", type=int, default=22, metavar="N", help="学習走行の深さ")
    t.set_defaults(func=pgo)

    t = ss.add_parser(
        "engine",
        help="エンジン本体をビルドする",
        description="計測と同じフラグでビルドする。",
    )
    t.add_argument("--arch", metavar="構成", help="例 512x16x64。省くと既定構成")
    t.add_argument("--halfka", action="store_true", help="入力をHalfKA（相手玉の平面つき）で作る")
    t.set_defaults(func=engine)

    t = ss.add_parser(
        "shapes",
        help="ネットワーク構成ごとにエンジンと評価ファイルを作る",
        description="構成を変えるとエンジンと評価ファイルの次元が同時に変わる。"
        "片方だけ作り直すと読み込みに失敗するので、対で作る。"
        "--from を付けると元の評価関数を各構成の次元へ合わせる。"
        "広げる向きは評価値が元と一致するので、速度の差だけを取り出せる。",
    )
    t.add_argument("specs", nargs="+", metavar="構成", help="例 256x16 512x16x32")
    t.add_argument("--from", dest="source", metavar="ネット", help="元の評価関数")
    t.add_argument("--tag", metavar="名前", help="出力名の頭（既定 shape）")
    t.add_argument("--halfka", action="store_true", help="入力をHalfKA（相手玉の平面つき）で作る")
    t.set_defaults(func=shapes)


# --- 共通 --------------------------------------------------------------


def require_clean_crates() -> None:
    """crates/ に未コミットの変更がないことを確かめる。

    あるままビルドすると、どのコードから作ったバイナリか後から辿れない。
    """
    dirty = not proc.succeeds(["git", "diff", "--quiet", "--", "crates/"])
    staged = not proc.succeeds(["git", "diff", "--cached", "--quiet", "--", "crates/"])
    if dirty or staged:
        print(proc.git("status", "--short", "--", "crates/"))
        raise proc.Fail("crates/ に未コミットの変更がある。コミットしてから実行する")


def cargo_build(
    extra_rustflags: str = "",
    *,
    dry_run: bool,
    env: dict[str, str] | None = None,
    args: list[str] | None = None,
) -> None:
    flags = config.rustflags()
    if extra_rustflags:
        flags = f"{flags} {extra_rustflags}"
    full = {"RUSTFLAGS": flags, **(env or {})}
    proc.run(
        ["cargo", "build", "--release", "--quiet", *(args or [])],
        dry_run=dry_run,
        env=full,
    )


def _copy(src: Path, dst: Path, *, dry_run: bool) -> None:
    if dry_run:
        print(f"[dry-run] cp {paths.rel(src)} {paths.rel(dst)}")
        return
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(src, dst)


# --- pair --------------------------------------------------------------


def pair(args: argparse.Namespace) -> int:
    return make_pair(
        args.name, baseline=args.baseline or "origin/main", dry_run=args.dry_run
    )


def make_pair(name: str, *, baseline: str, dry_run: bool) -> int:
    """比較用の2本を同じ条件で作る。

    比較元は crates/ だけを差し替えて作る。作業コピーを別に切ると
    ビルドキャッシュが分かれて遅くなるためである。

    2本が同一になったら終了コード1を返す。探索に差がない可能性がある。
    """
    name = paths.check_name(name)
    base_out = paths.BIN / f"base-{name}"
    cand_out = paths.BIN / f"cand-{name}"

    if not dry_run:
        require_clean_crates()
        if not proc.succeeds(["git", "rev-parse", "--verify", "--quiet", baseline]):
            raise proc.Fail(f"比較元のrefが見つからない: {baseline}")

    print(f"=== 比較用バイナリの作成: {name} ===")
    if not dry_run:
        print(f"candidate: {proc.git('rev-parse', '--short', 'HEAD')} （現在のHEAD）")
        print(
            f"baseline : {proc.git('rev-parse', '--short', baseline)} （{baseline}）"
        )

    print("candidateをビルド中...")
    cargo_build(dry_run=dry_run)
    _copy(paths.REPO / ENGINE, cand_out, dry_run=dry_run)

    print("baselineをビルド中...")
    proc.run(["git", "checkout", baseline, "--", "crates/"], dry_run=dry_run)
    try:
        cargo_build(dry_run=dry_run)
        _copy(paths.REPO / ENGINE, base_out, dry_run=dry_run)
    finally:
        # 失敗しても作業木を必ず戻す
        proc.run(["git", "checkout", "HEAD", "--", "crates/"], dry_run=dry_run)

    if dry_run:
        return proc.OK

    if filecmp.cmp(base_out, cand_out, shallow=False):
        print()
        print("2本が同一のバイナリになった。探索に差がない可能性がある。")
        print("機能検証で確かめること。")
        return proc.JUDGE

    print()
    print(f"できた: {paths.rel(base_out)} / {paths.rel(cand_out)}")
    print(f"次の手順: hmwr sprt run {name}")
    return proc.OK


# --- pgo ---------------------------------------------------------------


def pgo(args: argparse.Namespace) -> int:
    """計測付きビルド・学習走行・最適化ビルドの3段で作る。"""
    out = Path(args.out) if args.out else paths.BIN / "himawari-pgo"
    if not out.is_absolute():
        out = paths.REPO / out
    pgo_dir = paths.REPO / "target" / "pgo"

    if not args.dry_run:
        require_clean_crates()
    profdata = _find_profdata(dry_run=args.dry_run)

    if not args.dry_run:
        shutil.rmtree(pgo_dir, ignore_errors=True)
        (pgo_dir / "raw").mkdir(parents=True, exist_ok=True)

    print(f"=== 最適化ビルド → {paths.rel(out)} ===")

    print("1/3: 計測用ビルド")
    cargo_build(f"-C profile-generate={pgo_dir}/raw", dry_run=args.dry_run)

    print(f"2/3: 学習走行（ベンチ4局面、深さ{args.depth}）")
    instr = pgo_dir / "himawari-instr"
    _copy(paths.REPO / ENGINE, instr, dry_run=args.dry_run)
    proc.run(
        [
            str(paths.REPO / "target" / "release" / "bench"),
            str(instr),
            "--depth",
            str(args.depth),
            "--runs",
            "1",
        ],
        dry_run=args.dry_run,
        env=config.measure_env(),
    )
    raws = sorted(str(p) for p in (pgo_dir / "raw").glob("*.profraw"))
    if not raws and not args.dry_run:
        raise proc.Fail("計測データが出ていない。学習走行が失敗している")
    merged = pgo_dir / "merged.profdata"
    proc.run(
        [profdata, "merge", "-o", str(merged), *(raws or [f"{pgo_dir}/raw/*.profraw"])],
        dry_run=args.dry_run,
    )

    print("3/3: 最適化ビルド")
    cargo_build(f"-C profile-use={merged}", dry_run=args.dry_run)
    _copy(paths.REPO / ENGINE, out, dry_run=args.dry_run)

    print()
    print(f"できた: {paths.rel(out)}")
    print("target には最適化版が残る。素のビルドに戻すには hmwr build engine")
    return proc.OK


def _find_profdata(*, dry_run: bool) -> str:
    """ツールチェインに同梱の llvm-profdata を探す。"""
    sysroot = proc.capture(["rustc", "--print", "sysroot"]).strip()
    if sysroot:
        for path in Path(sysroot, "lib", "rustlib").rglob("llvm-profdata"):
            if path.is_file():
                return str(path)
    if dry_run:
        return "llvm-profdata"
    raise proc.Fail("llvm-profdata がない。rustup component add llvm-tools で入れる")


# --- engine / shapes ---------------------------------------------------


def engine(args: argparse.Namespace) -> int:
    env = {"HIMAWARI_ARCH": args.arch} if args.arch else {}
    extra = ["--features", "himawari-usi/halfka"] if args.halfka else []
    cargo_build(dry_run=args.dry_run, env=env, args=["-p", "himawari-usi", *extra])
    return proc.OK


def shapes(args: argparse.Namespace) -> int:
    """構成ごとにエンジンと評価ファイルを対で作る。

    構成を変えるとエンジンと評価ファイルの次元が同時に変わる。片方だけ
    作り直すと読み込みに失敗するので、対で作るところまでを1つにする。
    """
    source = args.source
    tag = args.tag or ("exp" if source else "shape")
    if source and not Path(source).is_file() and not args.dry_run:
        raise proc.Fail(f"元の評価関数がない: {source}")

    for spec in args.specs:
        if not ARCH_RE.match(spec):
            raise proc.Fail(
                f"構成の書き方が違う: {spec}（<FT>x<L1>[x<L2>[x<L3>]]）", proc.USAGE
            )

    print(f"=== 構成ごとのビルド（{len(args.specs)}件、名前 {tag}） ===")
    if source:
        print(f"元の評価関数: {source}（各構成へ合わせる）")

    for spec in args.specs:
        # 構成ごとに出力先を分ける。1つを使い回すと構成を変えるたびに
        # 全体が再コンパイルされる。halfkaは次元が違うので別の出力先にする
        shape_key = f"ka-{spec}" if args.halfka else spec
        target = paths.REPO / "target" / "shape" / shape_key
        binary = paths.BIN / f"shape-{shape_key}"
        net = paths.NETS / f"{tag}-{shape_key}.hmwr"

        print(f"ビルド: {shape_key}")
        feature_args = (
            ["--features", "himawari-usi/halfka,himawari-tools/halfka"]
            if args.halfka
            else []
        )
        cargo_build(
            dry_run=args.dry_run,
            env={"HIMAWARI_ARCH": spec, "CARGO_TARGET_DIR": str(target)},
            args=[
                "-p", "himawari-usi",
                "-p", "himawari-tools",
                "--bin", "himawari",
                "--bin", "makenet",
                *feature_args,
            ],
        )
        _copy(target / "release" / "himawari", binary, dry_run=args.dry_run)

        print(f"評価ファイル: {paths.rel(net)}")
        makenet = [str(target / "release" / "makenet")]
        makenet += ["--resize", source] if source else ["--seed", "1"]
        makenet += ["--out", str(net)]
        net.parent.mkdir(parents=True, exist_ok=True)
        proc.run(makenet, dry_run=args.dry_run)

    prefix = "ka-" if args.halfka else ""
    pairs = " ".join(
        f"data/bin/shape-{prefix}{s}=data/nets/{tag}-{prefix}{s}.hmwr" for s in args.specs
    )
    print()
    print(f"NPSを測る: hmwr bench --runs 5 {pairs}")
    return proc.OK
