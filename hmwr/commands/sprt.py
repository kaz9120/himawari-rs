"""対局ゲート（SPRT）の入口。`match` の別名として残している（ADR-0208）。

対局の実体は `match.py` にある。ここが持つのは3つの段取りだけである。
`sprt run` はペアの作成・機能検証・起動を順に行い、`sprt net` はビルドを
固定して評価関数だけを差し替える。どちらも `match run` の対局者の
組み立て方の違いで、移行が済んだら消す。`sprt extend` は上限で見送りに
なった実行を、控えた条件のまま延ばす（ADR-0217の追記）。
"""

from __future__ import annotations

import argparse
from pathlib import Path

from .. import conditions, config, paths, proc
from . import build, match, queue
from .match import Player, Spec, files, settings

# 延長の刻みと、オーナーに諮る線（ADR-0217の追記）
EXTEND_STEP = int(config.SPRT_MAX_PAIRS)
OWNER_LINE = 30_000

def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("sprt", help="対局で棋力を検定する")
    ss = p.add_subparsers(dest="sub", metavar="<操作>")

    t = ss.add_parser(
        "run",
        help="ペアを作り、機能検証を通してから起動する",
        description="ビルド・機能検証・起動を順に行う。判定が出るまで走り、"
        "落ちても棋譜から再開する。すでに判定済みなら結果を返して終わる。",
        epilog=match.VERDICT_EPILOG,
    )
    t.add_argument("name", help="実験名")
    t.add_argument("--baseline", metavar="REF", help="比較元のref（既定 origin/main）")
    t.add_argument(
        "--noninferiority",
        action="store_true",
        help="非劣性で測る（elo0=-5、elo1=0）",
    )
    t.add_argument("--tc", metavar="持ち時間", help="例 60+0.6（既定 10+0.1）")
    t.add_argument(
        "--concurrency",
        type=int,
        metavar="N",
        help="同時に指す局数（既定は物理コア数から決まる）。多スレッドの対局で下げる",
    )
    t.add_argument(
        "--set",
        action="append",
        metavar="KEY=VALUE",
        help="測定条件を直接渡す（繰り返し可）",
    )
    t.add_argument(
        "--max-pairs",
        type=int,
        metavar="N",
        help="このペア数で打ち切る。H1だけを採択するゲート用途の見送り上限",
    )
    t.add_argument(
        "--no-verify",
        dest="verify",
        action="store_false",
        help="機能検証を飛ばす。終盤にしか出ない機能を測るときだけ使う",
    )
    t.add_argument(
        "--foreground",
        action="store_true",
        help="切り離さずその場で走らせる。判定まで戻らない",
    )
    # 切り離した子プロセスが自分を呼び直すための内部フラグ
    t.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    t.set_defaults(func=run, verify=True)

    t = ss.add_parser(
        "net",
        help="ビルドを固定し、評価関数だけを差し替えて測る",
        description="同じビルドで対局し、評価関数を片側ずつ指定する。"
        "ネットとビルドの次元は揃える。",
        epilog=match.VERDICT_EPILOG,
    )
    t.add_argument("base", metavar="baselineネット")
    t.add_argument("cand", metavar="candidateネット")
    t.add_argument("name", help="実験名")
    t.add_argument("--bin", metavar="パス", help="対局に使うビルド")
    t.add_argument(
        "--cand-bin",
        metavar="パス",
        help="candidate側だけ別のビルドを使う。入力特徴の違う構成の比較用",
    )
    t.add_argument("--noninferiority", action="store_true", help="非劣性で測る")
    t.add_argument("--tc", metavar="持ち時間", help="例 60+0.6")
    t.add_argument(
        "--concurrency",
        type=int,
        metavar="N",
        help="同時に指す局数（既定は物理コア数から決まる）。多スレッドの対局で下げる",
    )
    t.add_argument("--set", action="append", metavar="KEY=VALUE", help="測定条件")
    t.add_argument(
        "--max-pairs",
        type=int,
        metavar="N",
        help="このペア数で打ち切る。H1だけを採択するゲート用途の見送り上限",
    )
    t.add_argument("--foreground", action="store_true", help="切り離さず走らせる")
    t.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    t.set_defaults(func=run_net)

    t = ss.add_parser(
        "extend",
        help="上限で見送りになった実行を、同じ条件で延ばす",
        description="点推定が中点より上の見送りだけを延ばす。"
        "結果ファイルを .result.capped-<ペア数> へ退避し、控えた条件（.cond）と"
        "同じ対局を棋譜の続きから起動する。ビルドは作り直さない。",
        epilog=match.VERDICT_EPILOG,
    )
    t.add_argument("name", help="実験名")
    t.add_argument(
        "--max-pairs",
        type=int,
        metavar="N",
        help=f"次の上限（既定は今のペア数に{EXTEND_STEP}を足した数）",
    )
    t.add_argument(
        "--owner-approved",
        action="store_true",
        help=f"{OWNER_LINE}ペアを超えて延ばすことを、オーナーに諮って承認を得た",
    )
    t.add_argument("--foreground", action="store_true", help="切り離さず走らせる")
    t.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    t.set_defaults(func=extend)

    match.add_show_wait(ss)


