"""対局ゲートのログ解析を検証する。

実ログの断片を固定文字列で埋め込み、判定行・途中経過行・数値抽出の挙動を
確かめる。**数字の転記はここが最後の砦なので、形式が変わったら落ちるように
しておく。**
"""

import pytest

from hmwr import sprt_log

START_DEFAULT = (
    "selfplay: cand vs base | tc 10+0.1 | 並列 8 | "
    "SPRT elo[0, 5] α=0.05 β=0.05 | 開始局面 30053件"
)
START_NONINF = (
    "selfplay: cand vs base | tc 10+0.1 | 並列 3 | "
    "SPRT elo[-5, 0] α=0.05 β=0.05 | 開始局面 30053件"
)


# --- 結果行の特定 ------------------------------------------------------


def test_finds_h1_verdict():
    lines = [
        "pairs   525 | +602 =46 -402 | [73,22,236,20,174] | "
        "Elo +67.0 [+46.4,+88.0] | LLR +3.05 [-2.94,2.94]",
        "H1採択（候補は有意に強い） | pairs 525 games 1050 | +602 =46 -402 | "
        "Elo +67.0 [+46.4,+88.0] | LLR +3.05",
    ]
    src, verdict = sprt_log.find_source_line(lines)
    assert verdict == "H1"
    assert src.startswith("H1採択")


def test_finds_h0_verdict():
    lines = [
        "H0採択（候補は有意に弱い） | pairs 100 games 200 | +50 =10 -140 | "
        "Elo -80.0 [-120.0,-40.0] | LLR -3.00"
    ]
    _, verdict = sprt_log.find_source_line(lines)
    assert verdict == "H0"


def test_uses_last_progress_line_without_verdict():
    lines = [
        "pairs   100 | +76 =17 -107 | [29,9,42,4,16] | "
        "Elo -54.3 [-102.9,-7.7] | LLR -0.51 [-2.94,2.94]",
        "pairs   346 | +313 =57 -322 | [72,21,160,30,63] | "
        "Elo -4.5 [-28.5,+19.4] | LLR -0.24 [-2.94,2.94]",
    ]
    src, verdict = sprt_log.find_source_line(lines)
    assert verdict == "判定前"
    assert src.startswith("pairs   346")


def test_takes_the_last_verdict_when_several_exist():
    lines = [
        "H0採択 | pairs 10 games 20 | +5 =0 -15 | Elo -50.0 [-60.0,-40.0] | LLR -3.0",
        "H1採択 | pairs 20 games 40 | +30 =0 -10 | Elo +50.0 [+40.0,+60.0] | LLR +3.0",
    ]
    _, verdict = sprt_log.find_source_line(lines)
    assert verdict == "H1"


def test_returns_none_when_nothing_matches():
    src, verdict = sprt_log.find_source_line(["何も一致しない行", ""])
    assert src is None
    assert verdict is None


# --- 数値の抽出 --------------------------------------------------------


def test_parses_verdict_line_with_games():
    src = (
        "H1採択（候補は有意に強い） | pairs 525 games 1050 | +602 =46 -402 | "
        "Elo +67.0 [+46.4,+88.0] | LLR +3.05"
    )
    assert sprt_log.parse_fields(src) == {
        "elo_num": "+67.0",
        "elo_ci": "[+46.4,+88.0]",
        "llr": "+3.05",
        "wdl": "+602 =46 -402",
        "games": 1050,
    }


def test_doubles_pairs_when_games_is_absent():
    src = (
        "pairs   117 | +77 =10 -147 | [42,7,56,3,9] | "
        "Elo -107.2 [-150.9,-66.6] | LLR -1.40 [-2.94,2.94]"
    )
    fields = sprt_log.parse_fields(src)
    assert fields["games"] == 234
    assert fields["elo_num"] == "-107.2"


def test_raises_when_required_field_is_missing():
    with pytest.raises(sprt_log.Unreadable):
        sprt_log.parse_fields("Eloもllrも無い行")


# --- 再開したログ ------------------------------------------------------


def test_drops_the_previous_run():
    """再開前の判定行を拾わない。ログは追記式である（ADR-0087）。"""
    lines = [
        START_DEFAULT,
        "pairs 100 | ...",
        "判定に至らず | pairs 100 games 200 | ...",
        START_NONINF,
        "pairs 200 | ...",
    ]
    tail = sprt_log.last_run_lines(lines)
    assert tail[0] == START_NONINF
    assert not any(line.startswith("判定に至らず") for line in tail)


def test_keeps_everything_without_a_start_line():
    lines = ["pairs 100 | ...", "pairs 200 | ..."]
    assert sprt_log.last_run_lines(lines) == lines


# --- 対立仮説の注記 ----------------------------------------------------


def test_reads_hypothesis_bounds():
    assert sprt_log.find_hypothesis([START_NONINF]) == ("-5", "0")


def test_takes_hypothesis_from_the_last_run():
    lines = [START_DEFAULT, "pairs 100 | ...", START_NONINF, "pairs 200 | ..."]
    assert sprt_log.find_hypothesis(lines) == ("-5", "0")


def test_hypothesis_is_none_when_absent():
    assert sprt_log.find_hypothesis(["起動行がない"]) is None


def test_note_is_empty_for_the_default_hypothesis():
    assert sprt_log.hypothesis_note(("0", "5")) == ""
    assert sprt_log.hypothesis_note(None) == ""


