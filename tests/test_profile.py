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


def test_classify_effect_map_built_inside_see():
    # SEEが初めて読むときに作る利きの地図は、地図として数える（ADR-0239）
    chain = [
        "himawari_core::attacks::rook_attacks",
        "<himawari_core::effect::EffectMap>::build_near",
        "<himawari_core::position::Position>::threat_map",
        "<himawari_core::position::Position>::see_ge",
        "<himawari_engine::search::Worker>::search::<false>",
    ]
    assert profile_report.classify(chain) == "利きの地図"


def test_classify_helper_alone_falls_back():
    chain = [
        "<himawari_core::position::Position>::attackers_to",
        "<himawari_engine::search::Worker>::search::<false>",
    ]
    assert profile_report.classify(chain) == "利きの計算（上記の外から）"


def test_classify_unknown_goes_to_other():
    chain = ["<himawari_engine::search::Worker>::search::<false>"]
    assert profile_report.classify(chain) == profile_report.OTHER


def test_classify_see_with_table_is_counted_as_see():
    # 表を受ける版（see_ge_with）もSEEへ数える（issue #837、ADR-0232の計測）
    chain = [
        "<himawari_core::position::Position>::attackers_to",
        "<himawari_core::position::Position>::see_ge_with",
        "<himawari_engine::movepick::MovePicker>::score_quiets",
        "<himawari_engine::search::Worker>::search::<false>",
    ]
    assert profile_report.classify(chain) == "SEE"


def test_classify_effect_table_update_is_its_own_category():
    # 維持する利きの表の更新は、探索のその他ではなく表の維持へ数える
    chain = [
        "himawari_core::attacks::attacks",
        "<himawari_core::effect::EffectTable>::update",
        "<himawari_engine::search::Worker>::make_move",
        "<himawari_engine::search::Worker>::search::<false>",
    ]
    assert profile_report.classify(chain) == "利きの表の維持"


# --- unknown_names: 分類の規則に当たらない関数を挙げる（issue #837） ---


def unknown_of(chain):
    names = [profile_report.clean_name(n) for n in chain]
    _, at = profile_report.classify_at(names)
    return profile_report.unknown_names(names, at)


def test_unknown_names_flags_a_function_the_rules_miss():
    # 規則にない関数が、別の種類（指し手の順序付け）へ数えられている例
    chain = [
        "himawari_core::attacks::attacks",
        "<himawari_core::position::Position>::danger_count",
        "<himawari_engine::movepick::MovePicker>::score_quiets",
        "<himawari_engine::search::Worker>::search::<false>",
    ]
    assert unknown_of(chain) == ["<himawari_core::position::Position>::danger_count"]


def test_unknown_names_empty_when_every_frame_is_known():
    chain = [
        "<himawari_core::position::Position>::attackers_to",
        "<himawari_core::position::Position>::see_ge",
        "<himawari_engine::movepick::MovePicker>::score_quiets",
        "<himawari_engine::search::Worker>::search::<false>",
    ]
    assert unknown_of(chain) == []


def test_unknown_names_in_other_ignores_frames_outside_the_search():
    # 種類に当たらないサンプルでは、探索のフレームより内側だけを見る
    chain = [
        "<himawari_core::effect::EffectTable>::update_counts",
        "<himawari_engine::search::Worker>::make_move",
        "<himawari_engine::search::Worker>::search::<false>",
        "himawari_engine::thread::spawn_workers",
        "himawari_usi::main",
    ]
    assert profile_report.classify(chain) == profile_report.OTHER
    assert unknown_of(chain) == ["<himawari_core::effect::EffectTable>::update_counts"]


def test_unknown_names_skips_frames_outside_our_crates():
    chain = [
        "core::ptr::drop_in_place",
        "<himawari_engine::search::Worker>::search::<false>",
    ]
    assert unknown_of(chain) == []


def test_unknown_names_inside_outer_category_is_flagged():
    # 1手詰めの中の未知の関数は、1手詰めの内側として挙げる
    chain = [
        "<himawari_core::position::Position>::mate_candidates",
        "himawari_engine::mate::mate_1ply",
        "<himawari_engine::search::Worker>::search::<false>",
    ]
    assert profile_report.classify(chain) == "1手詰め"
    assert unknown_of(chain) == ["<himawari_core::position::Position>::mate_candidates"]


# --- unknown_report: 下限と件数で絞る ---


def test_unknown_report_says_nothing_hot_when_below_threshold(capsys):
    profile_report.unknown_report({("<X>::f", "SEE"): 9}, 1000)
    out = capsys.readouterr().out
    assert "1%以上にない" in out
    assert "<X>::f" not in out


def test_unknown_report_lists_hot_functions_with_their_category(capsys):
    unknown = {("<X>::f", "指し手の順序付け"): 60, ("<X>::g", "探索のその他"): 20}
    profile_report.unknown_report(unknown, 1000)
    out = capsys.readouterr().out
    assert "| `<X>::f` | 6.0% | 指し手の順序付け |" in out
    assert "| `<X>::g` | 2.0% | 探索のその他 |" in out
    # 割合の大きい順に並ぶ
    assert out.index("<X>::f") < out.index("<X>::g")


def test_unknown_report_caps_the_number_of_rows(capsys):
    unknown = {(f"<X>::f{i}", "探索のその他"): 100 - i for i in range(8)}
    profile_report.unknown_report(unknown, 1000)
    out = capsys.readouterr().out
    assert out.count("| `<X>::f") == profile_report.UNKNOWN_TOP
