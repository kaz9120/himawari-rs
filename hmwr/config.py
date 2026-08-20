"""マシンごとに変わる設定と、測定の既定条件を持つ。

移行期間中は `scripts/env.sh` を正とし、その値をここへ読み込む。shellの
スクリプトがまだ env.sh を source しているため、値を二重に持つと片方だけが
古くなる。env.sh を使う最後のスクリプトが消えたら、ここへ直接書く。
"""

from __future__ import annotations

import os
import subprocess
from functools import lru_cache

from . import paths

# env.sh から取り込む値。ここに書いた順で読む
KEYS = (
    "SPRT_CONCURRENCY",
    "EVAL_FILE",
    "OPENINGS",
    "SPRT_TC",
    "SPRT_ELO0",
    "SPRT_ELO1",
    "SPRT_ALPHA",
    "SPRT_BETA",
    "SPRT_ADJUDICATE",
    "SPRT_MAX_PAIRS",
    "SPRT_HARD_MAX_PAIRS",
    "RUSTFLAGS_NATIVE",
)


@lru_cache(maxsize=1)
def shell_values() -> dict[str, str]:
    """env.sh がマシンに合わせて決めた値を読む。

    呼び出しは1回で済ませる。bashの起動は数十msかかり、コマンドごとに
    払う理由がない。
    """
    script = "source scripts/env.sh >/dev/null 2>&1 && " + "; ".join(
        f'printf "%s\\n" "${{{k}:-}}"' for k in KEYS
    )
    try:
        out = subprocess.run(
            ["bash", "-c", script],
            cwd=str(paths.REPO),
            capture_output=True,
            text=True,
            timeout=30,
        ).stdout.splitlines()
    except (OSError, subprocess.SubprocessError):
        out = []
    return {k: v for k, v in zip(KEYS, out) if v}


def get(key: str, default: str = "") -> str:
    """設定を1つ読む。環境変数で明示された値を優先する。"""
    if key in os.environ:
        return os.environ[key]
    return shell_values().get(key, default)


def rustflags() -> str:
    """計測・対局のビルドフラグ。ADR-0003の -C target-cpu=native を使う。"""
    return os.environ.get("RUSTFLAGS") or get("RUSTFLAGS_NATIVE", "-C target-cpu=native")


def measure_env() -> dict[str, str]:
    """Rust製の計測ツールへ渡す環境。

    `bench` と `verify` は評価関数の場所を EVAL_FILE から読む。shellから
    呼ぶときは `source scripts/env.sh` が要ったが、CLIから呼ぶときにそれを
    覚えているのは無理がある。ここが肩代わりする。
    """
    env = {"RUSTFLAGS": rustflags()}
    for key in ("EVAL_FILE", "OPENINGS"):
        if key not in os.environ:
            value = shell_values().get(key)
            if value:
                env[key] = value
    return env


def summary() -> list[tuple[str, str]]:
    """表示用の要約。測る前に条件を確かめるために使う。"""
    return [
        ("SPRT並列度", get("SPRT_CONCURRENCY", "?")),
        ("評価関数", paths.rel(get("EVAL_FILE", "（未設定）"))),
        ("開始局面", paths.rel(get("OPENINGS", "（未設定）"))),
        ("持ち時間", get("SPRT_TC", "?")),
        ("対立仮説", f'elo0={get("SPRT_ELO0", "?")} elo1={get("SPRT_ELO1", "?")}'),
        ("安全弁", f'{get("SPRT_HARD_MAX_PAIRS", "?")} ペア'),
        ("ビルドフラグ", rustflags()),
    ]
