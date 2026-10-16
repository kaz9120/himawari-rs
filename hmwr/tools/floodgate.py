#!/usr/bin/env python3
"""floodgateの棋譜をwdoorから回収する（ADR-0152）。

wdoorの対局者ページ（``x/<年>/player/<アカウント>.html``）を読み、そこに
載っている対局のCSAを ``data/raw/floodgate/<年>/`` へ落とす。

**対局者ページに載っている棋譜だけを正とする。** 日別アーカイブを走査して
ファイル名を部分一致で拾う方法も採れるが、名前の一致だけを頼りにすると
関係のない対局が混じる。対局者ページなら、その対局者の棋譜であることを
wdoor側が保証する（2026-08-09オーナー指示）。

回収は追記専用にする。取得済みのファイルは再取得も削除もしない。
入力集合が単調に増えるだけなら、ある時点のスナップショットに対する
分析と定跡追加を後から再現できる（ADR-0152の決定論）。

アカウント名（``Himawari+6fd5a66``）は変わらない。年はURLに含まれるので、
既定では2026年から今年（日本時間）までの対局者ページを順に読む。年が
明けた直後は今年のページがまだないので、今年の分の404だけは空として扱う。

終了コード: 0=成功、2=引数エラー、3=実行時エラー（ADR-0122）。
"""

import argparse
import datetime
import os
import pathlib
import re
import sys
import time

from .. import paths
import urllib.error
import urllib.parse
import urllib.request

PLAYER = "Himawari+6fd5a66"
PLAYER_URL_TEMPLATE = "https://wdoor.c.u-tokyo.ac.jp/shogi/x/{year}/player/" + PLAYER + ".html"
# 対局者ページが最初にできた年。ここから今年までを読む
FIRST_YEAR = 2026
# floodgateの日付は日本時間で切り替わる
JST = datetime.timezone(datetime.timedelta(hours=9))
USER_AGENT = "himawari-rs floodgate-fetch (+https://github.com/kaz9120/himawari-rs)"

HREF_RE = re.compile(r'href="([^"]+)"')
# 対局のリンクは日付のディレクトリを通る（/shogi/x/<年>/<月>/<日>/<名前>.html）。
# 同じページにある対局者ページや対戦成績ページは日付を通らないので外れる
GAME_PATH_RE = re.compile(r"/x/(\d{4})/\d{2}/\d{2}/([^/]+)\.html$")


def today():
    """日本時間の今日。年とレートの日付はこの暦で決まる。"""
    return datetime.datetime.now(JST).date()


def player_urls(year=None):
    """FIRST_YEARからyear（既定は今年）までの対局者ページを、古い順に返す。"""
    last = year if year is not None else today().year
    return [PLAYER_URL_TEMPLATE.format(year=y) for y in range(FIRST_YEAR, max(last, FIRST_YEAR) + 1)]


def error(message):
    """エラーメッセージを規約の書式でstderrへ出す。"""
    print(f"エラー: {message}", file=sys.stderr)


class ArgParser(argparse.ArgumentParser):
    """引数エラーを「エラー: ...」の書式・終了コード2に揃える。"""

    def error(self, message):
        error(message)
        sys.exit(2)


def build_parser(repo_root):
    parser = ArgParser(
        prog="floodgate-fetch.py",
        description="floodgateの棋譜をwdoorから回収する（ADR-0152）。",
        epilog="対局者ページに載っている棋譜だけを取る。"
        "取得済みのファイルはスキップする（追記専用）。",
    )
    parser.add_argument(
        "--player-url",
        action="append",
        metavar="URL",
        help="対局者ページ。複数指定できる"
        f"（既定 {FIRST_YEAR}年から今年まで: {PLAYER_URL_TEMPLATE}）",
    )
    parser.add_argument(
        "--out",
        default=str(repo_root / "data" / "raw" / "floodgate"),
        help="保存先ディレクトリ（既定 data/raw/floodgate）",
    )
    parser.add_argument(
        "--log",
        default=str(repo_root / "data" / "logs" / "floodgate-fetch.log"),
        help="詳細ログの追記先（既定 data/logs/floodgate-fetch.log）",
    )
    parser.add_argument(
        "--sleep", type=float, default=0.5, help="1リクエストごとの待ち秒数（既定0.5）"
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=30.0,
        help="1リクエストのタイムアウト秒（既定30）",
    )
    parser.add_argument(
        "--max-files",
        type=int,
        default=0,
        help="取得するCSAの上限枚数。0で無制限（既定0）",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="取得せず、落とす対象の一覧だけを出す",
    )
    return parser


def game_csa_urls(html, page_url):
    """対局者ページから、その対局者のCSAのURLを取り出す。

    リンクは相対でも絶対でも受ける。対局のページは ``.html`` にあり、CSAは
    同じ名前の ``.csa`` にある。(年, ファイル名, URL) を昇順で返す。
    処理順を固定して、同じページからは常に同じ順で取りにいく。
    """
    found = {}
    for href in HREF_RE.findall(html):
        parsed = urllib.parse.urlparse(urllib.parse.urljoin(page_url, href))
        m = GAME_PATH_RE.search(parsed.path)
        if not m:
            continue
        year, name = m.group(1), m.group(2)
        csa_path = parsed.path[: -len(".html")] + ".csa"
        url = urllib.parse.urlunparse(
            parsed._replace(path=csa_path, params="", query="", fragment="")
        )
        found[(year, f"{name}.csa")] = url
    return sorted((year, name, url) for (year, name), url in found.items())


