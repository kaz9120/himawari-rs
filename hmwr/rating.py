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


def csv_text(days: list[Day]) -> str:
    """全履歴をCSVにする。グラフの置き場にも同じ表を置く。"""
    lines = ["date,daily,two_weeks"]
    lines += [f"{d.date},{_cell(d.daily)},{_cell(d.two_weeks)}" for d in days]
    return "\n".join(lines) + "\n"


def _cell(value: int | None) -> str:
    return "" if value is None else str(value)


# --- 推移のグラフ（SVG） ---
#
# READMEが画像として読む（#750）。文字は閲覧者のフォントで描くので、日本語の
# フォントを持たないランナーでも化けない。色は prefers-color-scheme で
# 切り替え、背景も一緒に塗る。GitHubの表示テーマとOSの設定が食い違っても、
# 文字と背景の組は崩れない。
#
# 同じ入力からは同じバイト列を出す。座標は小数1桁に丸め、取得日は引数で受ける。

SVG_WIDTH = 760
SVG_HEIGHT = 360
# 描画域。右の余白は最新値のラベル、上は表題、下は日付と出典に使う
PLOT_LEFT, PLOT_RIGHT, PLOT_TOP, PLOT_BOTTOM = 56, 640, 72, 300
Y_STEPS = (10, 20, 25, 50, 100, 200, 250, 500, 1000)
FONT = (
    "-apple-system, BlinkMacSystemFont, 'Hiragino Sans', 'Noto Sans JP', "
    "'Yu Gothic', 'Segoe UI', sans-serif"
)
# 色はdatavizの既定の配色（カテゴリの1番と2番、面と文字の色）から取った
SVG_STYLE = """\
svg { --bg: #fcfcfb; --ink: #0b0b0b; --ink2: #52514e; --grid: #e4e3df; --s1: #2a78d6; --s2: #eb6834; }
@media (prefers-color-scheme: dark) {
  svg { --bg: #1a1a19; --ink: #f0efec; --ink2: #c3c2b7; --grid: #3a3a36; --s1: #3987e5; --s2: #d95926; }
}
.bg { fill: var(--bg); }
text { font-family: FONT; fill: var(--ink2); font-size: 12px; }
.title { fill: var(--ink); font-size: 16px; font-weight: 600; }
.value { fill: var(--ink); font-weight: 600; }
.note { font-size: 11px; }
.grid { stroke: var(--grid); stroke-width: 1; }
.axis { stroke: var(--ink2); stroke-width: 1; }
.line { fill: none; stroke-width: 2; stroke-linejoin: round; stroke-linecap: round; }
.daily { stroke: var(--s1); }
.two_weeks { stroke: var(--s2); }
.dot-daily { fill: var(--s1); stroke: var(--bg); stroke-width: 2; }
.dot-two_weeks { fill: var(--s2); stroke: var(--bg); stroke-width: 2; }
""".replace("FONT", FONT)

LABELS = {"daily": "日毎のレート", "two_weeks": "2週間のレート"}


def _f(x: float) -> str:
    """座標を小数1桁で書く。末尾の .0 は落とす。"""
    text = f"{x:.1f}"
    return text[:-2] if text.endswith(".0") else text


