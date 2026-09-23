"""実験のspec（`experiments/<名前>.toml`）を読み、検査する（ADR-0209）。

specは実験の中身を `hmwr` のコマンド列で持つ。Issueの本文ではなくgitに置くのは、
事前登録した手順を実行後に書き換えられないようにするためである。実行器は
origin/mainにあるspecだけを実行する。

```toml
adr = "0210"

[[step]]
id = "mix"
run = "hmwr data mix mixhao20_300M --in train_300M_q1 --in hao_extra_60M_q1"
```

ステップは `hmwr` のコマンド1つにする。型付きのステップやDAGは作らない。
"""

from __future__ import annotations

import re
import shlex
from dataclasses import dataclass
from pathlib import Path

from . import paths, proc

try:  # Python 3.11以降
    import tomllib
except ModuleNotFoundError:  # 開発機のPythonは3.10で、hmwrは外部依存を持たない
    tomllib = None

DIR = "experiments"
ADR_RE = re.compile(r"^[0-9]{4}$")

# 終わるまで戻らない形でしか、次のステップと順番を保てない
DETACHING = (("match", "run"), ("sprt", "run"), ("sprt", "net"))


class Invalid(proc.Fail):
    """specの書き方が規約に合わない。"""

    def __init__(self, message: str):
        super().__init__(message, proc.USAGE)


@dataclass(frozen=True)
class Step:
    id: str
    run: str

    @property
    def argv(self) -> list[str]:
        """`hmwr` の後ろに続く引数。シェルは通さない。"""
        return shlex.split(self.run)[1:]


@dataclass(frozen=True)
class Spec:
    name: str
    adr: str
    steps: tuple[Step, ...]


def path_of(name: str) -> str:
    return f"{DIR}/{paths.check_name(name)}.toml"


def load(name: str, *, ref: str | None = "origin/main") -> Spec:
    """specを読む。refを渡すとそのコミットの内容を、Noneなら作業ツリーを読む。"""
    rel = path_of(name)
    if ref is None:
        file = paths.REPO / rel
        if not file.is_file():
            raise proc.Fail(f"specがない: {rel}")
        return parse(file.read_text(encoding="utf-8"), name)
    text, err = proc.capture_both(["git", "show", f"{ref}:{rel}"])
    if not text:
        raise proc.Fail(
            f"specが {ref} にない: {rel}\n"
            "事前登録のPRをマージしてから走らせる\n" + err.strip()
        )
    return parse(text, name)


def names() -> list[str]:
    """作業ツリーにあるspecの名前。"""
    return sorted(p.stem for p in (paths.REPO / DIR).glob("*.toml"))


def parse(text: str, name: str) -> Spec:
    data = tomllib.loads(text) if tomllib else parse_subset(text)
    adr = data.get("adr")
    if not isinstance(adr, str) or not ADR_RE.match(adr):
        raise Invalid(f'{name}: adr は4桁の文字列で書く（例 adr = "0210"）')
    raw = data.get("step")
    if not isinstance(raw, list) or not raw:
        raise Invalid(f"{name}: [[step]] が1つも無い")

    steps: list[Step] = []
    for i, item in enumerate(raw, 1):
        sid, run = item.get("id"), item.get("run")
        if not isinstance(sid, str) or not isinstance(run, str):
            raise Invalid(f"{name}: {i}番目のステップに id と run が要る")
        try:
            paths.check_name(sid)
        except paths.BadName as e:
            raise Invalid(f"{name}: ステップのid: {e}") from e
        if any(s.id == sid for s in steps):
            raise Invalid(f"{name}: ステップのidが重複している: {sid}")
        steps.append(_checked(Step(sid, run), name))
    return Spec(name, adr, tuple(steps))


def _checked(step: Step, name: str) -> Step:
    try:
        words = shlex.split(step.run)
    except ValueError as e:
        raise Invalid(f"{name}/{step.id}: 引用符が閉じていない: {e}") from e
    if not words or words[0] != "hmwr":
        raise Invalid(
            f"{name}/{step.id}: ステップは hmwr のコマンドで書く: {step.run}\n"
            "足りない操作は hmwr へ足す"
        )
    if "--dry-run" in words:
        raise Invalid(f"{name}/{step.id}: specに --dry-run は書かない")
    if tuple(words[1:3]) in DETACHING and "--foreground" not in words:
        raise Invalid(
            f"{name}/{step.id}: 対局のステップには --foreground が要る。"
            "切り離して起動すると、終わる前に次のステップが走る"
        )
    return step


def parse_subset(text: str) -> dict:
    """TOMLのうちspecが使う部分だけを読む。tomllibが無いPython向け。

    読めるのは、コメント、`key = "文字列"`、`[[表の配列]]` の3つである。
    tomllibがある環境では、テストがこの読み取りと突き合わせる。
    """
    root: dict = {}
    current = root
    for n, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[[") and line.endswith("]]"):
            current = {}
            root.setdefault(line[2:-2].strip(), []).append(current)
            continue
        m = re.match(r'^([A-Za-z_][A-Za-z0-9_-]*)\s*=\s*"((?:[^"\\]|\\.)*)"\s*(?:#.*)?$', line)
        if not m:
            raise Invalid(f'{n}行目を読めない（key = "文字列" か [[step]] で書く）: {raw}')
        current[m.group(1)] = re.sub(r"\\(.)", r"\1", m.group(2))
    return root


def adr_file(adr: str) -> Path | None:
    found = sorted((paths.REPO / "docs" / "adr").glob(f"{adr}-*.md"))
    return found[0] if found else None
