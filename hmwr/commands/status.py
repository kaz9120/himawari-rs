"""今の状態を1つにまとめて出す。

状態ファイル（実行中の長いコマンド）、直近の結果、資源、設定を集める。
`--json` は機械向けで、人向けは同じ内容を表で出す。

GitHubに届かないときは、その部分だけ `error` を入れて他は出す。
"""

from __future__ import annotations

import argparse
import datetime
import json
import shutil
import time

from .. import config, heartbeat, paths, proc

RESULT_KEYS = ("decision", "elo", "ci_low", "ci_high", "games", "llr", "finished_at")
RECENT = 10


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser("status", help="実行中の長いコマンド・直近の結果・資源・設定を1つにまとめて出す")
    p.add_argument("--json", action="store_true", help="機械向けにJSONで出す")
    p.add_argument("--no-github", action="store_true", help="GitHubを読まない（開いているPRを省く）")
    p.add_argument("--config", action="store_true", help="設定だけを出す")
    p.set_defaults(func=show)


# --- 集める ------------------------------------------------------------


def _results() -> dict:
    matches = []
    if paths.SPRT.is_dir():
        files = sorted(paths.SPRT.glob("*.result"), key=lambda p: p.stat().st_mtime, reverse=True)
        for p in files[:RECENT]:
            row = {"name": p.stem}
            for line in p.read_text(encoding="utf-8").splitlines():
                if "=" in line:
                    k, v = line.split("=", 1)
                    if k in RESULT_KEYS:
                        row[k] = v
            matches.append(row)
    nets = []
    registry = paths.registry()
    if registry.is_file():
        lines = registry.read_text(encoding="utf-8").splitlines()
        if lines:
            head = lines[0].split("\t")
            for line in lines[-RECENT:][::-1]:
                cells = line.split("\t")
                nets.append(dict(zip(head, cells)))
    return {"matches": matches, "nets": nets}


def _gh(*args: str) -> str:
    code, out, err = proc.capture_code(["gh", *args])
    if code != proc.OK:
        raise proc.Fail(f"ghが失敗した: gh {' '.join(args)}\n{err.strip()}")
    return out


def _resources(use_github: bool) -> dict:
    # data/ が別のボリュームにあってもそちらを測る。無い作業ツリーでは根で測る
    data = paths.WORK / "data"
    usage = shutil.disk_usage(data if data.is_dir() else paths.WORK)
    out: dict = {
        "disk_free_gb": round(usage.free / 1e9, 1),
        "disk_total_gb": round(usage.total / 1e9, 1),
        "open_prs": [],
    }
    if use_github:
        try:
            raw = _gh("pr", "list", "--state", "open", "--json", "number,title,isDraft,headRefName")
            out["open_prs"] = json.loads(raw)
        except proc.Fail as e:
            out["error"] = str(e).splitlines()[0]
    return out


def _rate_eta(d: dict) -> tuple[float | None, int | None]:
    """付け直しの記録から、開始以来の平均の速さと残り時間を出す。"""
    try:
        # Python 3.10のfromisoformatは+0900の形を読めない
        t0 = datetime.datetime.strptime(d["started"], "%Y-%m-%dT%H:%M:%S%z")
        t1 = datetime.datetime.strptime(d["updated"], "%Y-%m-%dT%H:%M:%S%z")
        done, total = int(d["done"]), int(d["count"])
    except (KeyError, TypeError, ValueError):
        return None, None
    seconds = (t1 - t0).total_seconds()
    if seconds <= 0 or done <= 0:
        return None, None
    rate = done / seconds
    return round(rate, 2), (0 if d.get("finished") else round((total - done) / rate))


