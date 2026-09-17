"""対局の入口。2者を対局させ、決めた規則で止める（ADR-0208）。

`sprt run`・`sprt net`・`selfplay` の直接実行は、どれも「2者を対局させ、
ある規則で止める」の変種だった。対局者をビルド・評価関数・USIオプション・
時間オッズの組で表し、止める規則をSPRTの判定か固定ペア数から選ぶ。

完了は結果ファイルの有無で決まる。プロセスの生死やセッションの継続に
依存しない設計は ADR-0175 にある。

起動したプロセスは新しいセッションへ移す。エージェントがツールの
バックグラウンド実行のまま走らせると、数十分で回収されて止まるためである
（2026-08-18に2回。標準出力を捨てても起きたので出力量とは無関係）。
**切り離してよいのは、状態がファイルにあるからである。**
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
import time
from dataclasses import dataclass, field, replace
from pathlib import Path

from .. import conditions, config, paths, proc, sprt_log

# 異常終了からの再開を数える上限。判定に至らないまま無限に試し続けない
MAX_RETRY = 20
RETRY_WAIT = 5


@dataclass(frozen=True)
class Player:
    """対局者。ビルド・評価関数・USIオプション・時間オッズの組。"""

    build: str  # エンジンのパス
    net: str = ""  # 評価関数のパス。空なら現行の評価関数
    opts: tuple[str, ...] = ()  # この側だけに渡すUSIオプション（Name=Value）
    odds: str = ""  # 持ち時間の倍率


@dataclass(frozen=True)
class Spec:
    """1本の対局の条件。"""

    name: str
    base: Player
    cand: Player
    opts: tuple[str, ...] = ()  # 両側に渡すUSIオプション
    stop_pairs: int = 0  # 0ならSPRTの判定で止める。正ならそのペア数を指し切る
    openings: str = ""
    hash_mb: str = ""
    max_moves: str = ""
    env: dict[str, str] = field(default_factory=dict)
    adopt: bool = False  # 条件の記録がない棋譜を、この条件のものとして引き継ぐ


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("match", help="2者を対局させて強さを測る")
    ss = p.add_subparsers(dest="sub", metavar="<操作>")

    t = ss.add_parser(
        "run",
        help="対局を起動する",
        description="対局者はビルド・評価関数・USIオプション・時間オッズの組で表す。"
        "省いた項目は両側とも既定になる。ビルドを省くと data/bin/base-<名前> と "
        "cand-<名前>（hmwr build pair の出力）を使い、無ければ target/release/himawari "
        "を両側に使う。判定か指し切りまで走り、落ちても棋譜から再開する。"
        "すでに結果があれば、それを返して終わる。",
    )
    t.add_argument("name", help="実験名")
    t.add_argument("--build", metavar="名前", help="両側のビルド。data/bin/<名前>")
    t.add_argument("--base-build", metavar="名前", help="baseline側のビルド")
    t.add_argument("--cand-build", metavar="名前", help="candidate側のビルド")
    t.add_argument("--base-net", metavar="名前", help="baseline側の評価関数。data/nets/<名前>.hmwr")
    t.add_argument("--cand-net", metavar="名前", help="candidate側の評価関数")
    t.add_argument(
        "--opt", action="append", default=[], metavar="Name=Value", help="両側のUSIオプション"
    )
    t.add_argument("--base-opt", action="append", default=[], metavar="Name=Value")
    t.add_argument("--cand-opt", action="append", default=[], metavar="Name=Value")
    t.add_argument("--base-odds", metavar="倍率", help="baseline側の持ち時間の倍率")
    t.add_argument("--cand-odds", metavar="倍率", help="candidate側の持ち時間の倍率")
    t.add_argument(
        "--stop",
        default="sprt",
        metavar="規則",
        help="sprt（判定で止める。既定）か pairs:N（Nペアを指し切ってEloを推定する）",
    )
    t.add_argument("--openings", metavar="名前", help="開始局面集。openings/<名前>.txt")
    t.add_argument("--tc", metavar="持ち時間", help="例 60+0.6（既定 10+0.1）")
    t.add_argument("--hash", metavar="MB", help=f"置換表（既定 {config.MATCH_HASH}）")
    t.add_argument(
        "--max-moves", metavar="N", help=f"引き分けにする手数（既定 {config.MATCH_MAX_MOVES}）"
    )
    t.add_argument(
        "--noninferiority", action="store_true", help="非劣性で測る（elo0=-5、elo1=0）"
    )
    t.add_argument(
        "--set", action="append", metavar="KEY=VALUE", help="測定条件を直接渡す（繰り返し可）"
    )
    t.add_argument(
        "--max-pairs",
        type=int,
        metavar="N",
        help="--stop sprt の見送り上限。H1だけを採択するゲート用途",
    )
    t.add_argument(
        "--adopt",
        action="store_true",
        help="条件の記録がない棋譜を、この条件の続きとして引き継ぐ。"
        "条件が同じであることは、ログの起動行で確かめてから付ける",
    )
    t.add_argument(
        "--foreground", action="store_true", help="切り離さずその場で走らせる。終わるまで戻らない"
    )
    # 切り離した子プロセスが自分を呼び直すための内部フラグ
    t.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    t.set_defaults(func=run)

    add_show_wait(ss)


def add_show_wait(ss: argparse._SubParsersAction) -> None:
    t = ss.add_parser("show", help="途中経過や結果を出す。名前を省くと一覧")
    t.add_argument("name", nargs="?", help="実験名")
    t.add_argument("--all", action="store_true", help="完了した走行も並べる")
    t.set_defaults(func=show)

    t = ss.add_parser("wait", help="結果が出るまで待つ")
    t.add_argument("name", help="実験名")
    t.add_argument("--interval", type=int, default=60, metavar="秒", help="確認の間隔")
    t.set_defaults(func=wait)


def files(name: str) -> dict[str, Path]:
    """この名前で決まる置き場をまとめて返す。"""
    paths.check_name(name)
    return {
        "base": paths.BIN / f"base-{name}",
        "cand": paths.BIN / f"cand-{name}",
        "jsonl": paths.SPRT / f"{name}.jsonl",
        "result": paths.SPRT / f"{name}.result",
        "cond": paths.SPRT / f"{name}.cond",
        "log": paths.log("sprt", name),
    }


# --- 名前の解決 --------------------------------------------------------


def _named(value: str, root: Path, suffix: str) -> str:
    """名前を置き場のパスへ解決する。区切りを含むものはパスとして通す。

    パスを許すのは、リポジトリの外にある参照エンジンのような例外のためである。
    """
    if "/" in value:
        return abs_path(value)
    paths.check_name(value)
    known = (".hmwr", ".best", ".txt")
    return str(root / (value if value.endswith(known) or not suffix else value + suffix))


def abs_path(path: str) -> str:
    p = Path(path)
    return str(p if p.is_absolute() else paths.REPO / p)


def default_builds(name: str) -> tuple[str, str]:
    """ビルドを省いたときの両側。実験ごとに固定したいので data/bin を先に見る。"""
    f = files(name)
    if f["base"].is_file() and f["cand"].is_file():
        return str(f["base"]), str(f["cand"])
    if f["base"].is_file():
        return str(f["base"]), str(f["base"])
    engine = str(paths.release_bin("himawari"))
    return engine, engine


def settings(args: argparse.Namespace) -> dict[str, str]:
    """フラグを測定条件へ畳む。"""
    env: dict[str, str] = {}
    if getattr(args, "noninferiority", False):
        # 参照追従で「害がなければ入れたい」変更（ADR-0163）
        env["SPRT_ELO0"] = "-5"
        env["SPRT_ELO1"] = "0"
    if getattr(args, "tc", None):
        env["SPRT_TC"] = args.tc
    if getattr(args, "max_pairs", None):
        env["SPRT_MAX_PAIRS"] = str(args.max_pairs)
    # 未知の鍵は黙って無視されると誤設定に気づけない。上限3,000ペアの
    # つもりが安全弁の60,000まで走った事故が実例（2026-08-29）
    allowed = set(config.DEFAULTS) | {"SPRT_MAX_PAIRS"}
    for item in getattr(args, "set", None) or []:
        if "=" not in item:
            raise proc.Fail(f"--set はKEY=VALUEで書く: {item}", proc.USAGE)
        key, value = item.split("=", 1)
        if key not in allowed:
            raise proc.Fail(
                f"--set の鍵を知らない: {key}（使える鍵: {', '.join(sorted(allowed))}）",
                proc.USAGE,
            )
        env[key] = value
    return env


def _stop_pairs(text: str) -> int:
    if text == "sprt":
        return 0
    kind, _, count = text.partition(":")
    if kind == "pairs" and count.isdigit() and int(count) > 0:
        return int(count)
    raise proc.Fail(f"--stop は sprt か pairs:N で書く: {text}", proc.USAGE)


def spec_from(args: argparse.Namespace) -> Spec:
    base_build, cand_build = default_builds(args.name)
    if args.build:
        base_build = cand_build = _named(args.build, paths.BIN, "")
    if args.base_build:
        base_build = _named(args.base_build, paths.BIN, "")
    if args.cand_build:
        cand_build = _named(args.cand_build, paths.BIN, "")

    stop_pairs = _stop_pairs(args.stop)
    if stop_pairs and (args.max_pairs or args.noninferiority):
        raise proc.Fail("--max-pairs と --noninferiority は --stop sprt の指定である", proc.USAGE)

    return Spec(
        name=args.name,
        base=Player(
            base_build,
            _named(args.base_net, paths.NETS, ".hmwr") if args.base_net else "",
            tuple(args.base_opt),
            args.base_odds or "",
        ),
        cand=Player(
            cand_build,
            _named(args.cand_net, paths.NETS, ".hmwr") if args.cand_net else "",
            tuple(args.cand_opt),
            args.cand_odds or "",
        ),
        opts=tuple(args.opt),
        stop_pairs=stop_pairs,
        openings=_named(args.openings, paths.REPO / "openings", ".txt") if args.openings else "",
        hash_mb=args.hash or "",
        max_moves=args.max_moves or "",
        env=settings(args),
        adopt=args.adopt,
    )


# --- 起動 --------------------------------------------------------------


def run(args: argparse.Namespace) -> int:
    spec = spec_from(args)
    if args.worker:
        return until_decision(spec, dry_run=False)
    if decided(spec.name):
        return proc.OK
    if not args.dry_run:
        for path in (spec.base.build, spec.cand.build, spec.base.net, spec.cand.net, spec.openings):
            if path and not Path(path).is_file():
                raise proc.Fail(f"ファイルがない: {paths.rel(path)}")
    rest = [a for a in args.argv if a not in ("--dry-run", "--foreground")]
    return start(args, spec, rest, pass_env=False)


def decided(name: str) -> bool:
    """結果が出ていれば表示してTrueを返す。"""
    result = files(name)["result"]
    if not result.is_file():
        return False
    print(f"判定済み: {paths.rel(result)}")
    print(result.read_text(encoding="utf-8"), end="")
    return True


def start(args: argparse.Namespace, spec: Spec, rest: list[str], *, pass_env: bool = True) -> int:
    """結果が出るまで回す処理を、前面か切り離しかで起動する。

    切り離した子は `hmwr <rest> --worker` で自分を呼び直す。pass_envは、
    フラグから畳んだ測定条件を `--set` で渡し直すかを決める。restが元の
    引数そのままなら、渡し直すと二重になる。
    """
    f = files(spec.name)
    if args.foreground:
        return until_decision(spec, dry_run=args.dry_run)

    argv = [sys.executable, str(paths.REPO / "bin" / "hmwr"), *rest, "--worker"]
    if pass_env:
        for key, value in spec.env.items():
            argv += ["--set", f"{key}={value}"]

    if args.dry_run:
        _selfplay(_with_hard_max(spec), dry_run=True, attempt=1)
        print(f"[dry-run] （新しいセッションで）{proc.show(argv)}")
        return proc.OK

    # 親から切り離す。状態はファイルにあるので、掴んでおく必要がない
    with open(os.devnull, "wb") as devnull:
        child = subprocess.Popen(
            argv,
            cwd=str(paths.REPO),
            stdout=devnull,
            stderr=devnull,
            stdin=devnull,
            start_new_session=True,
        )
    print(f"起動した pid={child.pid}（新しいセッション）")
    print(f"経過: hmwr match show {spec.name}")
    print(f"完了: {paths.rel(f['result'])} の出現を見る")
    return proc.OK


# --- 結果が出るまで回す ------------------------------------------------


def until_decision(spec: Spec, *, dry_run: bool) -> int:
    """結果が出るまで走らせる。

    2つを自動化する。落ちたら棋譜から拾い直すことと、上限に達しても
    判定が出ていなければそのまま走り続けることである（ADR-0087・0175）。

    上限は収束の判定基準ではなく暴走を止める安全弁である。真のEloが
    対立仮説の中点ちょうどだと理論上収束しないため、無制限にはしない。

    効く範囲と効かない範囲がある。対局プロセスだけが落ちた場合はここが
    拾い直す。この処理自体が止められた場合はループごと消えるが、棋譜は
    残るので次に呼べば続きから走る。**「止まらない」ではなく「止まっても
    失わない」のが本質である。**
    """
    f = files(spec.name)
    if f["result"].is_file():
        print(f["result"].read_text(encoding="utf-8"), end="")
        return _exit_code(f["result"])

    spec = _with_hard_max(spec)
    hard_max = spec.env["SPRT_MAX_PAIRS"]

    for attempt in range(1, MAX_RETRY + 1):
        before = _games(f["jsonl"])
        code = _selfplay(spec, dry_run=dry_run, attempt=attempt)
        if dry_run:
            return proc.OK
        if code in (0, 1):
            return _finish(spec)
        if code == 2:
            if spec.stop_pairs and _games(f["jsonl"]) >= spec.stop_pairs * 2:
                return _finish(spec)
            if spec.stop_pairs:
                print(f"{spec.stop_pairs} ペアを指し切る前に止まった。同じコマンドで続きから走る。")
                return 2
            print(f"安全弁（{hard_max} ペア）まで走って判定に至らなかった。")
            print("局数を積むより対立仮説の立て方を見直す。")
            return 2

        # **1局も進まなかった再試行は繰り返さない。** 設定の誤りやバイナリの
        # 欠落なら、何度試しても同じところで落ちる。棋譜が増えているときだけ
        # 「途中で落ちた」とみなして拾い直す
        if _games(f["jsonl"]) == before:
            raise proc.Fail(
                f"1局も進まずに終了した（コード {code}）。設定かバイナリを確かめる。\n"
                f"ログ: {paths.rel(f['log'])}",
                code,
            )
        print(f"試行 {attempt} が異常終了（コード {code}）。再開する", file=sys.stderr)
        time.sleep(RETRY_WAIT)

    raise proc.Fail(f"{MAX_RETRY}回試しても結果に至らなかった")


def _with_hard_max(spec: Spec) -> Spec:
    """ペア数の上限を決める。固定ペア数ならその数、SPRTなら安全弁になる。"""
    if spec.stop_pairs:
        hard_max = str(spec.stop_pairs)
    else:
        hard_max = (
            spec.env.get("SPRT_MAX_PAIRS")
            or spec.env.get("SPRT_HARD_MAX_PAIRS")
            or config.get("SPRT_HARD_MAX_PAIRS", "60000")
        )
    return replace(spec, env={**spec.env, "SPRT_MAX_PAIRS": hard_max})


def _games(jsonl: Path) -> int:
    """棋譜に記録された局数。ファイルが無ければ0。"""
    if not jsonl.is_file():
        return 0
    return sum(1 for _ in jsonl.open("rb"))


def _selfplay(spec: Spec, *, dry_run: bool, attempt: int) -> int:
    """対局を1回走らせる。棋譜があれば続きから測る。"""
    f = files(spec.name)
    binary = paths.release_bin("selfplay")
    if not binary.is_file() and not dry_run:
        raise proc.Fail(f"{paths.rel(binary)} がない。cargo build --release を実行する")

    def setting(key: str, fallback: str = "") -> str:
        return spec.env.get(key) or config.get(key, fallback)

    argv = [str(binary), "--baseline", spec.base.build, "--candidate", spec.cand.build]
    if spec.base.net or spec.cand.net:
        # 評価関数を片側ずつ渡す。--option と併用すると、どちらが効くかが
        # 実装依存になるため、片側指定だけで完結させる
        argv += ["--bopt", f"EvalFile={spec.base.net or setting('EVAL_FILE')}"]
        argv += ["--copt", f"EvalFile={spec.cand.net or setting('EVAL_FILE')}"]
    elif setting("EVAL_FILE"):
        argv += ["--option", f"EvalFile={setting('EVAL_FILE')}"]
    for item in spec.opts:
        argv += ["--option", item]
    for item in spec.base.opts:
        argv += ["--bopt", item]
    for item in spec.cand.opts:
        argv += ["--copt", item]
    if spec.base.odds:
        argv += ["--bodds", spec.base.odds]
    if spec.cand.odds:
        argv += ["--codds", spec.cand.odds]

    argv += [
        "--openings", spec.openings or setting("OPENINGS"),
        "--tc", setting("SPRT_TC", "10+0.1"),
        "--concurrency", setting("SPRT_CONCURRENCY", "8"),
        "--hash", spec.hash_mb or config.MATCH_HASH,
        "--max-moves", spec.max_moves or config.MATCH_MAX_MOVES,
        "--adjudicate", setting("SPRT_ADJUDICATE", "2000,8"),
        "--elo0", setting("SPRT_ELO0", "0"),
        "--elo1", setting("SPRT_ELO1", "5"),
        "--alpha", setting("SPRT_ALPHA", "0.05"),
        "--beta", setting("SPRT_BETA", "0.05"),
    ]  # fmt: skip
    if spec.stop_pairs:
        argv += ["--no-stop"]

    f["jsonl"].parent.mkdir(parents=True, exist_ok=True)
    games = _games(f["jsonl"])
    check_conditions(f["cond"], proc.show(argv), games, adopt=spec.adopt, dry_run=dry_run)

    # ペア数の上限は条件に数えない。続きを積むときに変わるのが正常である
    argv += ["--max-pairs", setting("SPRT_MAX_PAIRS", "60000"), "--out", str(f["jsonl"])]
    if games:
        print(f"試行 {attempt}: 既存の棋譜から再開する（{games} 局）")
        argv += ["--resume", str(f["jsonl"])]
    else:
        print(f"試行 {attempt}: 新規に開始する")

    return proc.run(argv, dry_run=dry_run, log=f["log"], allowed=(0, 1, 2, 3))


def check_conditions(cond: Path, line: str, games: int, *, adopt: bool, dry_run: bool) -> None:
    """同じ名前の棋譜へ、違う条件の対局を積まない。"""
    conditions.check(
        cond, line, resuming=games > 0, adopt=adopt, dry_run=dry_run, what="棋譜"
    )


def _finish(spec: Spec) -> int:
    """結果が出た。結果ファイルを書き、判定を終了コードで返す。"""
    f = files(spec.name)
    try:
        text, verdict = sprt_log.report(
            f["log"], spec.name, result=f["result"], fixed_pairs=spec.stop_pairs
        )
    except sprt_log.Unreadable as e:
        raise proc.Fail(f"結果は出たが読めない: {e}") from e
    print(text)
    return sprt_log.EXIT_BY_VERDICT[verdict]


def _exit_code(result: Path) -> int:
    for line in result.read_text(encoding="utf-8").splitlines():
        if line.startswith("decision="):
            return sprt_log.EXIT_BY_VERDICT.get(line.split("=", 1)[1], proc.RUNTIME)
    raise proc.Fail(f"結果ファイルの判定を読めない: {paths.rel(result)}")


# --- 確認 --------------------------------------------------------------


def show(args: argparse.Namespace) -> int:
    """途中経過を出す。名前を省くと走行を新しい順に並べる。"""
    if not args.name:
        return _list(args.all)

    f = files(args.name)
    try:
        text, verdict = sprt_log.report(f["log"], args.name)
    except sprt_log.Unreadable as e:
        raise proc.Fail(str(e)) from e
    print(text)
    return sprt_log.EXIT_BY_VERDICT[verdict]


def _list(show_all: bool) -> int:
    """走行の一覧。完了は結果ファイルの有無で決まる（ADR-0175）。"""
    if not paths.SPRT.is_dir():
        print("走行はまだない")
        return proc.OK

    rows = []
    for jsonl in paths.SPRT.glob("*.jsonl"):
        name = jsonl.stem
        done = (paths.SPRT / f"{name}.result").is_file()
        games = sum(1 for _ in jsonl.open("rb"))
        rows.append((jsonl.stat().st_mtime, name, "完了" if done else "未完了", games))
    if not rows:
        print("走行はまだない")
        return proc.OK

    rows.sort(reverse=True)
    shown = rows if show_all else rows[:10]
    width = max(paths.display_width(r[1]) for r in shown) + 2
    print(paths.pad("名前", width) + paths.pad("状態", 8) + "局数")
    for _, name, state, games in shown:
        print(paths.pad(name, width) + paths.pad(state, 8) + str(games))
    if not show_all and len(rows) > len(shown):
        print(f"\n新しい順に{len(shown)}件。全{len(rows)}件を見るには --all")
    print("\n「未完了」は結果ファイルがない状態を指す。走っているとは限らない。")
    return proc.OK


def wait(args: argparse.Namespace) -> int:
    """結果が出るまで待つ。

    結果ファイルの出現を見る。対局プロセスが消えていて結果も無ければ、
    結果の前に止まったとみなす。
    """
    f = files(args.name)
    if args.dry_run:
        print(f"[dry-run] {paths.rel(f['result'])} の出現を待つ")
        return proc.OK

    while True:
        if f["result"].is_file():
            print(f["result"].read_text(encoding="utf-8"), end="")
            return _exit_code(f["result"])
        if not _running(args.name):
            print("結果の前に止まった（中断・停止・失敗のいずれか）", file=sys.stderr)
            if f["log"].is_file():
                tail = f["log"].read_text(encoding="utf-8").splitlines()[-2:]
                print("\n".join(tail), file=sys.stderr)
            return proc.JUDGE
        time.sleep(args.interval)


def _running(name: str) -> bool:
    """この実験の対局プロセスが生きているか。"""
    out = proc.capture(["pgrep", "-f", f"selfplay .*{name}"])
    return bool(out.strip())
