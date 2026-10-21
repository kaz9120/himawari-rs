"""Stopフックのコミット検査を検証する（ADR-0181、issue #764）。

違反は終了コード2で止め、検査そのものの不調は警告だけ出して0で抜ける。
cwdがずれても `CLAUDE_PROJECT_DIR` を基準に同じ対象を見る。
"""

import shutil
import subprocess
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parent.parent
HOOKS = REPO / ".claude" / "hooks"
SCRIPTS = ("branch_commits.sh", "check_commits.sh")
IDENT = ["-c", "user.name=t", "-c", "user.email=t@t"]


def _git(repo: Path, *args: str) -> None:
    subprocess.run(["git", "-C", str(repo), *IDENT, *args], check=True, capture_output=True)


def _install(root: Path) -> None:
    (root / ".claude" / "hooks").mkdir(parents=True)
    for name in SCRIPTS:
        shutil.copy(HOOKS / name, root / ".claude" / "hooks" / name)


@pytest.fixture
def repo(tmp_path):
    """main から分かれたブランチにいるリポジトリ。"""
    root = tmp_path / "repo"
    _install(root)
    _git(root, "init", "-q", "-b", "main")
    _git(root, "commit", "-q", "--allow-empty", "-m", "base")
    _git(root, "switch", "-q", "-c", "topic")
    return root


def _check(root: Path, mode: str, *, cwd: Path) -> subprocess.CompletedProcess:
    env = {
        "PATH": "/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin",
        "HOME": str(cwd),
        "CLAUDE_PROJECT_DIR": str(root),
    }
    return subprocess.run(
        ["sh", str(root / ".claude" / "hooks" / "check_commits.sh"), mode],
        cwd=cwd,
        env=env,
        capture_output=True,
        text=True,
    )


def test_nested_parens_stop_even_from_another_cwd(repo, tmp_path):
    _git(repo, "commit", "-q", "--allow-empty", "-m", "fix: x\n\n値は (clip(a)+1) になる")
    r = _check(repo, "parens", cwd=tmp_path)
    assert r.returncode == 2
    assert "入れ子の半角括弧" in r.stderr


def test_clean_body_passes(repo, tmp_path):
    _git(repo, "commit", "-q", "--allow-empty", "-m", "fix: x\n\n値は `clip` で切る（全角は通す）")
    r = _check(repo, "parens", cwd=tmp_path)
    assert (r.returncode, r.stderr) == (0, "")


def test_broken_check_warns_without_blocking(tmp_path):
    # gitの外では検査できない。終了は止めず、黙っても通さない
    _install(tmp_path)
    r = _check(tmp_path, "parens", cwd=tmp_path)
    assert r.returncode == 0
    assert "実行できなかった" in r.stderr


def test_missing_doclint_warns(repo, tmp_path):
    _git(repo, "commit", "-q", "--allow-empty", "-m", "fix: x")
    r = _check(repo, "subject", cwd=tmp_path)
    assert r.returncode == 0
    assert "hmwr/doclint.py がない" in r.stderr


def _install_doclint(root: Path) -> None:
    shutil.copytree(REPO / "hmwr" / "vendor", root / "hmwr" / "vendor")
    shutil.copy(REPO / "hmwr" / "doclint.py", root / "hmwr" / "doclint.py")


def test_subject_with_slop_word_stops(repo, tmp_path):
    _install_doclint(repo)
    _git(repo, "commit", "-q", "--allow-empty", "-m", "docs: 探索の土台を整える")
    r = _check(repo, "subject", cwd=tmp_path)
    assert r.returncode == 2
    assert "slop_vocabulary" in r.stderr


def test_clean_subject_passes(repo, tmp_path):
    _install_doclint(repo)
    _git(repo, "commit", "-q", "--allow-empty", "-m", "fix: 台帳へ足すときの一時ファイルを減らす")
    r = _check(repo, "subject", cwd=tmp_path)
    assert (r.returncode, r.stderr) == (0, "")