# --- 起動 --------------------------------------------------------------


def run(args: argparse.Namespace) -> int:
    """ビルド・機能検証・起動を順に行う。

    3つを別々に叩けると順番を飛ばせてしまう。機能検証を飛ばすと、探索に
    影響のない変更へ対局リソースを払うことになる（ADR-0074）。ここで
    順番を固定し、飛ばすには明示を求める。
    """
    f = files(args.name)
    spec = Spec(
        args.name, Player(str(f["base"])), Player(str(f["cand"])), env=settings(args)
    )

    if args.worker:
        # 切り離された子。判定が出るまで回す
        return match.until_decision(spec, dry_run=False)

    if match.decided(args.name):
        return proc.OK
    queue.guard("SPRT", dry_run=args.dry_run)

    code = build.make_pair(
        args.name,
        baseline=args.baseline or "origin/main",
        dry_run=args.dry_run,
    )
    if code == proc.JUDGE:
        # 2本が同一だった。対局しても差は出ない
        return proc.JUDGE

    if args.verify and not _verified(args):
        return proc.JUDGE

    return match.start(args, spec, ["sprt", "run", args.name])


def run_net(args: argparse.Namespace) -> int:
    """評価関数だけを差し替えて測る。ビルドは同じものを両側に使う。"""
    engine = args.bin or _default_bin(args.name)
    spec = Spec(
        args.name,
        Player(engine, _resolve(args.base)),
        Player(args.cand_bin or engine, _resolve(args.cand)),
        env=settings(args),
    )

    if args.worker:
        return match.until_decision(spec, dry_run=False)

    if match.decided(args.name):
        return proc.OK
    queue.guard("SPRT", dry_run=args.dry_run)

    for net in (args.base, args.cand):
        if not Path(net).is_file() and not args.dry_run:
            raise proc.Fail(f"ネットがない: {net}")

    rest = ["sprt", "net", args.base, args.cand, args.name]
    if args.cand_bin:
        rest += ["--cand-bin", args.cand_bin]
    if args.bin:
        rest += ["--bin", args.bin]
    return match.start(args, spec, rest)


# --- 延長 --------------------------------------------------------------


