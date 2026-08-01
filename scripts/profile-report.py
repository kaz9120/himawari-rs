#!/usr/bin/env python3
"""samplyのプロファイルからself時間の上位を出す（ADR-0099）。

関数単位とソース行単位の2つを出す。行番号はデバッグ情報が要る
（CARGO_PROFILE_RELEASE_DEBUG=1 でビルドする）。ソース行の解決には
atos を使うため、macOS上でしか行番号は出ない。

終了コード: 0=成功、2=引数エラー、3=実行時エラー（ADR-0122）。
"""

import argparse
import bisect
import gzip
import json
import platform
import re
import shutil
import subprocess
import sys
from collections import defaultdict

# Mach-Oの既定のロードアドレス。atosへ渡すRVAの基準
MACHO_VM_BASE = 0x100000000


def error(message):
    """エラーメッセージを規約の書式でstderrへ出す。"""
    print(f"エラー: {message}", file=sys.stderr)


class ArgParser(argparse.ArgumentParser):
    """引数エラーを「エラー: ...」の書式・終了コード2に揃える。"""

    def error(self, message):
        error(message)
        sys.exit(2)


def build_parser():
    parser = ArgParser(
        prog="profile-report.py",
        description="samplyのプロファイルからself時間の上位を出す（ADR-0099）。",
        epilog="行番号はデバッグ情報が要る（CARGO_PROFILE_RELEASE_DEBUG=1 でビルドする）。"
        "ソース行の解決はmacOS上のatosに限る。",
    )
    parser.add_argument("profile", help="samplyのプロファイル（*.json.gz）")
    parser.add_argument(
        "binary",
        nargs="?",
        default=None,
        help="デバッグ情報付きバイナリ。省略するとソース行は出さない",
    )
    parser.add_argument(
        "--top", type=int, default=20, help="上位何件を表示するか（既定20）"
    )
    return parser


def load_symbols(syms_path):
    """debug_name -> (rvaの昇順リスト, [(size, 関数名)]) を作る。"""
    with open(syms_path) as f:
        syms = json.load(f)
    strings = syms["string_table"]
    out = {}
    for entry in syms["data"]:
        table = sorted(entry["symbol_table"], key=lambda e: e["rva"])
        out[entry["debug_name"]] = (
            [e["rva"] for e in table],
            [(e["size"], strings[e["symbol"]]) for e in table],
        )
    return out


def make_resolver(lib_syms):
    def resolve(libname, addr):
        entry = lib_syms.get(libname)
        if entry is None:
            return libname
        starts, info = entry
        i = bisect.bisect_right(starts, addr) - 1
        if i < 0 or addr >= starts[i] + info[i][0]:
            return f"{libname}+0x{addr:x}"
        return info[i][1]

    return resolve


def hottest_thread(profile):
    """サンプル数が最も多いスレッド（＝探索スレッド）を返す。"""
    return max(profile["threads"], key=lambda t: len(t["samples"]["stack"]))


def sample_hz(profile):
    """プロファイルのサンプリング周波数をmeta.interval（ミリ秒）から求める。"""
    interval_ms = profile["meta"]["interval"]
    return 1000.0 / interval_ms


def collect(profile, resolve):
    """(lib, addr, 関数名) ごとのサンプル数と総数を返す。"""
    libs = profile["libs"]
    thread = hottest_thread(profile)
    samples, stacks = thread["samples"], thread["stackTable"]
    frames, funcs = thread["frameTable"], thread["funcTable"]
    resources, strings = thread["resourceTable"], thread["stringArray"]
    cache = {}

    def frame_info(fi):
        if fi in cache:
            return cache[fi]
        func, addr = frames["func"][fi], frames["address"][fi]
        ri = funcs["resource"][func]
        libname = None
        if ri is not None and ri >= 0:
            li = resources["lib"][ri]
            if li is not None:
                libname = libs[li]["debugName"]
        if libname and addr is not None and addr >= 0:
            cache[fi] = (libname, addr, resolve(libname, addr))
        else:
            cache[fi] = (None, None, strings[funcs["name"][func]])
        return cache[fi]

    counts = defaultdict(int)
    total = 0
    weights = samples.get("weight") or [1] * len(samples["stack"])
    for si, w in zip(samples["stack"], weights):
        if si is None:
            continue
        w = w or 1
        total += w
        counts[frame_info(stacks["frame"][si])] += w
    return counts, total


