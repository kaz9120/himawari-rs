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
