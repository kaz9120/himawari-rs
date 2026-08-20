"""探索定数のSPSAチューニング（ADR-0143）。

init（tuneビルドと対象一覧の生成）→ run（判定なしの対局ループ）→
結果の定数焼き込みとheld-out SPRT、の順で使う。runは切り離して走り、
状態は data/spsa/<名前>.state.json にある。落ちても同じコマンドで
続きから走る（ADR-0123）。乱数はペア番号から決定論で引くので、
再開しても摂動列は変わらない。
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

from .. import config, paths, proc, spsa_core
from ..spsa_core import Param
from . import build

# 1バッチ全ペアが結果を返さない状態が続いたら止める。設定の誤りなら
# 何度試しても同じところで落ちる
MAX_FAILED_BATCHES = 3


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("spsa", help="探索定数をSPSAでチューニングする")
    ss = p.add_subparsers(dest="sub", metavar="<操作>")

    t = ss.add_parser(
        "init",
        help="tuneビルドを作り、対象一覧の雛形を書く",
        description="cargoのtune featureでビルドし、エンジンが宣言する"
        "チューニング項目から data/spsa/<名前>.params.json を作る。"
        "対象や摂動幅を絞るときは、走らせる前にこのファイルを編集する。",
    )
    t.add_argument("name", help="実験名")
    t.set_defaults(func=init)

    t = ss.add_parser(
        "run",
        help="対局ループを回してθを動かす",
        description="params.jsonの全項目を同時に摂動し、θ+対θ−の1ペアごとに"
        "θを更新する。判定はなく、指定ペア数で止まって結果を書く。",
    )
    t.add_argument("name", help="実験名")
    t.add_argument("--pairs", type=int, metavar="N", help="総ペア数（既定 15000）")
    t.add_argument("--tc", metavar="持ち時間", help="例 10+0.1（既定はSPRTと同じ）")
    t.add_argument(
        "--concurrency", type=int, metavar="C", help="同時に走らせるペア数"
    )
    t.add_argument("--foreground", action="store_true", help="切り離さず走らせる")
    t.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    t.set_defaults(func=run)

    t = ss.add_parser("show", help="途中経過や結果を出す")
    t.add_argument("name", help="実験名")
    t.set_defaults(func=show)

    t = ss.add_parser("stop", help="次のバッチの前で止める（再開できる）")
    t.add_argument("name", help="実験名")
    t.set_defaults(func=stop)


def files(name: str) -> dict[str, Path]:
    paths.check_name(name)
    return {
        "bin": paths.BIN / f"tune-{name}",
        "params": paths.SPSA / f"{name}.params.json",
        "state": paths.SPSA / f"{name}.state.json",
        "result": paths.SPSA / f"{name}.result",
        "games": paths.SPSA / f"{name}.games.jsonl",
        "tmp": paths.SPSA / f"tmp-{name}",
        "stop": paths.SPSA / f"{name}.stop",
        "log": paths.log("spsa", name),
    }


# --- init --------------------------------------------------------------


def init(args: argparse.Namespace) -> int:
    """tuneビルドを作り、エンジンの宣言から対象一覧の雛形を書く。"""
    f = files(args.name)
    build.require_clean_crates()
    build.cargo_build(dry_run=args.dry_run, args=["-p", "himawari-usi", "--features", "tune"])
    build._copy(paths.REPO / "target" / "release" / "himawari", f["bin"], dry_run=args.dry_run)
    if args.dry_run:
        return proc.OK

    entries = _query_tunables(f["bin"])
    if not entries:
        raise proc.Fail("エンジンがチューニング項目を宣言しない。tuneビルドかを確かめる")

    if f["params"].is_file():
        print(f"既にある: {paths.rel(f['params'])}（上書きしない）")
        return proc.OK

    # 摂動の終端c_endは可動域の1/20、歩幅の終端r_endはfishtestの既定に置く。
    # 絞る・広げるはこのファイルを編集する
    params = [
        {
            "name": e["name"],
            "default": e["default"],
            "min": e["min"],
            "max": e["max"],
            "c_end": max((e["max"] - e["min"]) / 20.0, 1.0),
            "r_end": 0.002,
        }
        for e in entries
    ]
    f["params"].parent.mkdir(parents=True, exist_ok=True)
    f["params"].write_text(
        json.dumps({"pairs": 15000, "params": params}, ensure_ascii=False, indent=1) + "\n",
        encoding="utf-8",
    )
    print(f"書いた: {paths.rel(f['params'])}（{len(params)}項目）")
    print(f"次の手順: 対象を絞るなら編集し、hmwr spsa run {args.name}")
    return proc.OK


def _query_tunables(binary: Path) -> list[dict]:
    """tuneビルドへ `tunables` を送り、宣言された項目を集める。"""
    out = subprocess.run(
        [str(binary)],
        input="tunables\nquit\n",
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    ).stdout
    entries = []
    for line in out.splitlines():
        t = line.split()
        # tunable name <名前> default <値> min <下限> max <上限>
        if len(t) == 9 and t[0] == "tunable":
            entries.append(
                {"name": t[2], "default": int(t[4]), "min": int(t[6]), "max": int(t[8])}
            )
    return entries


# --- run ---------------------------------------------------------------


def run(args: argparse.Namespace) -> int:
    f = files(args.name)

    if args.worker:
        return until_done(args, log_to_file=True)

    if f["result"].is_file():
        print(f"完了済み: {paths.rel(f['result'])}")
        print(f["result"].read_text(encoding="utf-8"), end="")
        return proc.OK
    if not f["params"].is_file() or (not f["bin"].is_file() and not args.dry_run):
        raise proc.Fail(f"先に hmwr spsa init {args.name} を実行する")

    if args.foreground or args.dry_run:
        return until_done(args, log_to_file=False)

    argv = [sys.executable, str(paths.REPO / "bin" / "hmwr"), "spsa", "run", args.name, "--worker"]
    for flag in ("pairs", "tc", "concurrency"):
        value = getattr(args, flag, None)
        if value:
            argv += [f"--{flag}", str(value)]
    f["stop"].unlink(missing_ok=True)
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
    print(f"経過: hmwr spsa show {args.name}")
    print(f"停止: hmwr spsa stop {args.name}")
    return proc.OK


def until_done(args: argparse.Namespace, *, log_to_file: bool) -> int:
    """指定ペア数までθを動かし、結果を書く。"""
    f = files(args.name)
    spec = json.loads(f["params"].read_text(encoding="utf-8"))
    params = [
        Param(
            name=p["name"],
            default=float(p["default"]),
            lo=float(p["min"]),
            hi=float(p["max"]),
            c_end=float(p["c_end"]),
            r_end=float(p["r_end"]),
        )
        for p in spec["params"]
    ]

    state = _load_state(f, args, spec, params)
    total = state["pairs_total"]
    concurrency = args.concurrency or config.concurrency()
    log = _logger(f["log"] if log_to_file else None)

    selfplay = paths.REPO / "target" / "release" / "selfplay"
    if not selfplay.is_file() and not args.dry_run:
        raise proc.Fail(f"{paths.rel(selfplay)} がない。cargo build --release を実行する")

    failed_batches = 0
    while state["pairs_done"] < total:
        if f["stop"].is_file():
            log(f"停止ファイルを見つけた。{state['pairs_done']}/{total}ペアで中断する")
            f["stop"].unlink(missing_ok=True)
            return proc.OK

        batch = min(concurrency, total - state["pairs_done"])
        jobs = []
        theta = state["theta"]
        for j in range(batch):
            k = state["pairs_done"] + j + 1
            c_mult, r_mult = spsa_core.schedule(k, total)
            delta = spsa_core.deltas(state["seed"], k, params)
            plus = spsa_core.perturbed(theta, params, delta, c_mult, +1)
            minus = spsa_core.perturbed(theta, params, delta, c_mult, -1)
            argv = _selfplay_argv(f, selfplay, state["tc"], k, plus, minus)
            if args.dry_run:
                print(f"[dry-run] {proc.show(argv)}")
                return proc.OK
            jobs.append((k, delta, c_mult, r_mult, _spawn(argv)))

        scored = 0
        for k, delta, c_mult, r_mult, child in jobs:
            child.wait()
            lines = _read_pair(f["tmp"] / f"{k}.jsonl")
            if lines is None:
                log(f"ペア{k}が結果を返さなかった（飛ばす）")
                continue
            score = spsa_core.pair_score(lines)
            state["theta"] = spsa_core.update(
                state["theta"], params, delta, c_mult, r_mult, score
            )
            _archive(f, lines)
            scored += 1

        failed_batches = 0 if scored else failed_batches + 1
        if failed_batches >= MAX_FAILED_BATCHES:
            raise proc.Fail(
                f"{MAX_FAILED_BATCHES}バッチ連続で1ペアも結果が返らない。"
                f"ログを確かめる: {paths.rel(f['log'])}"
            )

        state["pairs_done"] += batch
        _save_state(f, state)
        if (state["pairs_done"] // concurrency) % 25 == 0:
            log(_progress_line(state, params, total))

    _finish(f, state, params, log)
    return proc.OK


def _selfplay_argv(
    f: dict[str, Path],
    selfplay: Path,
    tc: str,
    k: int,
    plus: dict[str, int],
    minus: dict[str, int],
) -> list[str]:
    """1ペアぶんのselfplay起動引数。candidate側をθ+に割り当てる。"""
    argv = [
        str(selfplay),
        "--baseline", str(f["bin"]),
        "--candidate", str(f["bin"]),
        "--openings", config.get("OPENINGS"),
        "--tc", tc,
        "--concurrency", "1",
        "--max-pairs", "1",
        "--adjudicate", config.get("SPRT_ADJUDICATE"),
        "--option", f"EvalFile={config.get('EVAL_FILE')}",
        "--out", str(f["tmp"] / f"{k}.jsonl"),
    ]
    for name, value in plus.items():
        argv += ["--copt", f"{name}={value}"]
    for name, value in minus.items():
        argv += ["--bopt", f"{name}={value}"]
    return argv


def _spawn(argv: list[str]) -> subprocess.Popen:
    return subprocess.Popen(
        argv,
        cwd=str(paths.REPO),
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        stdin=subprocess.DEVNULL,
    )


def _read_pair(path: Path) -> list[dict] | None:
    """1ペアの棋譜を読む。2局そろっていなければNone。"""
    if not path.is_file():
        return None
    lines = [json.loads(x) for x in path.read_text(encoding="utf-8").splitlines() if x]
    return lines if len(lines) == 2 else None


def _archive(f: dict[str, Path], lines: list[dict]) -> None:
    with f["games"].open("a", encoding="utf-8") as out:
        for rec in lines:
            out.write(json.dumps(rec, ensure_ascii=False) + "\n")


def _load_state(
    f: dict[str, Path],
    args: argparse.Namespace,
    spec: dict,
    params: list[Param],
) -> dict:
    """状態を読む。無ければフラグとparams.jsonから作る。"""
    if f["state"].is_file():
        state = json.loads(f["state"].read_text(encoding="utf-8"))
        missing = [p.name for p in params if p.name not in state["theta"]]
        if missing:
            raise proc.Fail(f"状態とparams.jsonが食い違う（{missing[:3]}…）。名前を変えて始める")
        return state
    state = {
        "pairs_total": args.pairs or spec.get("pairs", 15000),
        "tc": args.tc or config.get("SPRT_TC", config.SPRT_TC),
        "seed": int.from_bytes(os.urandom(4), "big"),
        "pairs_done": 0,
        "theta": {p.name: p.default for p in params},
        "started": datetime.now(timezone.utc).isoformat(timespec="seconds"),
    }
    if not args.dry_run:
        f["tmp"].mkdir(parents=True, exist_ok=True)
        for stale in f["tmp"].glob("*.jsonl"):
            stale.unlink()
        _save_state(f, state)
    return state


def _save_state(f: dict[str, Path], state: dict) -> None:
    state["updated"] = datetime.now(timezone.utc).isoformat(timespec="seconds")
    tmp = f["state"].with_suffix(".json.tmp")
    tmp.write_text(json.dumps(state, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    tmp.replace(f["state"])


def _logger(log: Path | None):
    def emit(msg: str) -> None:
        stamp = datetime.now(timezone.utc).isoformat(timespec="seconds")
        line = f"{stamp} {msg}"
        print(line, flush=True)
        if log is not None:
            with log.open("a", encoding="utf-8") as out:
                out.write(line + "\n")

    return emit


def _progress_line(state: dict, params: list[Param], total: int) -> str:
    moved = sum(1 for p in params if round(state["theta"][p.name]) != round(p.default))
    return f"{state['pairs_done']}/{total}ペア（初期値から動いた項目 {moved}/{len(params)}）"


def _finish(f: dict[str, Path], state: dict, params: list[Param], log) -> None:
    """最終θを結果ファイルへ書く。焼き込みとheld-out検収は人（エージェント）が行う。"""
    rows = ["name\tdefault\ttuned"]
    for p in params:
        rows.append(f"{p.name}\t{round(p.default)}\t{round(state['theta'][p.name])}")
    body = "\n".join(rows) + "\n"
    f["result"].write_text(body, encoding="utf-8")
    log(f"完了: {state['pairs_done']}ペア。結果: {paths.rel(f['result'])}")
    log("次の手順: 動いた定数をtunables.rsへ焼き込み、SPRT既定条件で検収する（ADR-0143）")
    for stale in f["tmp"].glob("*.jsonl"):
        stale.unlink()


# --- show / stop -------------------------------------------------------


def show(args: argparse.Namespace) -> int:
    f = files(args.name)
    if f["result"].is_file():
        print(f["result"].read_text(encoding="utf-8"), end="")
        return proc.OK
    if not f["state"].is_file():
        print(f"状態がない。hmwr spsa init {args.name} から始める")
        return proc.JUDGE
    state = json.loads(f["state"].read_text(encoding="utf-8"))
    spec = json.loads(f["params"].read_text(encoding="utf-8"))
    defaults = {p["name"]: p["default"] for p in spec["params"]}
    print(f"{state['pairs_done']}/{state['pairs_total']}ペア（更新 {state.get('updated', '?')}）")
    print("name\tdefault\tnow")
    for name, value in state["theta"].items():
        print(f"{name}\t{defaults.get(name)}\t{round(value)}")
    return proc.OK


def stop(args: argparse.Namespace) -> int:
    f = files(args.name)
    f["stop"].touch()
    print(f"次のバッチの前で止まる。再開は hmwr spsa run {args.name}")
    return proc.OK
