#!/usr/bin/env python3
"""samplyのプロファイルからself時間の上位を出す（ADR-0099）。

関数単位とソース行単位の2つを出す。行番号はデバッグ情報が要る
（CARGO_PROFILE_RELEASE_DEBUG=1 でビルドする）。

使い方:
  scripts/profile-report.py <profile.json.gz> [バイナリ] [表示件数]
"""

import bisect
import gzip
import json
import re
import subprocess
import sys
from collections import defaultdict

# Mach-Oの既定のロードアドレス。atosへ渡すRVAの基準
MACHO_VM_BASE = 0x100000000


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
    """アドレス -> "関数名 (file.rs:123)" をatosで引く。"""
    lines = {}
    if not binary or not addrs:
        return lines
    addrs = sorted(addrs)
    for i in range(0, len(addrs), 2000):
        chunk = addrs[i : i + 2000]
        try:
            out = subprocess.run(
                ["atos", "-o", binary, "-arch", "arm64", "-l", hex(MACHO_VM_BASE)]
                + [hex(MACHO_VM_BASE + a) for a in chunk],
                capture_output=True,
                text=True,
                check=False,
            ).stdout.splitlines()
        except FileNotFoundError:
            return lines
        for a, line in zip(chunk, out):
            lines[a] = line
    return lines


def main():
    if len(sys.argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    prof_path = sys.argv[1]
    binary = sys.argv[2] if len(sys.argv) > 2 else None
    topn = int(sys.argv[3]) if len(sys.argv) > 3 else 20

    syms_path = prof_path.replace(".json.gz", ".json.syms.json")
    with gzip.open(prof_path) as f:
        profile = json.load(f)
    resolve = make_resolver(load_symbols(syms_path))
    counts, total = collect(profile, resolve)

    by_func = defaultdict(int)
    for (_, _, name), v in counts.items():
        by_func[name] += v

    print(f"総サンプル {total}（2000Hz、約{total / 2000:.1f}秒）")
    print()
    print(f"--- self時間の上位{topn}（関数） ---")
    print("| 箇所 | self時間 |")
    print("|---|---|")
    for name, v in sorted(by_func.items(), key=lambda x: -x[1])[:topn]:
        print(f"| `{name}` | {v * 100 / total:.2f}% |")

    # 自前バイナリのアドレスをソース行へ落とす
    own = [a for (lib, a, _) in counts if lib and a is not None and lib != "dyld"]
    lines = resolve_lines(binary, own)
    if not any(re.search(r"\([^()]+:\d+\)$", v) for v in lines.values()):
        print()
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


if __name__ == "__main__":
    sys.exit(main())
