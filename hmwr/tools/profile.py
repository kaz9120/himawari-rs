#!/usr/bin/env python3
"""samplyのプロファイルからself時間の上位を出す（ADR-0099）。

関数単位とソース行単位の2つを出す。行番号はデバッグ情報が要る
（CARGO_PROFILE_RELEASE_DEBUG=1 でビルドする）。ソース行の解決には
atos を使うため、macOS上でしか行番号は出ない。

`--categories` を付けると、探索時間を処理の種類ごとに分けた表を出す
（ADR-0230）。インライン展開された関数まで呼び出し連鎖を復元して数えるので、
self時間の表では呼び出し元へ溶けて見えない処理も拾える。表の後に、分類の
規則に当たらない関数のうち割合の大きいものを挙げる。関数が増えたときに、
その時間が別の種類へ黙って数えられるのを見つけるためである。

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
from pathlib import Path

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
    parser.add_argument(
        "--categories",
        action="store_true",
        help="探索時間を処理の種類ごとに分けた表を出す。バイナリが要る",
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


# --- 処理の種類ごとの内訳（ADR-0230） ---

# (表示名, 盤面操作か, 優先の型, 関数名の正規表現)
# 呼び出し連鎖を葉から根へ辿り、最初に当たった種類へサンプルを数える
# （優先の型 "inner"）。"outer" の種類は、連鎖のどこかに現れたら内側より
# 先に取る。1手詰めの中のis_legalや、do_moveの中の王手の更新を、呼び出し元の
# 処理として数えるためである。"fallback" は利きを引く下請けの関数で、
# 連鎖のどこにもほかの種類が無いときだけ取る。
CATEGORIES = [
    ("1手詰め", True, "outer", r"himawari_engine::mate::"),
    ("do_move・undo_move", True, "outer", r"Position>::(do_move|undo_move|do_null_move|undo_null_move)$"),
    # 維持する利きの表（ADR-0232の位置の表、ADR-0239・ADR-0240のバイト表）。
    # 内訳を分けるため、葉に近い関数から照合する
    ("利きの表: 複写", True, "inner", r"EffectCounts>::copy_from$"),
    ("利きの表: 飛び利きの伸縮", True, "inner", r"EffectCounts>::(extend|cut)$"),
    ("利きの表: 短い利き", True, "inner", r"(EffectCounts>::(add_piece|remove_piece)$|effect_count::short_attacks$)"),
    ("利きの表: 差分のその他", True, "inner", r"EffectCounts>::\w+$"),
    ("利きの表: 段の管理", True, "inner", r"SearchBoard>::(do_move|undo_move|do_null_move|undo_null_move|catch_up)$"),
    # 都度作る利きの地図（ADR-0239）。SEEと合法性判定の中から作られるので、
    # それらより先に照合する。1手詰めの中で作る分は1手詰めに入る
    ("利きの地図", True, "inner", r"(EffectMap>::(build|build_near|add|add_slider)$|Position>::threat_map$)"),
    # 維持する利きの表（ADR-0148・ADR-0232）。試作では探索のワーカーが
    # make_move・unmake_move で更新するので、do_move・undo_move の外側の
    # 優先には当たらない。Position の中から更新する形へ変えると、
    # do_move・undo_move へ数えられる
    ("利きの表の維持", True, "inner", r"EffectTable>::(update|rebuild)$"),
    ("SEE", True, "inner", r"Position>::(see_ge|least_valuable_attacker)(_with|_by)?$"),
    (
        "王手判定",
        True,
        "inner",
        r"Position>::(gives_check|gives_direct_check_slow|check_squares|compute_check_squares"
        r"|check_info|slider_blockers|blockers_for_king|pinners|update_checkers)$",
    ),
    (
        "合法性判定",
        True,
        "inner",
        r"Position>::(is_legal|pseudo_legal|is_pawn_drop_mate|to_move)(_with|_by)?$",
    ),
    ("指し手生成", True, "inner", r"himawari_core::movegen::"),
    ("利きの計算（上記の外から）", True, "fallback", r"(Position>::attackers_to$|himawari_core::attacks::)"),
    ("千日手・宣言勝ち", False, "inner", r"Position>::(scan_repetition|repetition_state\w*|can_declare_win)$"),
    (
        "NNUEの差分更新",
        False,
        "inner",
        r"(NnueState>::(push|pop|ensure_both|apply_range\w*|refresh_top|computed_ancestor)$"
        r"|nnue_acc::(apply_rows|diff_rows|hand_delta_of))",
    ),
    ("NNUEの推論", False, "inner", r"(himawari_engine::nnue_simd::|NnueState>::evaluate$)"),
    ("指し手の順序付け", False, "inner", r"himawari_engine::movepick::"),
    ("置換表", False, "inner", r"(Worker>::probe_tt$|himawari_engine::tt::)"),
]
OTHER = "探索のその他"
# このいずれかを連鎖に含むサンプルだけを探索時間として数える
SEARCH_FRAME = re.compile(r"himawari_engine::search::Worker>::")
# 規則に当たらない関数を挙げる対象。自前のクレートに限る
OWN_CODE = re.compile(r"himawari_(core|engine)::")
# 挙げる下限（探索時間に占める割合）と件数
UNKNOWN_MIN_RATIO = 0.01
UNKNOWN_TOP = 5
_COMPILED = [(name, board, kind, re.compile(pat)) for name, board, kind, pat in CATEGORIES]


def clean_name(name):
    """関数名からジェネリクスの実引数とハッシュを落とす。"""
    name = re.sub(r"::<[^<>]*>$", "", name)
    return re.sub(r"::h[0-9a-f]{16}$", "", name)


def classify_at(names):
    """名前の連鎖（葉が先頭、clean_name済み）を種類と決め手の添字へ落とす。

    添字は、その種類を決めたフレームの位置である。種類に当たらなければ
    (OTHER, None) を返す。外側優先の種類は、連鎖の中で最も外側の一致を
    決め手とする。その内側が、その種類の中で費やした時間に当たる。
    """
    for kind in ("outer", "inner", "fallback"):
        rules = [(name, pat) for name, _, k, pat in _COMPILED if k == kind]
        if kind == "outer":
            for name, pat in rules:
                hits = [i for i, n in enumerate(names) if pat.search(n)]
                if hits:
                    return name, max(hits)
        else:
            for i, n in enumerate(names):
                for name, pat in rules:
                    if pat.search(n):
                        return name, i
    return OTHER, None


def classify(chain):
    """論理フレームの連鎖（葉が先頭）を種類の名前へ落とす。"""
    return classify_at([clean_name(n) for n in chain])[0]


def unknown_names(names, index):
    """決め手のフレームより内側で、どの規則にも当たらない自前の関数を返す。

    分類器が知らない関数を見つけるための目印である。関数が増えたとき、
    その時間は別の種類へ黙って数えられる。ADR-0232の計測では `see_ge_with`
    がSEEではなく指し手の順序付けへ、`EffectTable::update` が探索のその他へ
    入っていた。種類が妥当かは連鎖だけでは決まらないので、判断の材料として
    挙げるだけにする。
    """
    if index is None:
        # 種類に当たらないサンプルは、探索のフレームより内側だけを見る。
        # 根の側にはUSIの入口やランタイムが並ぶ
        index = next((i for i, n in enumerate(names) if SEARCH_FRAME.search(n)), len(names))
    out = []
    for n in names[:index]:
        if not OWN_CODE.search(n) or SEARCH_FRAME.search(n):
            continue
        if any(pat.search(n) for _, _, _, pat in _COMPILED):
            continue
        if n not in out:
            out.append(n)
    return out


def demangle(names):
    """c++filtでRustの記号名を戻す。無ければそのまま返す。"""
    if not names or shutil.which("c++filt") is None:
        return list(names)
    out = subprocess.run(
        ["c++filt"], input="\n".join(names), capture_output=True, text=True, check=False
    ).stdout.splitlines()
    return out if len(out) == len(names) else list(names)


def resolve_inline(binary, addrs):
    """アドレス -> インライン展開を含む関数名の連鎖（内側が先頭）をatosで引く。"""
    reason = None
    if platform.system() != "Darwin":
        reason = f"atosはmacOS専用。{platform.system()}では実行できない"
    elif shutil.which("atos") is None:
        reason = "atosが見つからない（Xcodeコマンドラインツールが要る）"
    if reason:
        return {}, reason
    raw = {}
    arch = platform.machine()
    addrs = sorted(addrs)
    for i in range(0, len(addrs), 2000):
        chunk = addrs[i : i + 2000]
        out = subprocess.run(
            ["atos", "-i", "-o", binary, "-arch", arch, "-l", hex(MACHO_VM_BASE)]
            + [hex(MACHO_VM_BASE + a) for a in chunk],
            capture_output=True,
            text=True,
            check=False,
        ).stdout
        # 1アドレスぶんの連鎖は空行で区切られる
        blocks = [b for b in out.split("\n\n") if b.strip()]
        for a, block in zip(chunk, blocks):
            raw[a] = [re.sub(r" \(in [^)]*\).*$", "", ln) for ln in block.splitlines()]
    flat = sorted({n for chain in raw.values() for n in chain})
    table = dict(zip(flat, demangle(flat)))
    return {a: [table[n] for n in chain] for a, chain in raw.items()}, None


def category_report(data, binary):
    """全スレッドの探索中のサンプルを、処理の種類ごとに数えて表にする。"""
    if not binary:
        print("（種類ごとの内訳は出せない。デバッグ情報付きのバイナリの指定が要る）")
        return
    libs = data["libs"]
    own = {i for i, lib in enumerate(libs) if lib["debugName"] == Path(binary).name}
    if not own:
        # 記録した後でバイナリを改名した場合。システムの外にあるライブラリを本体とみなす
        own = {
            i
            for i, lib in enumerate(libs)
            if not lib["path"].startswith(("/usr/lib/", "/System/"))
        }
    # (スレッド番号, スタック番号) -> 物理フレームの列（葉が先頭）
    per_thread = []
    addrs = set()
    for thread in data["threads"]:
        stacks, frames = thread["stackTable"], thread["frameTable"]
        funcs, resources = thread["funcTable"], thread["resourceTable"]
        strings = thread["stringArray"]

        def frame_key(fi, leaf, funcs=funcs, frames=frames, resources=resources, strings=strings):
            func, addr = frames["func"][fi], frames["address"][fi]
            ri = funcs["resource"][func]
            if ri is not None and ri >= 0 and resources["lib"][ri] in own and addr is not None:
                # 葉でないフレームは戻り先を指すので、1引いて呼び出し命令へ寄せる
                a = addr if leaf else addr - 1
                addrs.add(a)
                return ("own", a)
            return ("name", strings[funcs["name"][func]])

        cache = {}

        def stack_frames(si, stacks=stacks, cache=cache, frame_key=frame_key):
            if si in cache:
                return cache[si]
            out, cur, leaf = [], si, True
            while cur is not None:
                out.append(frame_key(stacks["frame"][cur], leaf))
                leaf = False
                cur = stacks["prefix"][cur]
            cache[si] = out
            return out

        samples = thread["samples"]
        weights = samples.get("weight") or [1] * len(samples["stack"])
        for si, w in zip(samples["stack"], weights):
            if si is not None:
                per_thread.append((stack_frames(si), w or 1))

    chains, reason = resolve_inline(binary, addrs)
    if reason:
        print(f"（種類ごとの内訳は出せない。{reason}）")
        return

    helper = next(pat for _, _, k, pat in _COMPILED if k == "fallback")
    counts = defaultdict(int)
    total = 0
    in_helper = defaultdict(int)
    # (規則に当たらない関数, 数えられた種類) -> サンプル数
    unknown = defaultdict(int)
    memo = {}
    for frames, w in per_thread:
        key = id(frames)
        if key not in memo:
            chain = []
            for kind, v in frames:
                chain.extend(chains.get(v, [f"0x{v:x}"]) if kind == "own" else [v])
            if any(SEARCH_FRAME.search(n) for n in chain):
                names = [clean_name(n) for n in chain]
                cat, at = classify_at(names)
                memo[key] = (cat, any(helper.search(n) for n in names), unknown_names(names, at))
            else:
                memo[key] = None
        if memo[key] is None:
            continue
        cat, uses_helper, unknown_here = memo[key]
        counts[cat] += w
        total += w
        if uses_helper:
            in_helper[cat] += w
        for name in unknown_here:
            unknown[(name, cat)] += w
    if total == 0:
        print("（探索中のサンプルがない）")
        return

    print(f"--- 探索時間の内訳（全スレッド、探索中のサンプル {total}） ---")
    # 「うち利き」は、その種類の時間のうち利きを引く関数（attackers_to・
    # attacks）の中にいた分。利きを差分で持つと安くなる見込みの上限に当たる
    print("| 処理 | 盤面操作 | 割合 | うち利き |")
    print("|---|---|---|---|")
    board_total = 0
    board_helper = 0
    for name, board, _, _ in CATEGORIES + [(OTHER, False, "", "")]:
        v = counts.get(name, 0)
        h = in_helper.get(name, 0)
        if board:
            board_total += v
            board_helper += h
        print(
            f"| {name} | {'○' if board else ''} | {v * 100 / total:.1f}% | {h * 100 / total:.1f}% |"
        )
    print(
        f"| 盤面操作の合計 | ○ | {board_total * 100 / total:.1f}% "
        f"| {board_helper * 100 / total:.1f}% |"
    )
    print()
    unknown_report(unknown, total)


def unknown_report(unknown, total):
    """分類の規則に当たらない関数のうち、割合の大きいものを挙げる。"""
    hot = [(k, v) for k, v in unknown.items() if v >= total * UNKNOWN_MIN_RATIO]
    hot.sort(key=lambda x: -x[1])
    if not hot:
        print(f"（分類の規則に当たらない関数は、探索時間の{UNKNOWN_MIN_RATIO:.0%}以上にない）")
        return
    print(f"--- 分類の規則に当たらない関数（探索時間の{UNKNOWN_MIN_RATIO:.0%}以上） ---")
    print("| 関数 | 割合 | 数えられた種類 |")
    print("|---|---|---|")
    for (name, cat), v in hot[:UNKNOWN_TOP]:
        print(f"| `{name}` | {v * 100 / total:.1f}% | {cat} |")
    print("数えられた種類が妥当か確かめ、違えばCATEGORIES（hmwr/tools/profile.py）へ足す。")


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
        code = report({"data": data, "syms_path": syms_path}, args.binary, args.top)
        if args.categories:
            print()
            category_report(data, args.binary)
        return code
    except (OSError, json.JSONDecodeError, KeyError) as e:
        error(f"プロファイルの解析に失敗した: {e}")
        return 3


if __name__ == "__main__":
    sys.exit(main())
