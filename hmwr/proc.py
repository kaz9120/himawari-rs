"""外部プロセスの起動と、失敗の伝え方をひとつにまとめる。

shellで各スクリプトが自前に持っていた3つ（ログへの記録、予行演習、
終了コードの扱い）をここへ集める。**同じ処理を各所に持たせないことが、
書き方のばらつきを防ぐ唯一の方法である。**
"""

from __future__ import annotations

import os
import subprocess
import sys
from collections.abc import Callable
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
    stdin_text: str | None = None,
    on_line: Callable[[str], None] | None = None,
) -> int:
    """外部コマンドを実行する。

    dry_runなら実行せず、走るはずのコマンドを表示して返る。logを渡すと
    出力を端末とファイルの両方へ流す（追記）。allowedにない終了コードは
    Failとして投げる。stdin_textを渡すと標準入力へ流し込む（USIエンジンの
    ように行を食わせて動かすコマンド向け）。on_lineを渡すと、出力の各行で
    呼ぶ（心拍の更新用。logを渡したときだけ効く）。
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
        code = subprocess.run(
            argv, cwd=workdir, env=full_env, input=stdin_text, text=True, check=False
        ).returncode
    else:
        print(f"ログ: {paths.rel(log)}", flush=True)
        code = _tee(argv, workdir, full_env, log, line, stdin_text, on_line)

    if code not in allowed:
        raise Fail(f"失敗した（終了コード {code}）: {line}", code)
    return code


def _tee(
    argv: list[str],
    cwd: str,
    env: dict[str, str],
    log: Path,
    header: str,
    stdin_text: str | None = None,
    on_line: Callable[[str], None] | None = None,
) -> int:
    """出力を端末とログの両方へ流す。"""
    with open(log, "ab") as fh:
        fh.write(f"\n=== {header} ===\n".encode())
        proc = subprocess.Popen(
            argv,
            cwd=cwd,
            env=env,
            stdin=subprocess.PIPE if stdin_text is not None else None,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        )
        if stdin_text is not None:
            assert proc.stdin is not None
            # 相手が読み終える前に閉じると壊れるので、書いてから閉じる
            proc.stdin.write(stdin_text.encode())
            proc.stdin.close()
        assert proc.stdout is not None
        for chunk in proc.stdout:
            sys.stdout.buffer.write(chunk)
            sys.stdout.flush()
            fh.write(chunk)
            if on_line is not None:
                on_line(chunk.decode("utf-8", errors="replace").rstrip("\n"))
        return proc.wait()


def run_all(argvs: list[list[str]], *, log: Path) -> None:
    """複数のコマンドを並列に走らせ、全部の終了を待つ。

    出力は端末へ流さず、ログへ追記する。行は混ざるが、単スレッドの道具を
    区間で割って回す用途なので、読むのは件数の要約だけで足りる。1つでも
    失敗したらFailを投げる。
    """
    print(f"ログ: {paths.rel(log)}", flush=True)
    with open(log, "ab") as fh:
        procs = []
        for argv in argvs:
            line = show(argv)
            print(f"$ {line}", flush=True)
            fh.write(f"\n=== {line} ===\n".encode())
            fh.flush()
            procs.append(
                subprocess.Popen(argv, cwd=str(paths.REPO), stdout=fh, stderr=subprocess.STDOUT)
            )
        codes = [p.wait() for p in procs]
    bad = [show(a) for a, c in zip(argvs, codes) if c != OK]
    if bad:
        raise Fail("失敗した: " + " / ".join(bad))


def capture(argv: list[str], *, cwd: Path | None = None) -> str:
    """出力を取り込む。失敗しても投げず、空文字を返す。"""
    return capture_both(argv, cwd=cwd)[0]


def capture_both(argv: list[str], *, cwd: Path | None = None) -> tuple[str, str]:
    """標準出力と標準エラーを組で返す。

    **失敗したときに原因を見せるために、標準エラーも取る。** 捨てると
    「測れない」だけが残り、何が起きたか分からなくなる。
    """
    try:
        result = subprocess.run(
            argv,
            cwd=str(cwd or paths.REPO),
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError as e:
        return "", str(e)
    return result.stdout, result.stderr


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
