"""floodgateのレートの読み取りを検証する（ADR-0209）。通信はしない。"""

import datetime
import os
from pathlib import Path

import pytest

from hmwr import cli, proc, rating
from hmwr.tools import floodgate

DAILY = "date,rate\r\n2026-09-16,3603\r\n2026-09-17,3610\r\n2026-09-18,3611\r\n"
TWO_WEEKS = "date,rate\n2026-09-17,3873\n2026-09-18,3866\n\n"
PLAYER = "https://wdoor.c.u-tokyo.ac.jp/shogi/x/2026/player/Himawari+6fd5a66.html"


def test_csv_urls_come_from_the_player_page():
    assert rating.csv_urls(PLAYER) == {
        "daily": "https://wdoor.c.u-tokyo.ac.jp/shogi/x/2026/prating/Himawari+6fd5a66.csv",
        "two_weeks": "https://wdoor.c.u-tokyo.ac.jp/shogi/x/2026/prating14/Himawari+6fd5a66.csv",
    }
    with pytest.raises(proc.Fail):
        rating.csv_urls("https://example.com/somewhere.html")


def test_parse_skips_the_header_and_broken_rows():
    assert rating.parse(DAILY) == {"2026-09-16": 3603, "2026-09-17": 3610, "2026-09-18": 3611}
    assert rating.parse("date,rate\n404 Not Found\n2026-09-18,abc\n") == {}


def test_series_are_joined_by_date():
    days = rating.join(rating.parse(DAILY), rating.parse(TWO_WEEKS))
    assert [(d.date, d.daily, d.two_weeks) for d in days] == [
        ("2026-09-16", 3603, None),
        ("2026-09-17", 3610, 3873),
        ("2026-09-18", 3611, 3866),
    ]


def test_markdown_shows_the_window_and_the_change_over_it():
    days = rating.join(rating.parse(DAILY), rating.parse(TWO_WEEKS))
    text = rating.markdown(days, 2)
    assert "2026-09-18 時点: 日毎 **3611**（2026-09-17から+1）、2週間 **3866**（-7）" in text
    assert "| 2026-09-16 |" not in text
    assert "| 2026-09-18 | 3611 | 3866 |" in text


def test_the_command_fetches_both_series(monkeypatch, capsys):
    served = {u: t for u, t in zip(rating.csv_urls(PLAYER).values(), (DAILY, TWO_WEEKS))}
    monkeypatch.setattr(rating, "fetch", lambda url: served[url])
    assert cli.main(["kifu", "rate", "--player-url", PLAYER, "--csv"]) == proc.OK
    assert capsys.readouterr().out.splitlines() == [
        "date,daily,two_weeks",
        "2026-09-16,3603,",
        "2026-09-17,3610,3873",
        "2026-09-18,3611,3866",
    ]


PLAYER_2027 = PLAYER.replace("/2026/", "/2027/")


def test_default_pages_run_from_the_first_year_to_this_year():
    assert floodgate.player_urls(2026) == [PLAYER]
    assert floodgate.player_urls(2027) == [PLAYER, PLAYER_2027]
    # 時計が狂って過去へ戻っても、最初の年は読む
    assert floodgate.player_urls(2025) == [PLAYER]


def test_years_are_joined_and_a_missing_latest_year_is_empty(monkeypatch):
    served = dict(zip(rating.csv_urls(PLAYER).values(), (DAILY, TWO_WEEKS)))
    daily_2027 = "date,rate\n2026-09-18,3620\n2027-01-01,3630\n"
    urls_2027 = list(rating.csv_urls(PLAYER_2027).values())
    monkeypatch.setattr(rating, "fetch", lambda url: served.get(url))

    # 年が明けた直後: 今年のCSVはまだ無い
    days = rating.load([PLAYER, PLAYER_2027], missing_ok_last=True)
    assert days[-1].date == "2026-09-18"
    with pytest.raises(proc.Fail):
        rating.load([PLAYER, PLAYER_2027])

    # 今年のCSVができたら連結する。重なる日付は後ろの年を採る
    served[urls_2027[0]] = daily_2027
    served[urls_2027[1]] = "date,rate\n"
    days = rating.load([PLAYER, PLAYER_2027], missing_ok_last=True)
    assert [(d.date, d.daily, d.two_weeks) for d in days][-2:] == [
        ("2026-09-18", 3620, 3866),
        ("2027-01-01", 3630, None),
    ]


