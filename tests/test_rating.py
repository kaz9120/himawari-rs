"""floodgateのレートの読み取りを検証する（ADR-0209）。通信はしない。"""

import datetime

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