def extend(args: argparse.Namespace) -> int:
    """見送りの実行を延ばす（ADR-0217の追記）。

    退避と再開を1コマンドにする。手で改名すると退避の名前が揺れ、改名を
    忘れると `sprt run` が判定済みとして即座に返る（issue #782）。
    `sprt run` で再開するとペアを作り直すので、mainが進んでいれば比較元が
    変わる。ここでは控えた条件から対局者を組み立て直し、ビルドに触らない。
    """
    f = files(args.name)
    if args.worker:
        # 切り離された子。退避は親が済ませている
        return match.until_decision(_spec_from_conditions(args.name, args.max_pairs), dry_run=False)

    result = _read_result(f["result"])
    if result.get("decision") != "見送り":
        raise proc.Fail(
            f"見送りではない（{result.get('decision', '不明')}）。延ばすのは上限で見送りになった実行だけ"
        )
    pairs = int(result["games"]) // 2
    elo = float(result["elo"])
    mid = (float(result["elo0"]) + float(result["elo1"])) / 2
    print(f"見送り: {pairs}ペア、Elo {result['elo']}（中点 {mid:+.1f}）、LLR {result.get('llr', '?')}")
    if elo <= mid:
        print("点推定が中点以下なので延ばさない。見送りで確定し、ADRへ記録する（ADR-0217の追記）")
        return proc.JUDGE

    cap = args.max_pairs or pairs + EXTEND_STEP
    if cap <= pairs:
        raise proc.Fail(f"--max-pairs は今のペア数（{pairs}）より大きくする: {cap}", proc.USAGE)
    if cap > OWNER_LINE and not args.owner_approved:
        raise proc.Fail(
            f"{OWNER_LINE}ペアを超えて延ばすときはオーナーに諮る（ADR-0217の追記）。"
            "承認を得たら --owner-approved を付ける",
            proc.USAGE,
        )

    spec = _spec_from_conditions(args.name, cap)
    if not args.dry_run:
        for path in (spec.base.build, spec.cand.build, spec.base.net, spec.cand.net, spec.openings):
            if path and not Path(path).is_file():
                raise proc.Fail(f"ファイルがない: {paths.rel(path)}（hmwr clean が消した可能性がある）")
    queue.guard("SPRT", dry_run=args.dry_run)

    archive = f["result"].with_name(f"{f['result'].name}.capped-{pairs}")
    if archive.exists():
        raise proc.Fail(f"退避先がすでにある: {paths.rel(archive)}")
    if args.dry_run:
        print(f"[dry-run] 退避 {paths.rel(f['result'])} → {paths.rel(archive)}")
    else:
        f["result"].rename(archive)
        print(f"退避した: {paths.rel(archive)}")
    print(f"上限を {cap} ペアへ延ばして続ける")
    rest = ["sprt", "extend", args.name, "--max-pairs", str(cap)]
    return match.start(args, spec, rest, pass_env=False)


def _read_result(result: Path) -> dict[str, str]:
    """結果ファイルを鍵と値の組で読む。"""
    if not result.is_file():
        raise proc.Fail(f"結果がない: {paths.rel(result)}（走っている途中か、まだ起動していない）")
    fields = {}
    for line in result.read_text(encoding="utf-8").splitlines():
        key, sep, value = line.partition("=")
        if sep:
            fields[key] = value
    missing = [k for k in ("games", "elo", "elo0", "elo1") if k not in fields]
    if missing:
        raise proc.Fail(f"結果ファイルに {', '.join(missing)} がない: {paths.rel(result)}")
    return fields


# 条件の記録にある、測定条件の鍵へ畳むフラグ
_ENV_FLAGS = {
    "--tc": "SPRT_TC",
    "--concurrency": "SPRT_CONCURRENCY",
    "--adjudicate": "SPRT_ADJUDICATE",
    "--elo0": "SPRT_ELO0",
    "--elo1": "SPRT_ELO1",
    "--alpha": "SPRT_ALPHA",
    "--beta": "SPRT_BETA",
}


def _resolve(value: str) -> str:
    """控えた相対パスを絶対パスへ戻す。控えるときに置き場の根を外している。"""
    path = Path(value)
    if path.is_absolute():
        return value
    work = paths.WORK / path
    return str(work if work.exists() else paths.REPO / path)


