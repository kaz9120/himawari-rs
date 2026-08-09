"""scripts/sprt-summary.py のパース処理を検証する。

実ログの断片を固定文字列で埋め込み、判定行・pairs行・数値抽出の
挙動を確かめる。
"""

from conftest import load_module

sprt_summary = load_module("sprt_summary", "sprt-summary.py")


# --- find_source_line ---


def test_find_source_line_h1_verdict():
    lines = [
        "pairs   525 | +602 =46 -402 | [73,22,236,20,174] | Elo +67.0 [+46.4,+88.0] | LLR +3.05 [-2.94,2.94]",
        "H1採択（候補は有意に強い） | pairs 525 games 1050 | +602 =46 -402 | Elo +67.0 [+46.4,+88.0] | LLR +3.05",
    ]
    src, verdict = sprt_summary.find_source_line(lines)
    assert verdict == "H1"
    assert src.startswith("H1採択")


def test_find_source_line_h0_verdict():
    lines = ["H0採択（候補は有意に弱い） | pairs 100 games 200 | +50 =10 -140 | Elo -80.0 [-120.0,-40.0] | LLR -3.00"]
    src, verdict = sprt_summary.find_source_line(lines)
    assert verdict == "H0"


def test_find_source_line_no_verdict_uses_last_pairs_line():
    # 実ログ（adr0107.log）の断片。判定行が無いので最終pairs行を使う
    lines = [
        "pairs   100 | +76 =17 -107 | [29,9,42,4,16] | Elo -54.3 [-102.9,-7.7] | LLR -0.51 [-2.94,2.94]",
        "pairs   346 | +313 =57 -322 | [72,21,160,30,63] | Elo -4.5 [-28.5,+19.4] | LLR -0.24 [-2.94,2.94]",
    ]
    src, verdict = sprt_summary.find_source_line(lines)
    assert verdict == "判定前"
    assert src.startswith("pairs   346")


def test_find_source_line_taking_saved_ignores_earlier_verdict_lines():
    # 判定行が複数あれば最後の行を使う（tail -1相当）
    lines = [
        "H0採択 | pairs 10 games 20 | +5 =0 -15 | Elo -50.0 [-60.0,-40.0] | LLR -3.0",
        "H1採択 | pairs 20 games 40 | +30 =0 -10 | Elo +50.0 [+40.0,+60.0] | LLR +3.0",
    ]
    src, verdict = sprt_summary.find_source_line(lines)
    assert verdict == "H1"


def test_find_source_line_none_found():
    src, verdict = sprt_summary.find_source_line(["何も一致しない行", ""])
    assert src is None
    assert verdict is None


# --- parse_fields ---


def test_parse_fields_verdict_line_with_games():
    src = "H1採択（候補は有意に強い） | pairs 525 games 1050 | +602 =46 -402 | Elo +67.0 [+46.4,+88.0] | LLR +3.05"
    fields = sprt_summary.parse_fields(src)
    assert fields == {
        "elo_num": "+67.0",
        "elo_ci": "[+46.4,+88.0]",
        "llr": "+3.05",
        "wdl": "+602 =46 -402",
        "games": 1050,
    }


def test_parse_fields_pairs_line_without_games_doubles_pairs():
    # 実ログ（adr0102.log）の断片。games フィールドが無いのでpairs*2で求める
    src = "pairs   117 | +77 =10 -147 | [42,7,56,3,9] | Elo -107.2 [-150.9,-66.6] | LLR -1.40 [-2.94,2.94]"
    fields = sprt_summary.parse_fields(src)
    assert fields["games"] == 234
    assert fields["elo_num"] == "-107.2"
    assert fields["elo_ci"] == "[-150.9,-66.6]"
    assert fields["llr"] == "-1.40"
    assert fields["wdl"] == "+77 =10 -147"


def test_parse_fields_missing_required_field_raises():
    import pytest

    with pytest.raises(ValueError):
        sprt_summary.parse_fields("Eloもllrも無い行")


# --- default_feature ---


def test_default_feature_strips_log_suffix():
    assert sprt_summary.default_feature("data/sprt/adr0100.log") == "adr0100"


def test_default_feature_keeps_name_without_log_suffix():
    assert sprt_summary.default_feature("data/sprt/adr0100") == "adr0100"


# --- build_report ---


def test_build_report_h1_format():
    fields = {
        "elo_num": "+67.0",
        "elo_ci": "[+46.4,+88.0]",
        "llr": "+3.05",
        "wdl": "+602 =46 -402",
        "games": 1050,
    }
    out = sprt_summary.build_report("adr0100", "H1", fields)
    assert out.splitlines()[0] == "=== adr0100（H1） ==="
    assert "SPRT: +67.0 [+46.4,+88.0] 1050games H1" in out
    assert "| adr0100 | **+67.0 [+46.4,+88.0]**（1050局、LLR +3.05でH1採択） |" in out
    assert "| 対局数 | 1050（525ペア） |" in out
    assert "| 判定 | **H1** |" in out


def test_build_report_uchikiri_format_differs_from_verdict_wording():
    fields = {
        "elo_num": "+3.7",
        "elo_ci": "[-14.0,+21.4]",
        "llr": "+0.38",
        "wdl": "+577 =87 -564",
        "games": 1228,
    }
    out = sprt_summary.build_report("adr0101", "打ち切り", fields)
    assert "（1228局、LLR +0.38で打ち切り） |" in out
    assert "採択" not in out.split("RESULTS.md")[1].split("PR本文")[0]


# --- 終了コード ---


def test_exit_by_verdict_mapping():
    assert sprt_summary.EXIT_BY_VERDICT == {"H1": 0, "H0": 1, "打ち切り": 2, "判定前": 2}
