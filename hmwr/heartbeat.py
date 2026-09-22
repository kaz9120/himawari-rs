"""長く走るコマンドの心拍（ADR-0220）。

ログは人が読む用のまま残し、機械は心拍だけを読む。心拍は
`data/status/<領域>-<名前>.json` に、60秒ごとと開始・終了時に書く。
`hmwr status` とポータルはこのファイルだけを見て、ログを解釈しない。

形式は1つで、領域ごとの追加情報は `detail` に入れる。

    {
      "kind": "relabel", "name": "train_7860M_q1", "state": "running",
      "progress": {"done": 62500864, "total": 2000000000, "unit": "局面"},
      "rate": 2397.0, "eta_seconds": 19800,
      "started": "...", "updated": "...", "pid": 12345,
      "log": "data/logs/relabel-train_7860M_q1.log",
      "detail": {"scale": 430}
    }

`state` は running・done・failed・stopped の4つ。書き込みは一時ファイルへ
書いてから改名するので、読み手が途中の内容を見ることはない。
"""

from __future__ import annotations

import json
import os
import time
from pathlib import Path

from . import paths

STATES = ("running", "done", "failed", "stopped")
INTERVAL = 60.0


def status_dir() -> Path:
    return paths.STATUS


def path_of(kind: str, name: str) -> Path:
    return status_dir() / f"{kind}-{paths.check_name(name)}.json"


def _now() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%S%z")


class Heartbeat:
    """1つの走行の心拍。`update` は間隔を守り、`finish` は必ず書く。"""

    def __init__(
        self,
        kind: str,
        name: str,
        *,
        total: int | None = None,
        unit: str = "",
        log: Path | str | None = None,
        detail: dict | None = None,
        interval: float = INTERVAL,
    ):
        self.path = path_of(kind, name)
        self.interval = interval
        self._t0 = time.time()
        self._last_write = 0.0
        self._done0: int | None = None
        self.data = {
            "kind": kind,
            "name": name,
            "state": "running",
            "progress": {"done": 0, "total": total, "unit": unit},
            "rate": None,
            "eta_seconds": None,
            "started": _now(),
            "updated": None,
            "pid": os.getpid(),
            "log": paths.rel(log) if log else None,
            "detail": dict(detail or {}),
        }
        self._write()

    def update(self, done: int | None = None, *, detail: dict | None = None, force: bool = False) -> None:
        """進み具合を反映する。間隔（既定60秒）に満たなければ書かない。"""
        now = time.time()
        if done is not None:
            if self._done0 is None:
                self._done0 = done
                self._t_done0 = now
            self.data["progress"]["done"] = done
            elapsed = now - self._t_done0
            if elapsed > 0 and done > self._done0:
                rate = (done - self._done0) / elapsed
                self.data["rate"] = round(rate, 2)
                total = self.data["progress"]["total"]
                if total is not None:
                    self.data["eta_seconds"] = round((total - done) / rate)
        if detail:
            self.data["detail"].update(detail)
        if force or now - self._last_write >= self.interval:
            self._write()

    def finish(self, state: str = "done", **detail) -> None:
        if state not in STATES:
            raise ValueError(f"知らない状態: {state}")
        self.data["state"] = state
        if detail:
            self.data["detail"].update(detail)
        if state == "done" and self.data["progress"]["total"] is not None:
            self.data["progress"]["done"] = self.data["progress"]["total"]
            self.data["eta_seconds"] = 0
        self._write()

    def _write(self) -> None:
        self.data["updated"] = _now()
        self.path.parent.mkdir(parents=True, exist_ok=True)
        tmp = self.path.with_name(self.path.name + ".tmp")
        tmp.write_text(json.dumps(self.data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        tmp.replace(self.path)
        self._last_write = time.time()


def _alive(pid: int | None) -> bool:
    if not pid:
        return False
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def read_all(stale_after: float = 600.0) -> list[dict]:
    """心拍をすべて読み、走行中のものに `alive`（プロセスの生死）と
    `stale`（更新が途絶えた）を付けて、更新の新しい順に返す。"""
    out = []
    for p in status_dir().glob("*.json"):
        try:
            d = json.loads(p.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue
        d["file"] = paths.rel(p)
        if d.get("state") == "running":
            d["alive"] = _alive(d.get("pid"))
            age = time.time() - p.stat().st_mtime
            d["stale"] = age > stale_after
        out.append(d)
    out.sort(key=lambda d: d.get("updated") or "", reverse=True)
    return out