class Fetcher:
    """HTTPの取得と待ちをまとめる。待ちは取得の直後に入れる。"""

    def __init__(self, timeout, sleep):
        self.timeout = timeout
        self.sleep = sleep
        self.requests = 0

    def get(self, url):
        """URLの中身をbytesで返す。404はNone。"""
        req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as res:
                return res.read()
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return None
            raise
        finally:
            self.requests += 1
            if self.sleep > 0:
                time.sleep(self.sleep)


class Log:
    """詳細ログを追記する。ログの置き場はスクリプトが決める（ADR-0149）。"""

    def __init__(self, path):
        self.path = path
        os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
        self.file = open(path, "a", encoding="utf-8")
        self.run_id = datetime.datetime.now().strftime("%Y-%m-%dT%H:%M:%S")

    def write(self, message):
        self.file.write(f"{self.run_id} {message}\n")
        self.file.flush()

    def close(self):
        self.file.close()


def fetch_player(page_url, args, fetcher, log, counts, missing_ok=False):
    """対局者ページ1枚を読み、未取得のCSAを落とす。

    countsを更新する。上限に達したらFalseを返す。missing_okなら、ページが
    ないこと（404）を対局0局として扱う。
    """
    body = fetcher.get(page_url)
    if body is None and missing_ok:
        log.write(f"対局者ページがまだない（対局0局として扱う）: {page_url}")
        return True
    if body is None:
        log.write(f"対局者ページがない: {page_url}")
        error(f"対局者ページがない: {page_url}")
        counts["errors"] += 1
        return True

    games = game_csa_urls(body.decode("utf-8", "replace"), page_url)
    counts["pages"] += 1
    counts["found"] += len(games)
    log.write(f"対局者ページ {len(games)}局: {page_url}")

    for year, name, url in games:
        dest = pathlib.Path(args.out) / year / name
        if dest.exists():
            counts["skipped"] += 1
            continue
        if args.max_files and counts["fetched"] >= args.max_files:
            log.write(f"上限{args.max_files}枚に達したので打ち切る")
            counts["truncated"] = True
            return False
        if args.dry_run:
            counts["would_fetch"] += 1
            print(f"取得予定: {dest}")
            continue
        data = fetcher.get(url)
        if data is None:
            log.write(f"取得できない（404）: {url}")
            counts["errors"] += 1
            continue
        dest.parent.mkdir(parents=True, exist_ok=True)
        # 書き途中のファイルを残さない。次回に取得済みと誤判定される
        tmp = dest.with_name(dest.name + ".part")
        tmp.write_bytes(data)
        tmp.replace(dest)
        counts["fetched"] += 1
        log.write(f"取得 {len(data)}バイト: {dest}")
    return True


def main(argv=None):
    repo_root = paths.REPO
    parser = build_parser(repo_root)
    args = parser.parse_args(argv)

    if args.sleep < 0:
        error(f"--sleep が負: {args.sleep}")
        return 2
    if args.max_files < 0:
        error(f"--max-files が負: {args.max_files}")
        return 2
    # 既定の並びでは、最後（今年）のページだけがまだ無くてよい
    pages = args.player_url or player_urls()
    missing_ok_page = None if args.player_url else pages[-1]

    counts = {
        "pages": 0,
        "found": 0,
        "fetched": 0,
        "skipped": 0,
        "would_fetch": 0,
        "errors": 0,
        "truncated": False,
    }
    fetcher = Fetcher(args.timeout, args.sleep)
    try:
        log = Log(args.log)
    except OSError as e:
        error(f"ログを開けない: {args.log}（{e}）")
        return 3

    log.write(f"開始 pages={len(pages)} out={args.out}")
    try:
        for page_url in pages:
            missing_ok = page_url == missing_ok_page
            if not fetch_player(page_url, args, fetcher, log, counts, missing_ok):
                break
    except (urllib.error.URLError, OSError, TimeoutError) as e:
        error(f"取得に失敗した（{e}）")
        log.write(f"失敗: {e}")
        log.close()
        return 3

    log.write(
        f"終了 リクエスト{fetcher.requests} 該当{counts['found']} "
        f"取得{counts['fetched']} 既取得{counts['skipped']} 失敗{counts['errors']}"
    )
    log.close()

    print("=== floodgate回収（ADR-0152）===")
    for url in pages:
        print(f"対局者ページ : {url}")
    print(f"保存先       : {args.out}")
    print(f"該当棋譜     : {counts['found']}")
    if args.dry_run:
        print(f"取得予定     : {counts['would_fetch']}（--dry-run のため取得しない）")
    else:
        print(f"新規取得     : {counts['fetched']}")
    print(f"既取得       : {counts['skipped']}")
    print(f"リクエスト   : {fetcher.requests}")
    print(f"ログ         : {args.log}")
    if counts["truncated"]:
        print(f"--max-files {args.max_files} で打ち切った。続きは再実行で取れる")
    if counts["errors"]:
        error(f"取得できなかった棋譜が {counts['errors']} 件ある。ログを見る: {args.log}")
        return 3
    return 0


if __name__ == "__main__":
    sys.exit(main())
