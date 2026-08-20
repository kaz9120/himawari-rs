"""外部プロセスの起動と、失敗の伝え方をひとつにまとめる。

shellで各スクリプトが自前に持っていた3つ（ログへの記録、予行演習、
終了コードの扱い）をここへ集める。**同じ処理を各所に持たせないことが、
書き方のばらつきを防ぐ唯一の方法である。**
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

from . import paths

# 終了コードの規約
OK = 0
JUDGE = 1  # 正常だが「進むな」を意味する判定（機能検証の全一致など）
USAGE = 2
RUNTIME = 3


class Fail(Exception):
    """メッセージと終了コードを持つ失敗。"""

    def __init__(self, message: str, code: int = RUNTIME):
        super().__init__(message)
        self.code = code


def show(argv: list[str], env: dict[str, str] | None = None) -> str:
    """実行するコマンドを1行で表す。パスは相対にして読みやすくする。"""
    text = " ".join(paths.rel(a) for a in argv)
    if env:
        prefix = " ".join(f"{k}={paths.rel(v)}" for k, v in sorted(env.items()))
        text = f"{prefix} {text}"
    return text


def run(
    argv: list[str],
    *,
    dry_run: bool = False,
    env: dict[str, str] | None = None,
    log: Path | None = None,
    allowed: tuple[int, ...] = (OK,),
    cwd: Path | None = None,
) -> int:
    """外部コマンドを実行する。

    dry_runなら実行せず、走るはずのコマンドを表示して返る。logを渡すと
    出力を端末とファイルの両方へ流す（追記）。allowedにない終了コードは
    Failとして投げる。
    """
    line = show(argv, env)
    if dry_run:
        print(f"[dry-run] {line}")
        if log:
            print(f"[dry-run] ログ: {paths.rel(log)}")
        return OK

    print(f"$ {line}", flush=True)
    full_env = dict(os.environ)
    if env:
        full_env.update(env)
    workdir = str(cwd or paths.REPO)

    if log is None:
        code = subprocess.call(argv, cwd=workdir, env=full_env)
    else:
        print(f"ログ: {paths.rel(log)}", flush=True)
        code = _tee(argv, workdir, full_env, log, line)

    if code not in allowed:
        raise Fail(f"失敗した（終了コード {code}）: {line}", code)
    return code


def _tee(argv: list[str], cwd: str, env: dict[str, str], log: Path, header: str) -> int:
    """出力を端末とログの両方へ流す。"""
    with open(log, "ab") as fh:
        fh.write(f"\n=== {header} ===\n".encode())
        proc = subprocess.Popen(
            argv,
            cwd=cwd,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        )
        assert proc.stdout is not None
        for chunk in proc.stdout:
            sys.stdout.buffer.write(chunk)
            sys.stdout.flush()
            fh.write(chunk)
        return proc.wait()


def capture(argv: list[str], *, cwd: Path | None = None) -> str:
    """出力を取り込む。失敗しても投げず、空文字を返す。"""
    try:
        result = subprocess.run(
            argv,
            cwd=str(cwd or paths.REPO),
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError:
        return ""
    return result.stdout


def succeeds(argv: list[str], *, cwd: Path | None = None) -> bool:
    """終了コードだけを見る。出力は捨てる。"""
    try:
        return (
            subprocess.run(
                argv,
                cwd=str(cwd or paths.REPO),
                capture_output=True,
                check=False,
            ).returncode
            == 0
        )
    except OSError:
        return False


def git(*args: str) -> str:
    """gitの出力を取り込む。改行は落とす。"""
    return capture(["git", *args]).strip()


def script(name: str) -> str:
    """scripts/ のスクリプトのパス。移行が終われば呼び出しは消える。"""
    path = paths.SCRIPTS / name
    if not path.is_file():
        raise Fail(f"スクリプトがない: {paths.rel(path)}")
    return str(path)


def cargo_tool(binary: str, args: list[str]) -> list[str]:
    """crates/tools のバイナリを走らせるコマンドを組み立てる。

    毎回 `cargo run --release -q -p himawari-tools --bin <名前> --` と
    書かずに済ませることが目的である。
    """
    return [
        "cargo",
        "run",
        "--release",
        "-q",
        "-p",
        "himawari-tools",
        "--bin",
        binary,
        "--",
        *args,
    ]
