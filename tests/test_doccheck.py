"""文書の相対リンクの検査を検証する。"""

import pytest

from hmwr import cli, doccheck, proc


@pytest.fixture
def repo(tmp_path):
    docs = tmp_path / "docs"
    docs.mkdir()
    (docs / "a.md").write_text("# A\n", encoding="utf-8")
    (docs / "b.md").write_text("# B\n", encoding="utf-8")
    return tmp_path


def messages(found):
    return [str(f) for f in found]


def test_a_consistent_repository_has_no_findings(repo):
    assert doccheck.run_all(repo) == []


def test_dead_relative_links_are_found(repo):
    (repo / "docs/a.md").write_text(
        "[ある](b.md) [節つき](b.md#decision) [無い](missing.md)\n"
        "[外](https://example.com/x.md) [ページ内](#context) [GitHub上](../../../releases)\n"
        "`[コードの中](nothing.md)`\n\n```\n[柵の中](nothing.md)\n```\n",
        encoding="utf-8",
    )
    found = messages(doccheck.check_links(repo))
    assert found == ["docs/a.md: リンク先がない: missing.md"]


def test_the_command_reports_findings_with_the_judge_code(repo, monkeypatch, capsys):
    monkeypatch.setattr(doccheck.paths, "REPO", repo)
    assert cli.main(["doc", "check"]) == proc.OK
    (repo / "docs/a.md").write_text("[無い](missing.md)\n", encoding="utf-8")
    assert cli.main(["doc", "check"]) == proc.JUDGE
    assert "1件の食い違い" in capsys.readouterr().out


def test_this_repository_is_consistent():
    """リポジトリの文書そのものを検査する。CIでの検査を兼ねる。"""
    assert messages(doccheck.run_all()) == []
