"""floodgateのレートの読み取りを検証する（ADR-0209）。通信はしない。"""

import pytest

from hmwr import cli, proc, rating

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
