"""たまった成果物の掃除（ADR-0189）。

保持は日数で決める。例外は現行の評価関数の系列だけで、名前の一覧を
持たない。何を残すかを列挙し始めると、一覧の手入れが新しいゴミになる。
"""

from __future__ import annotations

import argparse
import shutil
import time
from pathlib import Path

from .. import config, paths, proc


def add_parser(sub: argparse._SubParsersAction) -> None:
    p = sub.add_parser(
        "clean",
        help="古い成果物を保持方針で掃除する",
        description="SPRTの棋譜・比較用バイナリ・ネット・チェックポイント・"
        "ログのうち、保持日数を過ぎたものを消す。現行の評価関数の系列と "
        "*.result（結果の要約）は残す。教師データ（data/train）は消さない。"
        "マージ済みブランチのworktreeも片付ける。"
        "既定は一覧だけを出す下見で、--apply を付けたときだけ消す。",
    )
    p.add_argument("--apply", action="store_true", help="実際に消す（既定は下見）")
    p.add_argument("--days", type=int, default=30, metavar="N", help="保持日数（既定30）")
    p.set_defaults(func=run)


def _size(path: Path) -> int:
    if path.is_dir():
        return sum(f.stat().st_size for f in path.rglob("*") if f.is_file())
    return path.stat().st_size


def _protected_stems() -> set[str]:
    """現行の評価関数の系列。並べ替え（ADR-0168）の前後どちらも守る。"""
    stem = Path(config.get("EVAL_FILE")).name.removesuffix(".hmwr")
    return {stem, stem.removesuffix("_reorder")}


def _candidates(days: int) -> list[tuple[str, Path]]:
    cutoff = time.time() - days * 86400
    stems = _protected_stems()

    def old(p: Path) -> bool:
        return p.stat().st_mtime < cutoff

    found: list[tuple[str, Path]] = []
    for p in sorted((paths.REPO / "data/sprt").glob("*.jsonl")):
        if old(p):
            found.append(("sprt棋譜", p))
    for p in sorted((paths.REPO / "data/bin").iterdir()):
        if old(p):
            found.append(("バイナリ", p))
    for p in sorted((paths.REPO / "data/nets").iterdir()):
        name = p.name.removesuffix(".best").removesuffix(".hmwr")
        if old(p) and name not in stems:
            found.append(("ネット", p))
    ckpt = paths.REPO / "training/checkpoints"
    if ckpt.is_dir():
        for p in sorted(ckpt.iterdir()):
            if old(p) and p.name not in stems:
                found.append(("チェックポイント", p))
    for p in sorted((paths.REPO / "data/logs").iterdir()):
        if old(p):
            found.append(("ログ", p))
    for p in sorted((paths.REPO / "data/train").glob("*.stop")):
        if old(p):
            found.append(("停止ファイル", p))
    found += _merged_worktrees()
    return found


def _prune_worktrees(*, apply: bool) -> None:
    """登録だけ残ってディレクトリが消えたworktreeを片付ける。

    gitdirの指す先がないものはgitが `prunable` と印を付ける。中身がない
    ので消して困るものはなく、日数もマージ状態も関係しない。
    """
    stale = [
        line for line in proc.git("worktree", "list", "--porcelain").splitlines()
        if line.startswith("prunable")
    ]
    if not stale:
        return
    print(f"登録だけ残ったworktree: {len(stale)}件")
    if apply:
        proc.run(["git", "worktree", "prune"])


def _merged_worktrees() -> list[tuple[str, Path]]:
    """役目を終えたworktree。

    日数では決めない。ブランチがmainへ入っていれば、その作業ツリーは
    いつ作ったかによらず用済みである。逆に、未コミットの変更が残って
    いれば古くても消せない。判断の材料が日数ではなくgitの状態にある。
    """
    main = paths.REPO.resolve()
    found: list[tuple[str, Path]] = []
    current = {}
    for line in proc.git("worktree", "list", "--porcelain").splitlines():
        if line.startswith("worktree "):
            current = {"path": Path(line[len("worktree ") :])}
        elif line.startswith("branch "):
            current["branch"] = line[len("branch ") :]
        elif not line.strip() and current.get("path"):
            _collect_worktree(current, main, found)
            current = {}
    if current.get("path"):
        _collect_worktree(current, main, found)
    return found


def _collect_worktree(wt: dict, main: Path, found: list[tuple[str, Path]]) -> None:
    path = wt["path"]
    if path.resolve() == main or not path.is_dir():
        return
    branch = wt.get("branch")
    if not branch:
        # detached HEADは意図が読めないので触らない
        return
    if not _is_merged(branch.removeprefix("refs/heads/")):
        return
    if proc.git("-C", str(path), "status", "--porcelain"):
        print(f"  （未コミットの変更があるので残す: {paths.rel(path)}）")
        return
    found.append(("worktree", path))


def _is_merged(branch: str) -> bool:
    """ブランチがmainへ入ったか。

    squashマージなのでコミットの祖先関係では判定できない。取り込まれた
    後もローカルのコミットはmainの祖先にならないためである。上流が消えて
    いれば済むが、--delete-branch を使わずマージしたPRでは残る。最後は
    GitHubへ聞く。聞けないときは「分からない」を「消さない」へ倒す。
    """
    if proc.git("rev-parse", "--verify", f"refs/remotes/origin/{branch}") == "":
        return True
    merged = proc.capture(
        ["gh", "pr", "list", "--head", branch, "--state", "merged",
         "--json", "number", "-q", ".[].number"]
    ).strip()
    return bool(merged)


def run(args: argparse.Namespace) -> int:
    found = _candidates(args.days)
    _prune_worktrees(apply=args.apply and not args.dry_run)
    if not found:
        print(f"保持{args.days}日を過ぎた成果物はない")
        return 0

    total = 0
    by_kind: dict[str, tuple[int, int]] = {}
    for kind, p in found:
        size = _size(p)
        total += size
        n, s = by_kind.get(kind, (0, 0))
        by_kind[kind] = (n + 1, s + size)
        print(f"  {kind}\t{size / 2**20:8.1f}MB\t{paths.rel(p)}")

    print()
    for kind, (n, size) in by_kind.items():
        print(f"{kind}: {n}件 {size / 2**30:.2f}GB")
    print(f"合計: {len(found)}件 {total / 2**30:.2f}GB")

    if not args.apply or args.dry_run:
        print("\n消すには --apply を付ける")
        return 0

    for kind, p in found:
        if kind == "worktree":
            # gitのメタデータも一緒に落とすので、rmtreeでは足りない
            proc.run(["git", "worktree", "remove", str(p)])
        elif p.is_dir():
            shutil.rmtree(p)
        else:
            p.unlink()
    print(f"\n{len(found)}件 {total / 2**30:.2f}GB を消した")
    return 0