def _relabel_progress() -> list[dict]:
    """その場の付け直しの進み具合（ADR-0219）。状態ファイルを持たない旧い実行の代わりに読む。"""
    out = []
    for p in paths.TRAIN.glob("*.psv.relabel.json"):
        try:
            d = json.loads(p.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue
        rate, eta = _rate_eta(d)
        out.append(
            {
                "kind": "relabel",
                "name": p.name.split(".psv")[0],
                "state": "done" if d.get("finished") else "running",
                "progress": {"done": d.get("done"), "total": d.get("count"), "unit": "局面"},
                "rate": rate,
                "eta_seconds": eta,
                "started": d.get("started"),
                "updated": d.get("updated"),
                "detail": {k: d.get(k) for k in ("labeler", "scale", "start")},
                "file": paths.rel(p),
            }
        )
    return out


def collect(*, use_github: bool = True) -> dict:
    beats = heartbeat.read_all()
    known = {(b["kind"], b["name"]) for b in beats}
    beats += [r for r in _relabel_progress() if (r["kind"], r["name"]) not in known]
    return {
        "generated": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "heartbeats": beats,
        "results": _results(),
        "resources": _resources(use_github),
        "config": [list(row) for row in config.summary()],
    }


# --- 見せる ------------------------------------------------------------


def _eta(seconds) -> str:
    if seconds is None:
        return ""
    if seconds < 3600:
        return f"残り{seconds / 60:.0f}分"
    return f"残り{seconds / 3600:.1f}時間"


def render(d: dict) -> str:
    lines = []
    lines.append("== 状態ファイル")
    beats = d["heartbeats"]
    if not beats:
        lines.append("  なし")
    for b in beats:
        p = b.get("progress") or {}
        prog = ""
        if p.get("total"):
            prog = f"{p.get('done', 0):,}/{p['total']:,}{p.get('unit', '')}"
        elif p.get("done"):
            prog = f"{p['done']:,}{p.get('unit', '')}"
        flags = []
        if b.get("state") == "running" and b.get("alive") is False:
            flags.append("プロセスなし")
        if b.get("stale"):
            flags.append("更新が途絶")
        rate = f" {b['rate']:,.0f}/秒" if b.get("rate") else ""
        lines.append(
            f"  {paths.pad(b['kind'] + ':' + b['name'], 34)}{paths.pad(b['state'], 9)}{prog}{rate} "
            f"{_eta(b.get('eta_seconds'))} {' '.join(flags)}".rstrip()
        )
    lines.append("== 直近の対局")
    for m in d["results"]["matches"][:5]:
        elo = f"{m.get('elo', '')} [{m.get('ci_low', '')}, {m.get('ci_high', '')}]" if m.get("elo") else ""
        lines.append(f"  {paths.pad(m['name'], 34)}{paths.pad(m.get('decision', ''), 8)}{elo} {m.get('games', '')}局")
    lines.append("== 直近の学習")
    for n in d["results"]["nets"][:5]:
        lines.append(f"  {paths.pad(n.get('name', ''), 34)}valid {n.get('best_valid', '')}  {n.get('timestamp', '')[:10]}")
    r = d["resources"]
    lines.append("== 資源")
    lines.append(f"  ディスク: 空き {r['disk_free_gb']} GB / {r['disk_total_gb']} GB")
    if r.get("open_prs"):
        for pr in r["open_prs"]:
            lines.append(f"  PR #{pr['number']}{'（draft）' if pr.get('isDraft') else ''} {pr['title']}")
    else:
        lines.append("  開いているPR: なし")
    lines.append("== 設定")
    width = max(paths.display_width(k) for k, _ in d["config"]) + 2
    for k, v in d["config"]:
        lines.append(f"  {paths.pad(k, width)}{v}")
    return "\n".join(lines)


def show(args: argparse.Namespace) -> int:
    if args.config:
        rows = config.summary()
        width = max(paths.display_width(k) for k, _ in rows) + 2
        for key, value in rows:
            print(f"{paths.pad(key, width)}{value}")
        return proc.OK
    d = collect(use_github=not args.no_github)
    if args.json:
        print(json.dumps(d, ensure_ascii=False, indent=2))
    else:
        print(render(d))
    return proc.OK
