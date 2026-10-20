"""PRの作成とマージを検証する（ADR-0070）。GitHubには触らない。"""

import json

import pytest

from hmwr import cli, proc, sprt_log
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


# --- マージの検査（issue #764） ----------------------------------------

TRAILERS = "Co-Authored-By: x <x@example.com>"
H1 = {
    "decision": "H1", "elo": "+5.8", "ci_low": "+1.2", "ci_high": "+10.4",
    "games": "20496", "elo0": "0", "elo1": "5",
}  # fmt: skip
TRAILER = "SPRT: +5.8 [+1.2,+10.4] 20496games H1"


def _pr(*commits, title="feat: x", checks=("SUCCESS",)):
    return {
        "number": 9,
        "title": title,
        "state": "OPEN",
        "isDraft": False,
        "headRefOid": "abc",
        "commits": [{"messageHeadline": h, "messageBody": b} for h, b in commits],
        "statusCheckRollup": [{"conclusion": c} for c in checks],
    }


def test_single_commit_squash_uses_the_commit_message():
    subject, body = pr.squash_message(_pr(("fix: y", "本文\n\n" + TRAILERS)))
    assert subject == "fix: y (#9)"
    assert body == "本文\n\n" + TRAILERS


def test_many_commits_squash_lists_them_under_the_title():
    subject, body = pr.squash_message(_pr(("fix: a", "A"), ("fix: b", "B"), title="fix: t"))
    assert subject == "fix: t (#9)"
    assert body == "* fix: a\n\nA\n\n* fix: b\n\nB"


def test_nested_parens_in_the_body_stop_the_merge():
    found = pr.problems("chore", "chore: x (#9)", "値は (clip(a)+1) になる", None)
    assert any("入れ子の半角括弧" in f for f in found)


def test_full_width_and_single_parens_pass():
    assert pr.problems("chore", "chore: x（ADR-0001） (#9)", "`f(x)` を（全角で）呼ぶ", None) == []


def test_chore_must_not_be_feat():
    assert pr.problems("chore", "feat: x (#9)", "", None)


def test_strength_needs_h1_and_a_matching_trailer():
    body = f"本文\n\n{TRAILER}\n{TRAILERS}"
    assert pr.problems("strength", "feat: x（+5.8 Elo） (#9)", body, H1) == []
    assert pr.problems("strength", "feat: x (#9)", body, {**H1, "decision": "H0"})
    assert pr.problems("strength", "feat: x (#9)", body, None)
    # 数字の転記ミスは止める
    wrong = body.replace("+5.8", "+5.9")
    assert any("トレーラ" in f for f in pr.problems("strength", "feat: x (#9)", wrong, H1))


def test_a_trailer_buried_in_an_earlier_paragraph_is_refused():
    # 複数コミットのsquashで最初のコミットにだけトレーラがあると、CHANGELOGへ載らない
    body = f"* feat: a\n\n{TRAILER}\n\n* fix: b\n\n{TRAILERS}"
    assert any("トレーラ" in f for f in pr.problems("strength", "feat: x (#9)", body, H1))


def test_non_inferiority_trailer_carries_the_condition():
    result = {**H1, "elo0": "-5", "elo1": "0"}
    assert pr.expected_trailer(result) == TRAILER + "（非劣性 elo0=-5 elo1=0）"


@pytest.mark.parametrize(
    "hyp",
    [("0", "5"), ("-5", "0"), ("0", "10"), None],
)
def test_the_expected_trailer_matches_what_the_log_writes(hyp):
    """検査する側とトレーラを書く側で、注記の組み立てを1本にする。

    別々に持つと、加速SPRT（elo1=10）のように既定でも非劣性でもない
    対立仮説で食い違い、正しいトレーラがマージで弾かれる。
    """
    result = {k: v for k, v in H1.items() if k not in ("elo0", "elo1")}
    if hyp is not None:
        result["elo0"], result["elo1"] = hyp
    assert pr.expected_trailer(result) == TRAILER + sprt_log.hypothesis_note(hyp)


@pytest.mark.parametrize(
    "checks,green",
    [
        (("SUCCESS", "SKIPPED"), True),
        (("SUCCESS", "FAILURE"), False),
        (("SUCCESS", ""), False),
        ((), False),
    ],
)
def test_ci_must_be_green(checks, green):
    assert pr.ci_green(_pr(("fix: a", ""), checks=checks)) is green


def test_dry_run_checks_but_does_not_merge(monkeypatch, capsys):
    data = _pr(("fix: a", "本文 (clip(a)) だめ"))
    monkeypatch.setattr(proc, "capture_code", lambda argv, **kw: (0, json.dumps(data), ""))
    assert cli.main(["--dry-run", "pr", "merge", "9", "--kind", "chore"]) == proc.JUDGE
    assert "入れ子の半角括弧" in capsys.readouterr().err

    data["commits"][0]["messageBody"] = "本文"
    assert cli.main(["--dry-run", "pr", "merge", "9", "--kind", "chore"]) == proc.OK
    out = capsys.readouterr().out
    assert "[dry-run] gh pr merge 9 --squash --delete-branch --match-head-commit abc" in out


def test_strength_requires_the_sprt_name():
    assert cli.main(["--dry-run", "pr", "merge", "9", "--kind", "strength"]) == proc.USAGE
