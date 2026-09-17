"""PRの作成を検証する（ADR-0070）。GitHubには触らない。"""

import pytest

from hmwr import cli, proc
from hmwr.commands import pr


def test_the_skeleton_drops_front_matter_and_guidance():
    text = pr.skeleton("chore")
    assert text.startswith("## 何を変えたか")
    assert "<!--" not in text and "about:" not in text
    assert "## チェック" in text


@pytest.mark.parametrize("kind", sorted(pr.KINDS))
def test_every_kind_has_headings(kind):
    assert "何を変えたか" in pr.headings(kind)


def test_a_body_missing_a_section_is_refused(tmp_path, capsys):
    body = tmp_path / "body.md"
    body.write_text("## 何を変えたか\n\n変えた。\n", encoding="utf-8")
    argv = ["--dry-run", "pr", "create", "--kind", "chore", "--title", "chore: x"]
    assert cli.main([*argv, "--body-file", str(body)]) == proc.USAGE
    assert "棋力に影響しない根拠" in capsys.readouterr().err


def test_a_complete_body_reaches_gh(tmp_path, capsys):
    body = tmp_path / "body.md"
    body.write_text(pr.skeleton("chore"), encoding="utf-8")
    argv = ["--dry-run", "pr", "create", "--kind", "chore", "--body-file", str(body)]
    assert cli.main([*argv, "--title", "chore: x", "--draft"]) == proc.OK
    out = capsys.readouterr().out
    assert "gh pr create --title chore: x --body-file" in out and "--draft" in out


def test_the_title_needs_a_conventional_type(tmp_path):
    body = tmp_path / "body.md"
    body.write_text(pr.skeleton("chore"), encoding="utf-8")
    argv = ["--dry-run", "pr", "create", "--kind", "chore", "--body-file", str(body)]
    assert cli.main([*argv, "--title", "いろいろ直す"]) == proc.USAGE