def _spec_from_conditions(name: str, max_pairs: int) -> Spec:
    """控えた条件（.cond）から対局の条件を組み立て直す。

    `sprt run`・`sprt net`・`match run` のどれで起動したかを問わない。組み立てた
    結果を同じ書式に戻し、控えた行と一致することを確かめる。一致しなければ
    延ばさない。違う条件の対局を同じ棋譜へ積まないためである。
    """
    f = files(name)
    line = conditions.recorded(f["cond"])
    if line is None:
        raise proc.Fail(f"条件の記録がない: {paths.rel(f['cond'])}。延ばせない")

    base = cand = base_net = cand_net = ""
    base_odds = cand_odds = openings = hash_mb = max_moves = ""
    opts: list[str] = []
    base_opts: list[str] = []
    cand_opts: list[str] = []
    env = {"SPRT_MAX_PAIRS": str(max_pairs)}
    words = iter(line.split()[1:])
    for flag in words:
        if flag == "--no-stop":
            raise proc.Fail("指し切りの対局は延ばさない。延ばすのはSPRTの見送りだけ")
        value = next(words, None)
        if value is None:
            raise proc.Fail(f"条件の記録を読めない（{flag} に値がない）: {paths.rel(f['cond'])}")
        evalfile = value.removeprefix("EvalFile=") if value.startswith("EvalFile=") else None
        if flag == "--baseline":
            base = _resolve(value)
        elif flag == "--candidate":
            cand = _resolve(value)
        elif flag == "--bopt" and evalfile is not None and not base_net:
            base_net = _resolve(evalfile)
        elif flag == "--copt" and evalfile is not None and not cand_net:
            cand_net = _resolve(evalfile)
        elif flag == "--bopt":
            base_opts.append(value)
        elif flag == "--copt":
            cand_opts.append(value)
        elif flag == "--option" and evalfile is not None:
            env["EVAL_FILE"] = _resolve(evalfile)
        elif flag == "--option":
            opts.append(value)
        elif flag == "--bodds":
            base_odds = value
        elif flag == "--codds":
            cand_odds = value
        elif flag == "--openings":
            openings = _resolve(value)
        elif flag == "--hash":
            hash_mb = value
        elif flag == "--max-moves":
            max_moves = value
        elif flag in _ENV_FLAGS:
            env[_ENV_FLAGS[flag]] = value
        else:
            raise proc.Fail(f"条件の記録に知らないフラグがある: {flag}（{paths.rel(f['cond'])}）")

    spec = Spec(
        name,
        Player(base, base_net, tuple(base_opts), base_odds),
        Player(cand, cand_net, tuple(cand_opts), cand_odds),
        opts=tuple(opts),
        openings=openings,
        hash_mb=hash_mb,
        max_moves=max_moves,
        env=env,
    )
    rebuilt = conditions.normalize(proc.show(match.play_command(spec)))
    if rebuilt != line:
        raise proc.Fail(
            f"控えた条件を組み立て直せない: {paths.rel(f['cond'])}\n"
            f"控え: {line}\n組み立て: {rebuilt}"
        )
    return spec


def _verified(args: argparse.Namespace) -> bool:
    """機能検証を通す。全局面で一致したらFalseを返し、起動を止める。"""
    f = files(args.name)
    code = proc.run(
        proc.cargo_tool("verify", [str(f["base"]), str(f["cand"])]),
        dry_run=args.dry_run,
        env=config.measure_env(),
        log=paths.log("verify", args.name),
        allowed=(proc.OK, proc.JUDGE),
    )
    if code != proc.JUDGE:
        return True
    print()
    print("全局面でノード数が一致した。この変更は探索に影響していない。")
    print("対局にかけても中立にしかならないので起動しない。")
    print("終盤にしか出ない機能なら、終盤局面を別に用意して測り直す。")
    print("それでも走らせるなら --no-verify を付ける。")
    return False


def _default_bin(name: str) -> str:
    """対局に使うビルド。実験ごとに固定したいので data/bin を先に見る。"""
    fixed = paths.BIN / f"base-{name}"
    if fixed.is_file():
        return str(fixed)
    return str(paths.release_bin("himawari"))
