"""floodgateのレートの推移を読む（ADR-0209）。

対局者ページのグラフは、2本のCSVから描かれている。日毎のレートと、直近
2週間の対局だけで計算したレートである。どちらも初日からの全履歴を持つので、
いつ取っても同じ系列が得られる。**取得のタイミングに結果が依存しない。**
ページの表にある「いまの値」を控える方式だと、取った瞬間の1点しか残らない。

  /shogi/x/<年>/player/<対局者>.html      対局者ページ
  /shogi/x/<年>/prating/<対局者>.csv      日毎のレート
  /shogi/x/<年>/prating14/<対局者>.csv    2週間のレート

CSVは年ごとに分かれる。既定では2026年から今年までを読み、日付で連結する。
年が明けた直後は今年のCSVがまだないので、今年の分の404だけは空として扱う。
"""

from __future__ import annotations

import datetime
import re
import urllib.error
import urllib.request
from dataclasses import dataclass

from . import proc
from .tools import floodgate

PLAYER_RE = re.compile(r"^(?P<base>.+/x/\d{4})/player/(?P<player>[^/]+)\.html$")
SERIES = {"daily": "prating", "two_weeks": "prating14"}


@dataclass(frozen=True)
class Day:
    date: str
    daily: int | None
    two_weeks: int | None


def csv_urls(player_url: str) -> dict[str, str]:
    """対局者ページのURLから、2本のCSVのURLを作る。"""
    m = PLAYER_RE.match(player_url)
    if m is None:
        raise proc.Fail(f"対局者ページのURLの形が違う: {player_url}", proc.USAGE)
    return {key: f"{m['base']}/{name}/{m['player']}.csv" for key, name in SERIES.items()}


def fetch(url: str) -> str | None:
    """CSVの中身を返す。404はNone。"""
    request = urllib.request.Request(url, headers={"User-Agent": floodgate.USER_AGENT})
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            return response.read().decode("utf-8", errors="replace")
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return None
        raise proc.Fail(f"取得に失敗した: {url}\n{e}") from e
    except (urllib.error.URLError, TimeoutError) as e:
        raise proc.Fail(f"取得に失敗した: {url}\n{e}") from e


def parse(text: str) -> dict[str, int]:
    """`date,rate` のCSVを読む。見出しと、数値でない行は飛ばす。"""
    rows: dict[str, int] = {}
    for line in text.replace("\r", "").splitlines():
        date, _, rate = line.partition(",")
        if re.fullmatch(r"\d{4}-\d{2}-\d{2}", date) and re.fullmatch(r"-?\d+", rate.strip()):
            rows[date] = int(rate)
    return rows


def join(daily: dict[str, int], two_weeks: dict[str, int]) -> list[Day]:
    dates = sorted(set(daily) | set(two_weeks))
    return [Day(d, daily.get(d), two_weeks.get(d)) for d in dates]


def load(player_urls: list[str], missing_ok_last: bool = False) -> list[Day]:
    """対局者ページごとに2本のCSVを読み、日付で連結する。

    同じ日付が2つの年にあれば、後ろのページの値を採る。missing_ok_lastなら、
    最後のページのCSVがないこと（404）を空として扱う。
    """
    daily: dict[str, int] = {}
    two_weeks: dict[str, int] = {}
    for i, page in enumerate(player_urls):
        missing_ok = missing_ok_last and i == len(player_urls) - 1
        for key, url in csv_urls(page).items():
            text = fetch(url)
            if text is None and not missing_ok:
                raise proc.Fail(f"CSVがない（404）: {url}")
            (daily if key == "daily" else two_weeks).update(parse(text or ""))
    return join(daily, two_weeks)


def check_fresh(days: list[Day], today: datetime.date, max_age: int) -> None:
    """最新の日付がmax_age日以上古ければ止める。

    年の変わり目にURLが追従しない、放流が止まった、といった異常は、
    古い表を出し続ける形で黙って進む。ここで判定結果として落とす。
    """
    if not days:
        raise proc.Fail("レートの記録が1日もない", proc.JUDGE)
    latest = datetime.date.fromisoformat(days[-1].date)
    age = (today - latest).days
    if age >= max_age:
        raise proc.Fail(
            f"最新の記録が{days[-1].date}で、{age}日古い（上限{max_age}日未満）。"
            "対局者ページのURLか放流を確かめる",
            proc.JUDGE,
        )


def _delta(now: int | None, before: int | None) -> str:
    if now is None or before is None:
        return ""
    return f"{now - before:+d}"


def markdown(days: list[Day], window: int) -> str:
    """直近window日の表と、期間の最初からの差を返す。"""
    if not days:
        return "レートの記録がまだない。"
    shown = days[-window:]
    first, last = shown[0], shown[-1]
    lines = [
        f"{last.date} 時点: 日毎 **{last.daily}**（{first.date}から{_delta(last.daily, first.daily)}）、"
        f"2週間 **{last.two_weeks}**（{_delta(last.two_weeks, first.two_weeks)}）",
        "",
        "| 日付 | 日毎 | 2週間 |",
        "|---|---|---|",
    ]
    lines += [f"| {d.date} | {d.daily or ''} | {d.two_weeks or ''} |" for d in shown]
    return "\n".join(lines)