def test_check_fresh_stops_when_the_latest_day_is_old():
    days = rating.join(rating.parse(DAILY), rating.parse(TWO_WEEKS))
    rating.check_fresh(days, datetime.date(2026, 9, 19), 2)
    with pytest.raises(proc.Fail) as e:
        rating.check_fresh(days, datetime.date(2026, 9, 20), 2)
    assert e.value.code == proc.JUDGE
    with pytest.raises(proc.Fail):
        rating.check_fresh([], datetime.date(2026, 9, 20), 2)


def test_the_command_prints_then_fails_on_stale_data(monkeypatch, capsys):
    served = dict(zip(rating.csv_urls(PLAYER).values(), (DAILY, TWO_WEEKS)))
    monkeypatch.setattr(rating, "fetch", lambda url: served.get(url))
    monkeypatch.setattr(floodgate, "today", lambda: datetime.date(2026, 9, 25))
    argv = ["kifu", "rate", "--player-url", PLAYER, "--csv", "--max-age", "2"]
    assert cli.main(argv) == proc.JUDGE
    assert "2026-09-18,3611,3866" in capsys.readouterr().out


# グラフの見本。月をまたぎ、2週間のレートに欠けた日を1日含める
GOLDEN = Path(__file__).parent / "golden" / "rate.svg"
SAMPLE = [
    rating.Day("2026-07-29", 3178, 3161),
    rating.Day("2026-07-30", 3190, None),
    rating.Day("2026-07-31", 3215, 3230),
    rating.Day("2026-08-15", 3480, 3560),
    rating.Day("2026-09-01", 3550, 3710),
    rating.Day("2026-10-16", 3671, 3970),
]


def test_svg_matches_the_golden_file():
    """同じ入力から同じバイト列が出ることを、見本のファイルで固定する。

    見た目を変えたら UPDATE_GOLDEN=1 で書き直し、差分を目で確かめてから入れる。
    """
    got = rating.svg(SAMPLE, datetime.date(2026, 10, 16), PLAYER)
    if os.environ.get("UPDATE_GOLDEN"):
        GOLDEN.write_text(got, encoding="utf-8")
    assert got == GOLDEN.read_text(encoding="utf-8")
    assert rating.svg(SAMPLE, datetime.date(2026, 10, 16), PLAYER) == got


def test_svg_carries_the_date_the_source_and_both_series():
    got = rating.svg(SAMPLE, datetime.date(2026, 10, 16), PLAYER)
    assert "2026-10-16 取得" in got
    assert f"出典: {PLAYER}" in got
    assert "prefers-color-scheme: dark" in got
    # 欠けた日で2週間の線が切れ、孤立した最初の1日は点になる。数は線と凡例の和
    assert got.count('class="line daily"') == 2
    assert got.count('class="line two_weeks"') == 2
    assert '<circle class="dot-two_weeks" cx="56" cy="263.3" r="2"/>' in got
    assert "3671" in got and "3970" in got


def test_svg_without_records_still_renders():
    got = rating.svg([], datetime.date(2027, 1, 1), PLAYER)
    assert "レートの記録がまだない" in got and got.endswith("</svg>\n")


def test_y_ticks_stay_within_six_lines():
    assert rating._y_ticks(3161, 3974) == (200, 3000, 4000)
    assert rating._y_ticks(3600, 3600) == (10, 3600, 3610)


def test_month_labels_span_years():
    starts = rating._month_starts(datetime.date(2026, 11, 20), datetime.date(2027, 2, 3))
    assert starts == [datetime.date(2026, 12, 1), datetime.date(2027, 1, 1), datetime.date(2027, 2, 1)]


def test_the_command_writes_the_svg(monkeypatch, tmp_path, capsys):
    served = dict(zip(rating.csv_urls(PLAYER).values(), (DAILY, TWO_WEEKS)))
    monkeypatch.setattr(rating, "fetch", lambda url: served.get(url))
    monkeypatch.setattr(floodgate, "today", lambda: datetime.date(2026, 9, 18))
    out = tmp_path / "rate.svg"
    argv = ["kifu", "rate", "--player-url", PLAYER, "--csv", "--svg", str(out), "--max-age", "2"]
    assert cli.main(argv) == proc.OK
    assert capsys.readouterr().out.startswith("date,daily,two_weeks\n")
    assert "2026-09-18 取得" in out.read_text(encoding="utf-8")
