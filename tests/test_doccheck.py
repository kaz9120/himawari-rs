"""文書の整合の検査を検証する（ADR-0209）。"""

import pytest

from hmwr import cli, doccheck, proc

INDEX = """# ADR索引

| 番号 | 題 | 日付 | 置換 | Status |
|---|---|---|---|---|
| [0001](0001-a.md) | A | 2026-01-01 |  | accepted |
| [0002](0002-b.md) | B | 2026-01-02 |  | proposed（保留。注力の外） |
"""


@pytest.fixture
def repo(tmp_path):
    adr = tmp_path / "docs" / "adr"
    adr.mkdir(parents=True)
    (adr / "README.md").write_text(INDEX, encoding="utf-8")
    (adr / "0001-a.md").write_text("# 0001: A\n\n- Status: accepted（実装済み）\n", encoding="utf-8")
    (adr / "0002-b.md").write_text("# 0002: B\n\n- Status: proposed\n", encoding="utf-8")
    return tmp_path


def messages(found):
    return [str(f) for f in found]


def test_a_consistent_repository_has_no_findings(repo):
    assert doccheck.run_all(repo) == []


def test_status_is_compared_by_its_first_word(repo):
    """補足の括弧は本文と索引で違ってよい。比べるのは先頭の語だけにする。"""
    (repo / "docs/adr/0002-b.md").write_text("- Status: rejected\n", encoding="utf-8")
    found = messages(doccheck.check_index(repo))
    assert len(found) == 1 and "Statusが索引と違う" in found[0]


def test_an_adr_missing_from_the_index_is_found(repo):
    (repo / "docs/adr/0003-c.md").write_text("- Status: proposed\n", encoding="utf-8")
    assert any("索引の表に行がない" in m for m in messages(doccheck.check_index(repo)))


def test_an_index_row_without_a_file_is_found(repo):
    (repo / "docs/adr/0002-b.md").unlink()
    assert any("ADRのファイルがない" in m for m in messages(doccheck.check_index(repo)))


def test_a_renamed_file_is_found_through_the_index_link(repo):
    (repo / "docs/adr/0002-b.md").rename(repo / "docs/adr/0002-renamed.md")
    assert any("リンク先がファイル名と違う" in m for m in messages(doccheck.check_index(repo)))


def test_dead_relative_links_are_found(repo):
    (repo / "docs/adr/0001-a.md").write_text(
        "- Status: accepted\n\n"
        "[ある](0002-b.md) [節つき](0002-b.md#decision) [無い](0159-ft1024-revisit.md)\n"
        "[外](https://example.com/x.md) [ページ内](#context) [GitHub上](../../../releases)\n"
        "`[コードの中](nothing.md)`\n\n```\n[柵の中](nothing.md)\n```\n",
        encoding="utf-8",
    )
    found = messages(doccheck.check_links(repo))
    assert found == ["docs/adr/0001-a.md: リンク先がない: 0159-ft1024-revisit.md"]


def test_the_command_reports_findings_with_the_judge_code(repo, monkeypatch, capsys):
    monkeypatch.setattr(doccheck.paths, "REPO", repo)
    assert cli.main(["doc", "check"]) == proc.OK
    (repo / "docs/adr/0002-b.md").write_text("- Status: accepted\n", encoding="utf-8")
    assert cli.main(["doc", "check"]) == proc.JUDGE
    assert "1件の食い違い" in capsys.readouterr().out


def test_this_repository_is_consistent():
    """リポジトリの文書そのものを検査する。CIでの検査を兼ねる。"""
    assert messages(doccheck.run_all()) == []