def test_note_names_non_inferiority():
    """条件を書かずに数値だけが独り歩きするのを防ぐ（ADR-0163）。"""
    assert sprt_log.hypothesis_note(("-5", "0")) == "（非劣性 elo0=-5 elo1=0）"


def test_note_falls_back_to_raw_bounds():
    assert sprt_log.hypothesis_note(("0", "10")) == "（elo0=0 elo1=10）"


# --- 報告の整形 --------------------------------------------------------


def _fields():
    return {
        "elo_num": "+11.2",
        "elo_ci": "[+3.7,+18.8]",
        "llr": "+2.92",
        "wdl": "+3819 =284 -3571",
        "games": 7674,
    }


def test_report_contains_trailer_and_tables():
    fields = {
        "elo_num": "+67.0",
        "elo_ci": "[+46.4,+88.0]",
        "llr": "+3.05",
        "wdl": "+602 =46 -402",
        "games": 1050,
    }
    out = sprt_log.build_report("adr0100", "H1", fields)
    assert out.splitlines()[0] == "=== adr0100（H1） ==="
    assert "SPRT: +67.0 [+46.4,+88.0] 1050games H1" in out
    assert "| adr0100 | **+67.0 [+46.4,+88.0]**（1050局、LLR +3.05でH1採択） |" in out
    assert "| 対局数 | 1050（525ペア） |" in out
    assert "| 判定 | **H1** |" in out


def test_report_distinguishes_cutoff_from_adoption():
    fields = {
        "elo_num": "+3.7",
        "elo_ci": "[-14.0,+21.4]",
        "llr": "+0.38",
        "wdl": "+577 =87 -564",
        "games": 1228,
    }
    out = sprt_log.build_report("adr0101", "打ち切り", fields)
    assert "（1228局、LLR +0.38で打ち切り） |" in out


def test_report_puts_note_into_the_trailer():
    out = sprt_log.build_report("adr0162", "H1", _fields(), "（非劣性 elo0=-5 elo1=0）")
    assert "SPRT: +11.2 [+3.7,+18.8] 7674games H1（非劣性 elo0=-5 elo1=0）" in out


def test_exit_codes_follow_the_verdict():
    assert sprt_log.EXIT_BY_VERDICT == {"H1": 0, "H0": 1, "打ち切り": 2, "判定前": 2}


# --- 結果ファイル ------------------------------------------------------


def test_writes_key_value_pairs(tmp_path):
    out = tmp_path / "adr0173.result"
    sprt_log.write_result(out, "adr0173", "H1", _fields(), ("0", "5"))

    got = dict(
        line.split("=", 1) for line in out.read_text(encoding="utf-8").splitlines()
    )
    assert got["name"] == "adr0173"
    assert got["decision"] == "H1"
    assert got["elo"] == "+11.2"
    assert got["ci_low"] == "+3.7"
    assert got["ci_high"] == "+18.8"
    assert got["games"] == "7674"
    assert got["llr"] == "+2.92"
    assert (got["elo0"], got["elo1"]) == ("0", "5")
    assert got["finished_at"].endswith("Z")


def test_records_the_non_inferiority_hypothesis(tmp_path):
    out = tmp_path / "adr0174.result"
    sprt_log.write_result(out, "adr0174", "H0", _fields(), ("-5", "0"))
    got = dict(
        line.split("=", 1) for line in out.read_text(encoding="utf-8").splitlines()
    )
    assert got["decision"] == "H0"
    assert (got["elo0"], got["elo1"]) == ("-5", "0")


def test_leaves_no_temporary_file(tmp_path):
    out = tmp_path / "adr0173.result"
    sprt_log.write_result(out, "adr0173", "H1", _fields(), ("0", "5"))
    assert sorted(p.name for p in tmp_path.iterdir()) == ["adr0173.result"]


def test_does_not_write_result_before_a_verdict(tmp_path):
    """判定に至っていない走行を完了として記録しない（ADR-0175）。"""
    log = tmp_path / "sprt-x.log"
    log.write_text(
        "selfplay: c vs b | tc 10+0.1 | SPRT elo[-5, 0] a=0.05\n"
        "pairs  4998 | +4970 =180 -4846 | [1,2,3,4,5] | "
        "Elo +0.8 [-5.9,+7.4] | LLR +1.41 [-2.94,2.94]\n",
        encoding="utf-8",
    )
    out = tmp_path / "x.result"
    _, verdict = sprt_log.report(log, "x", result=out)

    assert verdict == "判定前"
    assert not out.exists()


def test_writes_result_on_a_verdict(tmp_path):
    log = tmp_path / "sprt-y.log"
    log.write_text(
        "selfplay: c vs b | tc 10+0.1 | SPRT elo[0, 5] a=0.05\n"
        "H1採択（候補は有意に強い） | pairs 3837 games 7674 | "
        "+3819 =284 -3571 | Elo +11.2 [+3.7,+18.8] | LLR +2.92\n",
        encoding="utf-8",
    )
    out = tmp_path / "y.result"
    text, verdict = sprt_log.report(log, "y", result=out)

    assert verdict == "H1"
    assert "decision=H1" in out.read_text(encoding="utf-8")
    assert "SPRT: +11.2 [+3.7,+18.8] 7674games H1" in text


def test_report_raises_when_log_is_missing(tmp_path):
    with pytest.raises(sprt_log.Unreadable):
        sprt_log.report(tmp_path / "nope.log", "x")
