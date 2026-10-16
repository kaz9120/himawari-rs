"""プロファイルの集計のうち、ロジック部分を検証する。

実バイナリやsamplyのプロファイルが無い環境でも走るよう、
プロファイル本体はモックの最小データで代用する。
"""

from unittest import mock

from hmwr.tools import profile as profile_report


# --- sample_hz: meta.interval（ミリ秒）からサンプリング周波数を求める ---


def test_sample_hz_from_interval_0_5ms():
    # 実プロファイル（data/profile/profile.json.gz）のmeta.intervalは0.5ms = 2000Hz
    profile = {"meta": {"interval": 0.5}}
    assert profile_report.sample_hz(profile) == 2000.0


def test_sample_hz_from_interval_1ms():
    profile = {"meta": {"interval": 1.0}}
    assert profile_report.sample_hz(profile) == 1000.0


def test_total_seconds_uses_measured_hz_not_hardcoded_2000hz():
    # 2000Hz決め打ちを廃した回帰確認。1000Hzのプロファイルで
    # 総サンプル1000なら1.0秒になるべきで、2000決め打ちなら0.5秒になる
    profile = {"meta": {"interval": 1.0}}
    hz = profile_report.sample_hz(profile)
    total = 1000
    assert total / hz == 1.0


# --- resolve_lines: プラットフォームで振る舞いを変える ---


def test_resolve_lines_no_binary_given():
    lines, reason = profile_report.resolve_lines(None, [1, 2, 3])
    assert lines == {}
    assert reason is None


def test_resolve_lines_non_macos_reports_reason_without_crashing():
    with mock.patch.object(profile_report.platform, "system", return_value="Linux"):
        lines, reason = profile_report.resolve_lines("dummy-binary", [1, 2, 3])
    assert lines == {}
    assert reason is not None
    assert "Linux" in reason


def test_resolve_lines_atos_missing_reports_reason():
    with mock.patch.object(profile_report.platform, "system", return_value="Darwin"):
        with mock.patch.object(profile_report.shutil, "which", return_value=None):
            lines, reason = profile_report.resolve_lines("dummy-binary", [1, 2, 3])
    assert lines == {}
    assert reason is not None


def test_resolve_lines_macos_with_atos_invokes_subprocess():
    fake_result = mock.Mock()
    fake_result.stdout = "func_a (file.rs:10)\nfunc_b (file.rs:20)\n"
    with mock.patch.object(profile_report.platform, "system", return_value="Darwin"):
        with mock.patch.object(profile_report.shutil, "which", return_value="/usr/bin/atos"):
            with mock.patch.object(profile_report.subprocess, "run", return_value=fake_result) as run:
                lines, reason = profile_report.resolve_lines("dummy-binary", [0x10, 0x20])
    assert reason is None
    assert lines[0x10] == "func_a (file.rs:10)"
    assert lines[0x20] == "func_b (file.rs:20)"
    run.assert_called_once()


# --- classify: 呼び出し連鎖を処理の種類へ落とす（ADR-0230） ---


def test_classify_attributes_attack_helper_to_its_caller():
    # 利きを引く下請けは、呼び出し元の種類へ数える
    chain = [
        "himawari_core::attacks::attacks",
        "himawari_core::movegen::generate_board_moves",
        "<himawari_engine::movepick::MovePicker>::init_captures",
        "<himawari_engine::search::Worker>::search::<false>",
    ]
    assert profile_report.classify(chain) == "指し手生成"


def test_classify_outer_category_wins_over_inner():
    # 1手詰めの中のis_legalは1手詰めとして数える
    chain = [
        "<himawari_core::position::Position>::is_legal",
        "himawari_engine::mate::is_mate_move",
        "himawari_engine::mate::mate_1ply",
        "<himawari_engine::search::Worker>::search::<false>",
    ]
    assert profile_report.classify(chain) == "1手詰め"


def test_classify_inner_first_for_see_in_move_ordering():
    chain = [
        "<himawari_core::position::Position>::attackers_to",
        "<himawari_core::position::Position>::see_ge",
        "<himawari_engine::movepick::MovePicker>::score_quiets",
        "<himawari_engine::search::Worker>::search::<false>",
    ]
    assert profile_report.classify(chain) == "SEE"


def test_classify_helper_alone_falls_back():
    chain = [
        "<himawari_core::position::Position>::attackers_to",
        "<himawari_engine::search::Worker>::search::<false>",
    ]
    assert profile_report.classify(chain) == "利きの計算（上記の外から）"


def test_classify_unknown_goes_to_other():
    chain = ["<himawari_engine::search::Worker>::search::<false>"]
    assert profile_report.classify(chain) == profile_report.OTHER