def _y_ticks(lo: int, hi: int) -> tuple[int, int, int]:
    """目盛りの刻み幅と、軸の下端・上端を決める。目盛りは6本以内にする。"""
    for step in Y_STEPS:
        bottom = lo // step * step
        top = max(-(-hi // step) * step, bottom + step)
        if (top - bottom) // step <= 5:
            return step, bottom, top
    step = Y_STEPS[-1]
    return step, lo // step * step, max(-(-hi // step) * step, lo // step * step + step)


def _month_starts(first: datetime.date, last: datetime.date) -> list[datetime.date]:
    """firstの翌月から、lastまでの各月の1日。"""
    y, m = first.year, first.month
    out = []
    while True:
        y, m = (y + 1, 1) if m == 12 else (y, m + 1)
        d = datetime.date(y, m, 1)
        if d > last:
            return out
        out.append(d)


def svg(days: list[Day], fetched: datetime.date, source: str) -> str:
    """日毎と2週間のレートを1枚の折れ線にする。fetchedは取得日、sourceは出典のURL。"""
    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{SVG_WIDTH}" height="{SVG_HEIGHT}" '
        f'viewBox="0 0 {SVG_WIDTH} {SVG_HEIGHT}" role="img" aria-label="floodgateのレートの推移">',
        f"<style>\n{SVG_STYLE}</style>",
        f'<rect class="bg" width="{SVG_WIDTH}" height="{SVG_HEIGHT}" rx="6"/>',
        f'<text class="title" x="{PLOT_LEFT}" y="30">floodgateのレートの推移</text>',
        f'<text class="note" x="{PLOT_LEFT}" y="50">{fetched.isoformat()} 取得。'
        "2週間のレートは、直近2週間の対局だけで計算した値</text>",
    ]
    footer = (
        f'<text class="note" x="{PLOT_LEFT}" y="{SVG_HEIGHT - 12}">'
        f"出典: {_escape(source)}</text>"
    )

    values = [v for d in days for v in (d.daily, d.two_weeks) if v is not None]
    if not values:
        out.append(f'<text x="{PLOT_LEFT}" y="{PLOT_TOP + 40}">レートの記録がまだない</text>')
        return "\n".join(out + [footer, "</svg>"]) + "\n"

    first = datetime.date.fromisoformat(days[0].date)
    last = datetime.date.fromisoformat(days[-1].date)
    span = max((last - first).days, 1)
    step, y_lo, y_hi = _y_ticks(min(values), max(values))

    def x_of(date: datetime.date) -> float:
        return PLOT_LEFT + (PLOT_RIGHT - PLOT_LEFT) * (date - first).days / span

    def y_of(value: int) -> float:
        return PLOT_BOTTOM - (PLOT_BOTTOM - PLOT_TOP) * (value - y_lo) / (y_hi - y_lo)

    # 横の目盛り線とレートの値
    for v in range(y_lo, y_hi + 1, step):
        y = _f(y_of(v))
        out.append(f'<line class="grid" x1="{PLOT_LEFT}" y1="{y}" x2="{PLOT_RIGHT}" y2="{y}"/>')
        out.append(f'<text x="{PLOT_LEFT - 8}" y="{y}" dy="4" text-anchor="end">{v}</text>')
    out.append(
        f'<line class="axis" x1="{PLOT_LEFT}" y1="{PLOT_BOTTOM}" '
        f'x2="{PLOT_RIGHT}" y2="{PLOT_BOTTOM}"/>'
    )

    # 月の区切り。12本を超えるなら間引く。最初の目盛りと1月には年を添える
    months = _month_starts(first, last)
    every = -(-len(months) // 12) or 1
    for i, d in enumerate(months[::every]):
        x = _f(x_of(d))
        label = f"{d.year}年{d.month}月" if i == 0 or d.month <= every else f"{d.month}月"
        out.append(
            f'<line class="axis" x1="{x}" y1="{PLOT_BOTTOM}" x2="{x}" y2="{PLOT_BOTTOM + 4}"/>'
        )
        out.append(f'<text x="{x}" y="{PLOT_BOTTOM + 18}" text-anchor="middle">{label}</text>')

    # 折れ線。値のない日で線を切る
    latest: dict[str, tuple[float, float, int]] = {}
    for key in ("daily", "two_weeks"):
        runs: list[list[str]] = [[]]
        for d in days:
            value = getattr(d, key)
            if value is None:
                if runs[-1]:
                    runs.append([])
                continue
            x, y = x_of(datetime.date.fromisoformat(d.date)), y_of(value)
            runs[-1].append(f"{_f(x)},{_f(y)}")
            latest[key] = (x, y, value)
        for run in runs:
            if len(run) == 1:
                cx, cy = run[0].split(",")
                out.append(f'<circle class="dot-{key}" cx="{cx}" cy="{cy}" r="2"/>')
            elif run:
                out.append(f'<polyline class="line {key}" points="{" ".join(run)}"/>')

    # 最新の値を線の右端に直接書く。2本のラベルが近ければ上下へ離す
    label_y = {key: y for key, (_, y, _) in latest.items()}
    if len(label_y) == 2 and abs(label_y["daily"] - label_y["two_weeks"]) < 32:
        mid = (label_y["daily"] + label_y["two_weeks"]) / 2
        upper, lower = sorted(label_y, key=lambda k: (label_y[k], k))
        label_y[upper], label_y[lower] = mid - 16, mid + 16
    for key, (x, y, value) in latest.items():
        out.append(f'<circle class="dot-{key}" cx="{_f(x)}" cy="{_f(y)}" r="4"/>')
        ly = _f(label_y[key])
        out.append(f'<text class="value" x="{PLOT_RIGHT + 12}" y="{ly}" dy="-1">{value}</text>')
        out.append(f'<text x="{PLOT_RIGHT + 12}" y="{ly}" dy="13">{LABELS[key]}</text>')

    # 凡例。表題の右に置く
    for i, key in enumerate(("daily", "two_weeks")):
        x = PLOT_RIGHT - 220 + i * 116
        out.append(f'<line class="line {key}" x1="{x}" y1="26" x2="{x + 18}" y2="26"/>')
        out.append(f'<text x="{x + 24}" y="30">{LABELS[key]}</text>')

    out += [footer, "</svg>"]
    return "\n".join(out) + "\n"


def _escape(text: str) -> str:
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