def resolve_lines(binary, addrs):
    """アドレス -> "関数名 (file.rs:123)" をatosで引く。

    戻り値は (アドレス -> 行文字列の辞書, 諦めた理由)。
    解決を試みて空だった場合と最初から試みなかった場合を区別するため、
    諦めた理由はNoneでない文字列で返す。
    """
    if not binary or not addrs:
        return {}, None
    system = platform.system()
    if system != "Darwin":
        return {}, f"atosはmacOS専用。{system}では実行できない"
    if shutil.which("atos") is None:
        return {}, "atosが見つからない（Xcodeコマンドラインツールが要る）"

    lines = {}
    arch = platform.machine()
    addrs = sorted(addrs)
    for i in range(0, len(addrs), 2000):
        chunk = addrs[i : i + 2000]
        out = subprocess.run(
            ["atos", "-o", binary, "-arch", arch, "-l", hex(MACHO_VM_BASE)]
            + [hex(MACHO_VM_BASE + a) for a in chunk],
            capture_output=True,
            text=True,
            check=False,
        ).stdout.splitlines()
        for a, line in zip(chunk, out):
            lines[a] = line
    return lines, None


def report(profile, binary, topn):
    """レポート本文を組み立てて標準出力へ書く。成功時は0を返す。"""
    resolve = make_resolver(load_symbols(profile["syms_path"]))
    counts, total = collect(profile["data"], resolve)
    hz = sample_hz(profile["data"])

    by_func = defaultdict(int)
    for (_, _, name), v in counts.items():
        by_func[name] += v

    print(f"総サンプル {total}（{hz:g}Hz、約{total / hz:.1f}秒）")
    print()
    print(f"--- self時間の上位{topn}（関数） ---")
    print("| 箇所 | self時間 |")
    print("|---|---|")
    for name, v in sorted(by_func.items(), key=lambda x: -x[1])[:topn]:
        print(f"| `{name}` | {v * 100 / total:.2f}% |")

    # 自前バイナリのアドレスをソース行へ落とす
    own = [a for (lib, a, _) in counts if lib and a is not None and lib != "dyld"]
    lines, skip_reason = resolve_lines(binary, own)
    has_lines = any(re.search(r"\([^()]+:\d+\)$", v) for v in lines.values())
    if not has_lines:
        print()
        if skip_reason:
            print(f"（行番号なし。{skip_reason}）")
        elif binary is None:
            print("（行番号なし。バイナリを指定すると出る）")
        else:
            print("（行番号なし。CARGO_PROFILE_RELEASE_DEBUG=1 でビルドすると出る）")
        return 0

    by_line = defaultdict(int)
    for (lib, addr, name), v in counts.items():
        if addr is None or addr not in lines:
            by_line[(name, "")] += v
            continue
        m = re.search(r"\(([^()]+:\d+)\)$", lines[addr])
        by_line[(name, m.group(1) if m else "")] += v

    print()
    print(f"--- self時間の上位{topn}（ソース行） ---")
    for (name, loc), v in sorted(by_line.items(), key=lambda x: -x[1])[:topn]:
        print(f"{v * 100 / total:6.2f}%  {loc:24s} {name}")
    return 0


def main(argv=None):
    parser = build_parser()
    args = parser.parse_args(argv)

    syms_path = args.profile.replace(".json.gz", ".json.syms.json")
    try:
        with gzip.open(args.profile) as f:
            data = json.load(f)
    except (OSError, gzip.BadGzipFile, json.JSONDecodeError) as e:
        error(f"プロファイルを読めない: {args.profile}（{e}）")
        return 3

    try:
        return report({"data": data, "syms_path": syms_path}, args.binary, args.top)
    except (OSError, json.JSONDecodeError, KeyError) as e:
        error(f"プロファイルの解析に失敗した: {e}")
        return 3


if __name__ == "__main__":
    sys.exit(main())
