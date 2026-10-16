"""実戦棋譜の回収のうち、ネットワークに触らない部分を検証する。

対局者ページからCSAのURLを取り出す処理は、wdoorへ問い合わせずに
確かめられる。取得そのものはこのテストでは走らせない。
"""

import datetime

from hmwr.tools import floodgate as fetch

PAGE_URL = "https://wdoor.c.u-tokyo.ac.jp/shogi/x/2026/player/Himawari+6fd5a66.html"

# 実物の対局者ページから、リンクの種類が分かる最小限を抜き出した形
PLAYER_HTML = """\
<h2>Matches</h2>
<tr><td><a href="nnz4+c8c10d8.html">nnz4</a></td>
<td><a href="/shogi/x/2026/player/H/n/Himawari+6fd5a66+nnz4+c8c10d8.html">9</a></td></tr>
<h2>Games</h2>
<li class="gameitem">
<a href="/shogi/x/2026/08/01/wdoor+floodgate-300-10F+Himawari+jhbr2+20260801000000.html">08-01</a>
<span><a href="/shogi/x/2026/player/Himawari+6fd5a66.html">Himawari</a></span></li>
<li class="gameitem">
<a href="/shogi/x/2026/07/31/wdoor+floodgate-300-10F+nnz4+Himawari+20260731170007.html">07-31</a>
</li>
<footer><a href="/shogi">top</a></footer>
"""


def test_game_csa_urls_converts_html_links_to_csa():
    got = fetch.game_csa_urls(PLAYER_HTML, PAGE_URL)
    assert got == [
        (
            "2026",
            "wdoor+floodgate-300-10F+Himawari+jhbr2+20260801000000.csa",
            "https://wdoor.c.u-tokyo.ac.jp/shogi/x/2026/08/01/"
            "wdoor+floodgate-300-10F+Himawari+jhbr2+20260801000000.csa",
        ),
        (
            "2026",
            "wdoor+floodgate-300-10F+nnz4+Himawari+20260731170007.csa",
            "https://wdoor.c.u-tokyo.ac.jp/shogi/x/2026/07/31/"
            "wdoor+floodgate-300-10F+nnz4+Himawari+20260731170007.csa",
        ),
    ]


def test_game_csa_urls_skips_player_and_head_to_head_pages():
    """対局者ページと対戦成績ページは日付を通らないので拾わない。"""
    only_players = """\
<a href="nnz4+c8c10d8.html">nnz4</a>
<a href="/shogi/x/2026/player/H/n/Himawari+6fd5a66+nnz4+c8c10d8.html">9</a>
<a href="/shogi/x/2026/player/Himawari+6fd5a66.html">Himawari</a>
"""
    assert fetch.game_csa_urls(only_players, PAGE_URL) == []


def test_game_csa_urls_accepts_relative_links():
    """相対リンクでも同じCSAへ解決する。"""
    relative = '<a href="../08/01/wdoor+floodgate-300-10F+Himawari+jhbr2+20260801000000.html">g</a>'
    got = fetch.game_csa_urls(relative, PAGE_URL)
    assert len(got) == 1
    assert got[0][0] == "2026"
    assert got[0][2].endswith(
        "/x/2026/08/01/wdoor+floodgate-300-10F+Himawari+jhbr2+20260801000000.csa"
    )


def test_game_csa_urls_is_deduped_and_sorted():
    got = fetch.game_csa_urls(PLAYER_HTML + PLAYER_HTML, PAGE_URL)
    assert got == fetch.game_csa_urls(PLAYER_HTML, PAGE_URL)
    assert [name for _, name, _ in got] == sorted(name for _, name, _ in got)


def test_game_csa_urls_drops_query_and_fragment():
    html = '<a href="/shogi/x/2026/08/01/wdoor+A+B+20260801000000.html?v=1#top">g</a>'
    got = fetch.game_csa_urls(html, PAGE_URL)
    assert got[0][2].endswith("/wdoor+A+B+20260801000000.csa")


def test_game_csa_urls_reads_the_year_from_the_path():
    html = '<a href="/shogi/x/2025/12/31/wdoor+A+B+20251231000000.html">g</a>'
    assert fetch.game_csa_urls(html, PAGE_URL)[0][0] == "2025"


def test_parser_defaults_to_the_himawari_player_page(tmp_path):
    args = fetch.build_parser(tmp_path).parse_args([])
    assert args.player_url is None
    assert fetch.player_urls(2026) == [PAGE_URL]


def _serve(monkeypatch, pages):
    """Fetcher.getを差し替える。pagesにないURLは404（None）にする。"""
    monkeypatch.setattr(fetch.Fetcher, "get", lambda self, url: pages.get(url))


def test_a_missing_page_for_this_year_counts_as_no_games(tmp_path, monkeypatch, capsys):
    """年が明けた直後は今年の対局者ページがない。そこで失敗しない。"""
    _serve(monkeypatch, {PAGE_URL: b"<h2>Games</h2>"})
    monkeypatch.setattr(fetch, "today", lambda: datetime.date(2027, 1, 1))
    argv = ["--out", str(tmp_path / "out"), "--log", str(tmp_path / "log"), "--sleep", "0"]
    assert fetch.main(argv) == 0
    out = capsys.readouterr().out
    assert "/x/2026/player/" in out and "/x/2027/player/" in out


def test_a_missing_page_that_was_asked_for_is_an_error(tmp_path, monkeypatch):
    _serve(monkeypatch, {})
    argv = ["--player-url", PAGE_URL, "--out", str(tmp_path), "--log", str(tmp_path / "log")]
    assert fetch.main(argv + ["--sleep", "0"]) == 3


def test_parser_accepts_multiple_player_urls(tmp_path):
    args = fetch.build_parser(tmp_path).parse_args(
        ["--player-url", "https://a/1.html", "--player-url", "https://b/2.html"]
    )
    assert args.player_url == ["https://a/1.html", "https://b/2.html"]


def test_parser_puts_output_under_data_raw_floodgate(tmp_path):
    args = fetch.build_parser(tmp_path).parse_args([])
    assert args.out == str(tmp_path / "data" / "raw" / "floodgate")
    assert args.log == str(tmp_path / "data" / "logs" / "floodgate-fetch.log")
