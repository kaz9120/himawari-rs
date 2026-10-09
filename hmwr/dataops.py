"""`hmwr data` の操作を、`psv` のサブコマンドへの対応表から作る（ADR-0208）。

**1操作は表の1行である。** 名前からのパス解決・既定値・ログ・完了マーカーは
ここの共通の実装が足す。部品を足すコストを1行にすることが、`psv` を
直接叩かせないための本体になる。

完了マーカーは `<出力>.done` に置く。出力は `.part` へ書いてから改名するので、
出力があれば最後まで書けている。完了マーカーには走らせたコマンドを控え、同じ名前で
条件の違う再実行を止める。
"""

from __future__ import annotations

import argparse
import json
import struct
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

from . import config, heartbeat, paths, proc
from .tools import focus_labels

# 静止化の並列数。**並列の出力はjobs固定で決定論になる**（逐次とは一致しない）
# ので、既定を1か所で持つ。ADR-0200〜0206のチェーン11回はすべて8だった
QUIET_JOBS = 8
SHUFFLE_SEED = 1

# 探索で付け直すときの既定（ADR-0205）。置換表は --hash を --jobs で割って
# ワーカーへ配るので、どちらを変えても出力が変わる。既定をここで持つ
RESCORE_DEPTH = 9
RESCORE_MAX_NODES = 1_000_000
RESCORE_JOBS = 8
RESCORE_HASH = 256

# 既定値の印。評価関数は実行時に config から引く
EVAL = object()

# DL系モデルによる付け直し（ADR-0215）。モデルは配布元のライセンスに従って
# 手元に置く。パスワードつきの配布なので、取得の手順はIssue #556にある
DL_MODEL = "data/models/dlshogi/model-dr2_exhi.onnx"
DL_SCALE = 600.0


@dataclass(frozen=True)
class Opt:
    """psvへそのまま渡すオプション。綴りはhmwrとpsvで同じにする。"""

    flag: str
    help: str
    metavar: str = "N"
    kind: type = int
    default: object = None  # Noneなら、指定がない限りpsvへ渡さない
    required: bool = False


@dataclass(frozen=True)
class Op:
    name: str  # hmwr data の操作名。ログの領域名にもなる
    psv: str  # psv のサブコマンド
    help: str
    description: str
    opts: tuple[Opt, ...] = ()
    suffix: str = ".psv"  # 出力の拡張子
    min_inputs: int = 1
    max_inputs: int | None = 1
    raw: bool = False  # --raw <データセット> で生データを入力にできる
    split_jobs: bool = False  # 入力を区間で割って並列に走らせ、結合する


EVAL_OPT = Opt("--eval-file", "評価関数（既定は EVAL_FILE）", "パス", str, EVAL)
LIMIT_OPT = Opt("--limit", "先頭のこの件数だけ処理する")
HASH_OPT = Opt("--hash", "置換表の大きさ", "MB")

OPS: tuple[Op, ...] = (
    Op(
        "shuffle",
        "shuffle",
        "全体をシャッフルする",
        "2パスのバケット法で動き、入力の大きさに制限はない。"
        "一時ファイルは出力と同じ場所に作る。--limit を渡すと、入力の先頭"
        "この件数だけを読む。生データの先頭だけを元にするときに使う。",
        opts=(Opt("--seed", "乱数種", default=SHUFFLE_SEED), LIMIT_OPT),
        raw=True,
    ),
    Op(
        "split",
        "head",
        "先頭から区間を切り出す",
        "学習用と検証用の切り出しに使う。検証用を先頭から取り、学習用は "
        "--skip で検証用の件数を飛ばして取る。",
        opts=(
            Opt("--count", "切り出す件数", required=True),
            Opt("--skip", "先頭から飛ばす件数"),
        ),
    ),
    Op(
        "mix",
        "shuffle",
        "複数の教師を混ぜてシャッフルする",
        "混合の比率は入力の件数で決まる。比率を変えるなら、先に split で"
        "件数を揃える。",
        opts=(
            Opt("--seed", "乱数種", default=SHUFFLE_SEED),
            Opt("--consume", "読み終えた入力を消し、空きのピークを出力1本ぶんに抑える。入力は戻せない", kind=bool),
        ),
        min_inputs=2,
        max_inputs=None,
    ),
    Op(
        "quiet",
        "quiet",
        "教師局面を静止局面へ置き換える",
        "評価関数が探索中に見るのは静止局面だが、公開データは"
        "取り合いの途中の局面へ収束後の評価値を付けて配られている。"
        "そのずれを消す。並列8で6,000万局面に2〜12分かかる。"
        "**学習データを静止化したら、検証集合も同じ設定で静止化する。**",
        opts=(
            Opt("--max-plies", "進める手数の上限", default=1),
            EVAL_OPT,
            Opt("--jobs", "並列数。変えると出力が変わる", default=QUIET_JOBS),
            LIMIT_OPT,
            HASH_OPT,
        ),
    ),
    Op(
        "rank",
        "rank",
        "兄弟局面の葉の群を作る",
        "ランキング損失の教師になる。psv rank は単スレッドなので、"
        "--jobs で入力を区間に割って並列に走らせ、順に結合する。"
        "区間はレコードの絶対位置で切る。",
        opts=(Opt("--skip", "先頭から飛ばす件数"), LIMIT_OPT, EVAL_OPT, HASH_OPT),
        suffix=".rankpsv",
        split_jobs=True,
    ),
    Op(
        "rescore",
        "relabel",
        "現行エンジンの探索でscoreと教師手を付け直す",
        "1局面ずつ深さを指定して読み直し、info行の最終値からscoreと最善手を取る。"
        "mateは±30000へ丸め、勝敗と手数は元のまま残す。"
        "8ワーカーで1,346局面/秒だったので、1億局面に20時間強かかる。"
        "DL系モデルの推論で付け直すのは `hmwr data relabel` である。",
        opts=(
            Opt("--depth", "読み直す深さ", default=RESCORE_DEPTH),
            Opt("--max-nodes", "1局面あたりのノード上限", default=RESCORE_MAX_NODES),
            Opt("--jobs", "並列数。変えると出力が変わる", default=RESCORE_JOBS),
            Opt("--hash", "置換表の大きさ。jobsで割って配る", "MB", default=RESCORE_HASH),
            EVAL_OPT,
            LIMIT_OPT,
        ),
    ),
    Op(
        "thin",
        "thin",
        "決着圏の局面を確率で間引く",
        "|score| が --threshold 以上の局面を、確率 --keep で残す。"
        "それ以外の局面はすべて残す。dlshogiのラベル（S=430）で勝率10%と90%の"
        "境は |score|=945 である。--count で書く件数を打ち切ると、入力の先頭"
        "だけを読む。seedを固定すれば出力は決まる。",
        opts=(
            Opt("--threshold", "決着圏とみなす |score| の下限", default=1318),
            Opt("--keep", "決着圏の局面を残す確率", "P", float, 0.5),
            Opt("--seed", "乱数の種", default=1),
            Opt("--count", "書く件数の上限"),
        ),
    ),
    Op(
        "dedup",
        "dedup",
        "同じ盤面の2回目以降を捨てる",
        "盤面（盤・手番・持ち駒）が初めて現れた局面だけを、入力の順に残す。"
        "手数と教師信号は比べない。--count で書く件数を打ち切ると、"
        "入力の先頭だけを読む。--limit は読む件数の上限である。出力1億件で約1.5GBのメモリを使う。",
        opts=(Opt("--count", "書く件数の上限"), LIMIT_OPT),
    ),
    Op(
        "oversample",
        "oversample",
        "該当する型の局面を複製して重くする",
        "教師の最善手から型を判定し、該当する局面を末尾へ（--times−1）回"
        "書き足す。判定に教師手が要るので、静止化の前に通す。出力の並びは"
        "元の全件のあとに複製が続くので、学習の前に shuffle を掛ける。",
        opts=(
            Opt("--kind", "重くする型", "型", str, "defense"),
            Opt("--times", "該当局面を何倍にするか", default=3),
            LIMIT_OPT,
        ),
    ),
)


def psv_bin() -> Path:
    return paths.release_bin("psv")


def dest(flag: str) -> str:
    return flag.lstrip("-").replace("-", "_")


# --- 登録 --------------------------------------------------------------


def add_parsers(ss: argparse._SubParsersAction) -> None:
    for op in OPS:
        t = ss.add_parser(op.name, help=op.help, description=op.description)
        t.add_argument("name", metavar="出力名", help=f"data/train/<出力名>{op.suffix} へ書く")
        t.add_argument(
            "--in",
            dest="inputs",
            action="append",
            default=[],
            metavar="入力名",
            help="data/train/<入力名>.psv を読む"
            + ("" if op.max_inputs == 1 else "。複数回渡せる"),
        )
        if op.raw:
            t.add_argument(
                "--raw",
                metavar="データセット",
                help="data/raw/<データセット>/ の生データ（*.bin）を入力にする",
            )
        for o in op.opts:
            default_note = ""
            if o.default is not None and o.default is not EVAL:
                default_note = f"（既定 {o.default}）"
            if o.kind is bool:
                t.add_argument(o.flag, action="store_true", help=o.help)
                continue
            t.add_argument(
                o.flag,
                type=o.kind,
                metavar=o.metavar,
                required=o.required,
                help=o.help + default_note,
            )
        if op.split_jobs:
            t.add_argument(
                "--jobs", type=int, default=1, metavar="N", help="並列数。--limit が要る（既定 1）"
            )
        t.add_argument(
            "--force", action="store_true", help="出力が既にあっても作り直す"
        )
        t.set_defaults(func=lambda args, op=op: run(op, args))

    t = ss.add_parser("stats", help="局面数・評価値の分布・勝敗を表示する")
    t.add_argument("name", metavar="名前", help="data/train/<名前>.psv")
    t.add_argument("--limit", type=int, metavar="N", help="先頭のこの件数だけ読む")
    t.set_defaults(func=stats)

    t = ss.add_parser(
        "rm",
        help="hmwr data が作った中間ファイルを消す",
        description="完了マーカーのある出力だけを消す。完了マーカーに作り方が残っているので、"
        "消しても同じコマンドで作り直せる。由来の記録がないファイルには触らない。",
    )
    t.add_argument(
        "names", nargs="+", metavar="名前", help="data/train/<名前>の.psv・.rankpsv・.focus"
    )
    t.set_defaults(func=remove)

    t = ss.add_parser(
        "openings",
        help="教師データから開始局面集を作る",
        description="手数の条件を満たす局面を先頭から拾い、openings/<出力名>.txt へ"
        "1行1局面のSFENで書く。入力はシャッフル済みのものを使う。",
    )
    t.add_argument("name", metavar="出力名", help="openings/<出力名>.txt へ書く")
    t.add_argument("--in", dest="inputs", action="append", default=[], metavar="入力名")
    t.add_argument("--count", type=int, required=True, metavar="N", help="拾う局面数")
    t.add_argument("--min-ply", type=int, default=0, metavar="N", help="この手数以上の局面だけ拾う")
    t.add_argument("--skip", type=int, default=0, metavar="N", help="先頭から飛ばす件数")
    t.add_argument("--force", action="store_true", help="出力が既にあっても作り直す")
    t.set_defaults(func=openings)

    t = ss.add_parser(
        "floodgate",
        help="floodgateの全対局の棋譜から局面を取り出す",
        description="wdoorの年別アーカイブ（GitHubのミラー）を data/raw/floodgate_archive/ へ取得し、"
        "年ごとに展開して、各対局の指す前の局面を1手ずつ書く。評価値は0で、あとで付け直す。"
        "勝敗は手番側から見た値。展開した棋譜は年ごとに消す。",
    )
    t.add_argument("name", metavar="出力名", help="data/train/<出力名>.psv へ書く")
    t.add_argument("--years", required=True, metavar="A-B", help="対象の年。例 2011-2026 か 2026")
    t.add_argument("--tag", default="2026-04-10", metavar="タグ", help="ミラーのリリースのタグ（既定 2026-04-10）")
    t.add_argument("--min-plies", type=int, default=30, metavar="N", help="これより短い対局は飛ばす（既定 30）")
    t.add_argument("--limit", type=int, metavar="N", help="年ごとに先頭N局だけ読む（試し用）")
    t.add_argument("--jobs", type=int, default=8, metavar="N", help="並列数。変えても出力は変わらない")
    t.add_argument("--force", action="store_true", help="出力が既にあっても作り直す")
    t.set_defaults(func=floodgate_op)

    t = ss.add_parser(
        "merge",
        help="シャッフル済みの教師を、ランダムに織り交ぜて1本にする",
        description="入力はどれもシャッフル済みであること。残り件数に比例した確率で織り交ぜるので、"
        "全体シャッフルをやり直さずに一様な並びになる。--consume を付けると、入力を末尾から"
        "読みながら切り詰めるので、空きは出力1本ぶんで済む。大きな母集団へ新しい局面を足すときに使う。",
    )
    t.add_argument("name", metavar="出力名", help="data/train/<出力名>.psv へ書く")
    t.add_argument("--in", dest="inputs", action="append", default=[], metavar="入力名",
                   help="data/train/<入力名>.psv を読む。複数回渡せる")
    t.add_argument("--seed", type=int, default=SHUFFLE_SEED, metavar="N", help=f"乱数種（既定 {SHUFFLE_SEED}）")
    t.add_argument("--consume", action="store_true", help="読んだ分だけ入力を切り詰め、最後に消す。入力は戻せない")
    t.add_argument("--force", action="store_true", help="出力が既にあっても作り直す")
    t.set_defaults(func=merge_op)

    t = ss.add_parser(
        "ledger",
        help="点数を付けた局面の台帳を扱う",
        description="dlshogiで点数を付けた局面の盤面ハッシュを data/ledger/ に持つ。"
        "付ける前に filter で台帳と突き合わせ、初めて見る局面だけを残す。付けたら add で足す。"
        "検証集合の局面も台帳に入れ、学習へ混ざらないようにする。",
    )
    lss = t.add_subparsers(dest="ledger_op", metavar="<操作>")
    lt = lss.add_parser("add", help="局面のハッシュを台帳へ足す")
    lt.add_argument("--in", dest="inputs", action="append", default=[], metavar="入力名",
                    help="data/train/<入力名>.psv。複数回渡せる")
    lt.set_defaults(func=ledger_add)
    lt = lss.add_parser("filter", help="台帳に無く、入力の中でも初出の局面だけを残す")
    lt.add_argument("name", metavar="出力名", help="data/train/<出力名>.psv へ書く")
    lt.add_argument("--in", dest="inputs", action="append", default=[], metavar="入力名")
    lt.add_argument("--force", action="store_true", help="出力が既にあっても作り直す")
    lt.set_defaults(func=ledger_filter)
    lt = lss.add_parser("show", help="台帳の局面数と、足した履歴を出す")
    lt.set_defaults(func=ledger_show)

    t = ss.add_parser(
        "pick",
        help="玉の段と勝率で局面を絞り込む",
        description="手番側の玉が --king-max 段目以内（1=敵陣の奥）で、教師の勝率 "
        "σ(score / --scale) が [--p-min, --p-max] に入る局面だけを残す。区画で選ぶ前に、"
        "足りない区画の候補を別のデータセットから取り出すときに使う。",
    )
    t.add_argument("name", metavar="出力名", help="data/train/<出力名>.psv へ書く")
    t.add_argument("--in", dest="inputs", action="append", default=[], metavar="入力名")
    t.add_argument("--king-max", type=int, default=9, metavar="N", help="手番側の玉の段の上限（既定 9）")
    t.add_argument("--p-min", type=float, default=0.0, metavar="P", help="勝率の下限（既定 0）")
    t.add_argument("--p-max", type=float, default=1.0, metavar="P", help="勝率の上限（既定 1）")
    t.add_argument("--scale", type=float, default=600.0, metavar="S", help="評価値→勝率の尺度（既定 600）")
    t.add_argument("--jobs", type=int, default=8, metavar="N", help="並列数。変えても出力は変わらない")
    t.add_argument("--force", action="store_true", help="出力が既にあっても作り直す")
    t.set_defaults(func=pick)

    t = ss.add_parser(
        "select",
        help="区画の配分とαで局面を選ぶ",
        description="局面を区画（手番側の玉の帯×形勢の帯）に分け、--king と --eval の配分で"
        "目標の局面数を決める。在庫の足りない区画は全部を取り、不足を同じ玉の帯の優勢へ回す。"
        "区画の中では、ありふれ度のα乗に反比例する確率で選ぶ。α=0 は自然な分布のまま。"
        "出力は入力の順に並ぶので、学習の前に shuffle を掛ける。",
    )
    t.add_argument("name", metavar="出力名", help="data/train/<出力名>.psv へ書く")
    t.add_argument("--in", dest="inputs", action="append", default=[], metavar="入力名",
                   help="data/train/<入力名>.psv を読む。複数回渡せる")
    t.add_argument("--king", required=True, metavar="A,B,C", help="玉の帯（敵陣,4〜6段,自陣）の配分")
    t.add_argument("--eval", dest="evals", required=True, metavar="A,B,C", help="形勢の帯（互角,優勢,勝勢）の配分")
    t.add_argument("--count", type=int, required=True, metavar="N", help="選ぶ局面数の目標")
    t.add_argument("--alpha", type=float, default=0.0, metavar="X", help="区画の中の選び方（既定 0）")
    t.add_argument("--scale", type=float, default=430.0, metavar="S", help="評価値→勝率の尺度（既定 430）")
    t.add_argument("--sample", type=int, default=5_000_000, metavar="N", help="頻度表を見積もる標本の数")
    t.add_argument("--seed", type=int, default=1, metavar="N", help="乱数種（既定 1）")
    t.add_argument("--jobs", type=int, default=8, metavar="N", help="並列数。変えても出力は変わらない")
    t.add_argument("--force", action="store_true", help="出力が既にあっても作り直す")
    t.set_defaults(func=select)

    t = ss.add_parser(
        "relabel",
        help="DL系モデルの推論1回の評価値でscoreを付け直す",
        description="局面・指し手・勝敗・手数は残し、scoreだけをモデルの勝率から"
        "戻した値へ書き換える。勝率→評価値の変換は `cp = scale × logit(p)` で、"
        "scaleを600から下げることは推論側の FV_SCALE を上げることに当たる。"
        "本エンジンの探索で付け直す `hmwr data rescore` と違い、こちらは探索しない。",
    )
    t.add_argument("name", metavar="出力名", help="data/train/<出力名>.psv へ書く")
    t.add_argument("--in", dest="inputs", action="append", default=[], metavar="入力名")
    t.add_argument(
        "--labeler",
        default="dlshogi",
        choices=("dlshogi", "rescale"),
        help="裏側のモデルの種類（既定 dlshogi）。rescale は推論せず、既存のscoreを "
        "scale/600 倍に縮める。スケールの水準を振るとき推論を1回で済ませる",
    )
    t.add_argument(
        "--model",
        default=DL_MODEL,
        metavar="パス",
        help=f"モデルのファイル（既定 {DL_MODEL}）",
    )
    t.add_argument(
        "--scale",
        type=float,
        default=DL_SCALE,
        metavar="S",
        help=f"勝率→評価値の変換のスケール（既定 {DL_SCALE:g}）",
    )
    t.add_argument(
        "--device",
        choices=("cpu", "coreml", "cuda"),
        help="推論のデバイス。省くと使えるものを選ぶ",
    )
    t.add_argument("--batch", type=int, default=4096, metavar="N", help="推論のバッチ（既定 4096）")
    t.add_argument("--limit", type=int, metavar="N", help="先頭のこの件数だけ処理する")
    t.add_argument(
        "--in-place",
        action="store_true",
        help="出力名のpsvをその場で書き換える。--in は要らない。書き換える前のscoreは "
        "<名前>.psv.scores-before.i16 へ控え、進み具合を <名前>.psv.relabel.json に記録して"
        "途中から続けられる。大きなpsvで空きが無いときに使う",
    )
    t.add_argument("--start", type=int, default=0, metavar="N", help="--in-place の範囲の先頭（既定 0）")
    t.add_argument("--count", type=int, metavar="N", help="--in-place の範囲の件数（既定は末尾まで）")
    t.add_argument("--force", action="store_true", help="出力が既にあっても作り直す")
    t.set_defaults(func=relabel)

    t = ss.add_parser(
        "focus",
        help="対局順の生データから焦点のヒートマップつき局面集を作る",
        description="ある局面から先のk手で駒が動いたマスと取られたマスを"
        "9×9のヒートマップにし、盤上の駒ごとの関与フラグを付ける。"
        "入力は対局順のままの生データに限る。シャッフル済みの教師は続きを"
        "持たないので使えない。出力は202バイト固定長で、"
        "data/train/<出力名>.focus へ書く。",
    )
    t.add_argument("name", metavar="出力名", help=f"data/train/<出力名>{focus_labels.SUFFIX} へ書く")
    t.add_argument(
        "--raw",
        required=True,
        metavar="データセット",
        help="data/raw/<データセット>/ の生データ（*.bin）を名前順に読む",
    )
    t.add_argument("--count", type=int, required=True, metavar="N", help="切り出す局面数")
    t.add_argument(
        "--plies",
        type=int,
        default=focus_labels.PLIES,
        metavar="N",
        help=f"先を見る手数（既定 {focus_labels.PLIES}）",
    )
    t.add_argument("--force", action="store_true", help="出力が既にあっても作り直す")
    t.set_defaults(func=focus)


# --- 実行 --------------------------------------------------------------


def _inputs(op: Op, args: argparse.Namespace) -> list[Path]:
    raw = getattr(args, "raw", None)
    if raw:
        if args.inputs:
            raise proc.Fail("--in と --raw は同時に渡せない", proc.USAGE)
        raw_dir = paths.RAW / paths.check_name(raw)
        found = sorted(raw_dir.glob("*.bin"))
        if not found:
            if args.dry_run:
                return [raw_dir / "*.bin"]
            raise proc.Fail(f"生データがない: {paths.rel(raw_dir)}")
        return found

    n = len(args.inputs)
    if n < op.min_inputs or (op.max_inputs is not None and n > op.max_inputs):
        want = (
            f"{op.min_inputs}個"
            if op.max_inputs == op.min_inputs
            else f"{op.min_inputs}個以上"
        )
        raise proc.Fail(f"--in は{want}要る（{n}個渡された）", proc.USAGE)
    found = [paths.TRAIN / f"{paths.check_name(x)}.psv" for x in args.inputs]
    if not args.dry_run:
        for p in found:
            if not p.is_file():
                raise proc.Fail(f"入力のpsvがない: {paths.rel(p)}")
    return found


def _opt_args(op: Op, args: argparse.Namespace, *, skip: tuple[str, ...] = ()) -> list[str]:
    out: list[str] = []
    for o in op.opts:
        if o.flag in skip:
            continue
        value = getattr(args, dest(o.flag))
        if value is None:
            value = o.default
        if value is EVAL:
            value = config.get("EVAL_FILE")
            if not value:
                if not args.dry_run:
                    raise proc.Fail("評価関数がない。--eval-file で渡す")
                value = "（未設定）"
        if o.kind is bool:
            if value:
                out.append(o.flag)
            continue
        if value is not None:
            out += [o.flag, str(value)]
    return out


def _commands(op: Op, args: argparse.Namespace, part: Path) -> tuple[list[list[str]], list[Path]]:
    """走らせるpsvのコマンド列と、結合する区間の出力を返す。

    入力はリポジトリからの相対パスで渡す（実行時のcwdはリポジトリ）。
    コンマで連結した引数は `proc.show` が相対にできず、完了マーカーの記録が
    マシンの置き場に依存してしまう。
    """
    head = [str(psv_bin()), op.psv, "--in", ",".join(paths.rel(p) for p in _inputs(op, args))]
    jobs = getattr(args, "jobs", 1) if op.split_jobs else 1
    if not op.split_jobs or jobs <= 1:
        return [[*head, "--out", str(part), *_opt_args(op, args)]], []

    if args.limit is None:
        raise proc.Fail("--jobs で割るには --limit が要る", proc.USAGE)
    base = args.skip or 0
    per = -(-args.limit // jobs)
    rest = _opt_args(op, args, skip=("--skip", "--limit"))
    commands, pieces = [], []
    for k in range(jobs):
        count = min(per, args.limit - k * per)
        if count <= 0:
            break
        piece = part.with_name(f"{part.name}{k:03d}")
        pieces.append(piece)
        commands.append(
            [*head, "--out", str(piece), "--skip", str(base + k * per), "--limit", str(count), *rest]
        )
    return commands, pieces


def _concat(pieces: list[Path], out: Path) -> None:
    with open(out, "wb") as dst:
        for piece in pieces:
            with open(piece, "rb") as src:
                while chunk := src.read(1 << 24):
                    dst.write(chunk)
    for piece in pieces:
        piece.unlink()


def run(op: Op, args: argparse.Namespace) -> int:
    out = paths.TRAIN / f"{paths.check_name(args.name)}{op.suffix}"
    part = out.with_name(out.name + ".part")
    done = out.with_name(out.name + ".done")
    commands, pieces = _commands(op, args, part)
    record = [proc.show(c) for c in commands]
    log = paths.log(op.name, args.name)

    if args.dry_run:
        for c in commands:
            proc.run(c, dry_run=True)
        if pieces:
            print(f"[dry-run] {len(pieces)}区間を順に結合する → {paths.rel(part)}")
        print(f"[dry-run] mv {paths.rel(part)} {paths.rel(out)}")
        print(f"[dry-run] ログ: {paths.rel(log)}")
        return proc.OK

    if out.exists() and not args.force:
        return _already(out, done, record)
    if not psv_bin().is_file():
        raise proc.Fail(f"{paths.rel(psv_bin())} がない。先に cargo build --release を実行する")

    out.parent.mkdir(parents=True, exist_ok=True)
    done.unlink(missing_ok=True)
    print(f"=== data {op.name}: {args.name} ===")
    started = time.time()
    if pieces:
        proc.run_all(commands, log=log)
        _concat(pieces, part)
    else:
        proc.run(commands[0], log=log)
    if getattr(args, "consume", False):
        # 読み終えた入力は psv が消した。由来の記録だけが残らないよう、完了マーカーも消す
        for src in _inputs(op, args):
            src.with_name(src.name + ".done").unlink(missing_ok=True)
    part.replace(out)
    done.write_text(
        json.dumps(
            {
                "commands": record,
                "bytes": out.stat().st_size,
                "seconds": round(time.time() - started),
                "finished": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n"
    )
    print(f"完了: {paths.rel(out)}（{out.stat().st_size:,}バイト）")
    return proc.OK


def _already(out: Path, done: Path, record: list[str]) -> int:
    """出力が既にあるときの扱い。同じ条件なら済み、違えば止める。"""
    if not done.is_file():
        raise proc.Fail(
            f"出力が既にあり、由来の記録がない: {paths.rel(out)}\n"
            "名前を変えるか、--force で作り直す"
        )
    before = json.loads(done.read_text()).get("commands")
    if before != record:
        raise proc.Fail(
            f"同じ名前で条件の違う出力がある: {paths.rel(out)}\n"
            f"前回: {before}\n今回: {record}\n"
            "名前を変えるか、--force で作り直す"
        )
    print(f"済み: {paths.rel(out)}（同じ条件で作成済み。何もしない）")
    return proc.OK


# --- 確認 --------------------------------------------------------------


def stats(args: argparse.Namespace) -> int:
    source = paths.TRAIN / f"{paths.check_name(args.name)}.psv"
    if not source.is_file() and not args.dry_run:
        raise proc.Fail(f"入力のpsvがない: {paths.rel(source)}")
    argv = [str(psv_bin()), "stats", "--in", paths.rel(source)]
    if args.limit is not None:
        argv += ["--limit", str(args.limit)]
    return proc.run(argv, dry_run=args.dry_run)


# --- 片付け ------------------------------------------------------------


def remove(args: argparse.Namespace) -> int:
    """完了マーカーのある出力だけを消す。消す前に全部の対象を確かめる。"""
    targets: list[tuple[Path, Path]] = []
    for name in args.names:
        paths.check_name(name)
        if args.dry_run:
            # 消す対象は、手順の前のステップが実行時に作る。予行では存在を問わない
            print(
                f"[dry-run] rm {paths.rel(paths.TRAIN / name)} の"
                ".psv・.rankpsv・.focus と、その完了マーカー"
            )
            continue
        found = [
            p
            for suffix in (".psv", ".rankpsv", focus_labels.SUFFIX)
            if (p := paths.TRAIN / f"{name}{suffix}").exists()
        ]
        if not found:
            raise proc.Fail(f"消す対象がない: {name}")
        for out in found:
            done = out.with_name(out.name + ".done")
            if not done.is_file():
                raise proc.Fail(
                    f"由来の記録がないので消さない: {paths.rel(out)}\n"
                    "hmwr data が作ったファイルだけを消せる"
                )
            targets.append((out, done))

    for out, done in targets:
        size = out.stat().st_size
        out.unlink()
        done.unlink()
        print(f"消した: {paths.rel(out)}（{size:,}バイト）")
    return proc.OK


# --- 開始局面集 --------------------------------------------------------

PSV_BYTES = 40
PLY_OFFSET = 36  # game_ply（u16、リトルエンディアン）


def openings(args: argparse.Namespace) -> int:
    """手数の条件で局面を拾い、SFENの列挙へ書き出す。"""
    if len(args.inputs) != 1:
        raise proc.Fail(f"--in は1個要る（{len(args.inputs)}個渡された）", proc.USAGE)
    source = paths.TRAIN / f"{paths.check_name(args.inputs[0])}.psv"
    out = paths.REPO / "openings" / f"{paths.check_name(args.name)}.txt"

    if args.dry_run:
        print(
            f"[dry-run] {paths.rel(source)} の{args.skip}件目から、"
            f"{args.min_ply}手目以降を{args.count}局面拾う"
        )
        print(f"[dry-run] {paths.rel(psv_bin())} dump で復元し {paths.rel(out)} へ書く")
        return proc.OK
    if not source.is_file():
        raise proc.Fail(f"入力のpsvがない: {paths.rel(source)}")
    if out.exists() and not args.force:
        raise proc.Fail(f"出力が既にある: {paths.rel(out)}\n名前を変えるか、--force で作り直す")

    picked = bytearray()
    with open(source, "rb") as fh:
        fh.seek(args.skip * PSV_BYTES)
        while len(picked) < args.count * PSV_BYTES:
            chunk = fh.read(PSV_BYTES * 65536)
            if not chunk:
                break
            for i in range(0, len(chunk) - PSV_BYTES + 1, PSV_BYTES):
                (ply,) = struct.unpack_from("<H", chunk, i + PLY_OFFSET)
                if ply >= args.min_ply:
                    picked += chunk[i : i + PSV_BYTES]
                    if len(picked) >= args.count * PSV_BYTES:
                        break
    found = len(picked) // PSV_BYTES
    if found < args.count:
        raise proc.Fail(f"条件を満たす局面が足りない（{found}/{args.count}）")

    with tempfile.NamedTemporaryFile(suffix=".psv") as tmp:
        tmp.write(picked)
        tmp.flush()
        dumped, err = proc.capture_both(
            [str(psv_bin()), "dump", "--in", tmp.name, "--limit", str(args.count)]
        )
    lines = [f"sfen {row.split(' | ')[0]}" for row in dumped.splitlines() if " | " in row]
    if len(lines) != args.count:
        raise proc.Fail(f"局面を復元できなかった（{len(lines)}/{args.count}）: {err.strip()}")
    out.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"完了: {paths.rel(out)}（{len(lines)}局面）")
    return proc.OK


# --- DL系モデルによる付け直し ------------------------------------------


def make_labeler(args: argparse.Namespace):
    """裏側のモデルを作る。テストでは差し替える。"""
    from .tools import dl_relabel

    if args.labeler == "rescale":
        return dl_relabel.RescaleLabeler()
    model = paths.REPO / args.model
    if not model.is_file():
        raise proc.Fail(
            f"モデルがない: {paths.rel(model)}\n"
            "配布元からライセンスに同意して取得し、この場所へ置く（Issue #556）"
        )
    return dl_relabel.DlshogiLabeler(model, args.device)


def relabel_in_place(args: argparse.Namespace) -> int:
    """出力名のpsvをその場で書き換える。進み具合は relabel.json が持つ。"""
    from .tools import dl_relabel

    if args.inputs:
        raise proc.Fail("--in-place では --in を渡さない。書き換える名前だけを渡す", proc.USAGE)
    target = paths.TRAIN / f"{paths.check_name(args.name)}.psv"
    model = "" if args.labeler == "rescale" else f" --model {args.model}"
    record = {"labeler": args.labeler, "model": args.model if args.labeler != "rescale" else None,
              "scale": args.scale}
    log = paths.log("relabel", args.name)
    if args.dry_run:
        rng = f"[{args.start}, {args.start + args.count})" if args.count else f"[{args.start}, 末尾)"
        print(f"[dry-run] relabel --in-place --labeler {args.labeler}{model} --scale {args.scale:g}"
              f" {paths.rel(target)} の {rng} をその場で書き換える")
        print(f"[dry-run] 元のscore: {paths.rel(dl_relabel.sidecar_path(target))}")
        print(f"[dry-run] 進み具合: {paths.rel(dl_relabel.progress_path(target))}")
        print(f"[dry-run] ログ: {paths.rel(log)}")
        return proc.OK
    if not target.is_file():
        raise proc.Fail(f"psvがない: {paths.rel(target)}")
    total = target.stat().st_size // dl_relabel.PSV_BYTES
    count = args.count or (total - args.start)
    labeler = make_labeler(args)
    print(f"=== data relabel（その場）: {args.name} [{args.start}, {args.start + count}) ===")
    device = getattr(labeler, "device", None)
    if device:
        print(f"デバイス: {device}")
    with open(log, "a", encoding="utf-8") as fh:

        def report(line: str) -> None:
            stamp = time.strftime("%Y-%m-%dT%H:%M:%S")
            print(line)
            fh.write(f"{stamp} {line}\n")
            fh.flush()

        report(f"開始: in-place {record} start={args.start} count={count}")
        beat = heartbeat.Heartbeat(
            "relabel", args.name, total=count, unit="局面", log=log,
            detail={"labeler": args.labeler, "scale": args.scale, "start": args.start, "device": device, "in_place": True},
        )
        try:
            state = dl_relabel.relabel_in_place(
                target, labeler, scale=args.scale, start=args.start, count=count,
                record=record, batch=args.batch, report=report, progress=beat.update,
            )
        except ValueError as e:
            beat.finish("failed", error=str(e).splitlines()[0])
            raise proc.Fail(str(e)) from e
        except BaseException as e:
            beat.finish("failed", error=type(e).__name__)
            raise
        beat.finish("done")
        report(f"終了: {state['done']:,}局面を書き換えた")
    print(f"完了: {paths.rel(target)}（元のscoreは {paths.rel(dl_relabel.sidecar_path(target))}）")
    return proc.OK


def relabel(args: argparse.Namespace) -> int:
    """scoreだけをDL系モデルの値へ書き換える。完了マーカーの扱いは他の操作と同じ。"""
    from .tools import dl_relabel

    if args.in_place:
        return relabel_in_place(args)
    if len(args.inputs) != 1:
        raise proc.Fail(f"--in は1個要る（{len(args.inputs)}個渡された）", proc.USAGE)
    source = paths.TRAIN / f"{paths.check_name(args.inputs[0])}.psv"
    out = paths.TRAIN / f"{paths.check_name(args.name)}.psv"
    part = out.with_name(out.name + ".part")
    done = out.with_name(out.name + ".done")
    model = "" if args.labeler == "rescale" else f" --model {args.model}"
    record = [
        f"relabel --labeler {args.labeler}{model} --scale {args.scale:g}"
        f" --in {paths.rel(source)}"
        + (f" --limit {args.limit}" if args.limit is not None else "")
    ]
    log = paths.log("relabel", args.name)

    if args.dry_run:
        print(f"[dry-run] {record[0]} → {paths.rel(part)}")
        print(f"[dry-run] mv {paths.rel(part)} {paths.rel(out)}")
        print(f"[dry-run] ログ: {paths.rel(log)}")
        return proc.OK
    if not source.is_file():
        raise proc.Fail(f"入力のpsvがない: {paths.rel(source)}")
    if out.exists() and not args.force:
        return _already(out, done, record)

    labeler = make_labeler(args)
    out.parent.mkdir(parents=True, exist_ok=True)
    done.unlink(missing_ok=True)
    print(f"=== data relabel: {args.name} ===")
    device = getattr(labeler, "device", None)
    if device:
        print(f"デバイス: {device}")
    with open(log, "a", encoding="utf-8") as fh:

        def report(line: str) -> None:
            stamp = time.strftime("%Y-%m-%dT%H:%M:%S")
            print(line)
            fh.write(f"{stamp} {line}\n")
            fh.flush()

        report(f"開始: {record[0]}")
        total = source.stat().st_size // dl_relabel.PSV_BYTES
        beat = heartbeat.Heartbeat(
            "relabel", args.name, total=min(total, args.limit) if args.limit else total, unit="局面",
            log=log, detail={"labeler": args.labeler, "scale": args.scale, "device": device},
        )
        try:
            stats = dl_relabel.relabel(
                source, part, labeler, scale=args.scale, batch=args.batch, limit=args.limit,
                report=report, progress=beat.update,
            )
        except BaseException as e:
            beat.finish("failed", error=type(e).__name__)
            raise
        beat.finish("done", **{k: stats[k] for k in ("positions_per_second", "corr_old_new")})
        report(
            f"終了: {stats['positions']:,}局面 {stats['positions_per_second']:,}局面/秒 "
            f"相関 {stats['corr_old_new']} 平均|score| {stats['mean_abs_old']}→{stats['mean_abs_new']}"
        )
    part.replace(out)
    done.write_text(
        json.dumps(
            {
                "commands": record,
                "bytes": out.stat().st_size,
                "seconds": stats["seconds"],
                "stats": stats,
                "finished": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n"
    )
    print(f"完了: {paths.rel(out)}（{out.stat().st_size:,}バイト）")
    return proc.OK


# --- 焦点のヒートマップ --------------------------------------------------


def focus(args: argparse.Namespace) -> int:
    """対局順の生データから、先k手の焦点を付けた局面集を作る（ADR-0213）。"""
    name = paths.check_name(args.name)
    dataset = paths.check_name(args.raw)
    raw_dir = paths.RAW / dataset
    out = paths.TRAIN / f"{name}{focus_labels.SUFFIX}"
    part = out.with_name(out.name + ".part")
    done = out.with_name(out.name + ".done")
    record = [f"focus --raw {dataset} --count {args.count} --plies {args.plies}"]
    log = paths.log("focus", name)
    sources = sorted(raw_dir.glob("*.bin"))

    if args.dry_run:
        print(f"[dry-run] {record[0]} → {paths.rel(part)}")
        print(f"[dry-run] 入力: {paths.rel(raw_dir)}/*.bin（{len(sources)}ファイル）")
        print(f"[dry-run] mv {paths.rel(part)} {paths.rel(out)}")
        print(f"[dry-run] ログ: {paths.rel(log)}")
        return proc.OK
    if not sources:
        raise proc.Fail(f"生データがない: {paths.rel(raw_dir)}")
    if out.exists() and not args.force:
        return _already(out, done, record)

    out.parent.mkdir(parents=True, exist_ok=True)
    done.unlink(missing_ok=True)
    print(f"=== data focus: {name} ===")
    with open(log, "a", encoding="utf-8") as fh:

        def report(line: str) -> None:
            print(line)
            fh.write(f"{time.strftime('%Y-%m-%dT%H:%M:%S')} {line}\n")
            fh.flush()

        report(f"開始: {record[0]}")
        stats = focus_labels.write(
            sources, part, count=args.count, plies=args.plies, report=report
        )
        if stats.rows < args.count:
            raise proc.Fail(f"切り出せた局面が足りない（{stats.rows}/{args.count}）")
        focus_labels.report_stats(stats, plies=args.plies, report=report)

    part.replace(out)
    done.write_text(
        json.dumps(
            {
                "commands": record,
                "bytes": out.stat().st_size,
                "seconds": round(stats.seconds),
                "stats": stats.summary(),
                "finished": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n"
    )
    print(f"完了: {paths.rel(out)}（{out.stat().st_size:,}バイト）")
    return proc.OK


# --- 区画で選ぶ（ADR-0227） ---------------------------------------------


def _ratios(text: str) -> list[float]:
    try:
        v = [float(x) for x in text.split(",")]
    except ValueError:
        raise proc.Fail(f"配分はコンマ区切りの3つの数で書く: {text}", proc.USAGE)
    if len(v) != 3 or abs(sum(v) - 1) > 1e-6:
        raise proc.Fail(f"配分は3つで、和を1にする: {text}", proc.USAGE)
    return v


def _run_cells(args, kind: str, record: list[str], sources: list[Path], body) -> int:
    out = paths.TRAIN / f"{paths.check_name(args.name)}.psv"
    part = out.with_name(out.name + ".part")
    done = out.with_name(out.name + ".done")
    log = paths.log(kind, args.name)
    if args.dry_run:
        print(f"[dry-run] {record[0]} → {paths.rel(part)}")
        print(f"[dry-run] mv {paths.rel(part)} {paths.rel(out)}")
        print(f"[dry-run] ログ: {paths.rel(log)}")
        return proc.OK
    for src in sources:
        if not src.is_file():
            raise proc.Fail(f"入力のpsvがない: {paths.rel(src)}")
    if out.exists() and not args.force:
        return _already(out, done, record)
    done.unlink(missing_ok=True)
    total = sum(src.stat().st_size for src in sources) // 40
    started = time.time()
    with open(log, "a", encoding="utf-8") as fh:

        def report(line: str) -> None:
            print(line, flush=True)
            fh.write(f"{time.strftime('%Y-%m-%dT%H:%M:%S')} {line}\n")
            fh.flush()

        report(f"開始: {record[0]}")
        with heartbeat.running(kind, args.name, total=total, unit="局面", log=log) as beat:
            stats = body(part, beat.update, report)
        report(f"終了: {stats['read']:,}局面を読み、{stats['kept']:,}局面を残した")
    part.replace(out)
    done.write_text(
        json.dumps(
            {"commands": record, "bytes": out.stat().st_size, "seconds": round(time.time() - started),
             "stats": stats, "finished": time.strftime("%Y-%m-%dT%H:%M:%S%z")},
            ensure_ascii=False, indent=2,
        )
        + "\n"
    )
    print(f"完了: {paths.rel(out)}（{out.stat().st_size:,}バイト）")
    return proc.OK


def pick(args: argparse.Namespace) -> int:
    """玉の段と勝率で局面を絞り込む（ADR-0227）。"""
    from .tools import cells_run

    if len(args.inputs) != 1:
        raise proc.Fail(f"--in は1個要る（{len(args.inputs)}個渡された）", proc.USAGE)
    source = paths.TRAIN / f"{paths.check_name(args.inputs[0])}.psv"
    record = [
        f"pick --in {paths.rel(source)} --king-max {args.king_max} --p-min {args.p_min:g}"
        f" --p-max {args.p_max:g} --scale {args.scale:g}"
    ]

    def body(part, progress, report):
        return cells_run.pick(source, part, king_max=args.king_max, pmin=args.p_min, pmax=args.p_max,
                              scale=args.scale, jobs=args.jobs, progress=progress)

    return _run_cells(args, "pick", record, [source], body)


def select(args: argparse.Namespace) -> int:
    """区画の配分とαで局面を選ぶ（ADR-0227）。"""
    from .tools import cells_run

    if not args.inputs:
        raise proc.Fail("--in が要る", proc.USAGE)
    sources = [paths.TRAIN / f"{paths.check_name(n)}.psv" for n in args.inputs]
    king, evals = _ratios(args.king), _ratios(args.evals)
    record = [
        "select " + " ".join(f"--in {paths.rel(s)}" for s in sources)
        + f" --king {args.king} --eval {args.evals} --count {args.count} --alpha {args.alpha:g}"
        f" --scale {args.scale:g} --sample {args.sample} --seed {args.seed}"
    ]

    def body(part, progress, report):
        return cells_run.select(sources, part, king=king, evals=evals, count=args.count, alpha=args.alpha,
                                eval_scale=args.scale, sample=args.sample, seed=args.seed, jobs=args.jobs,
                                progress=progress, report=report)

    return _run_cells(args, "select", record, sources, body)


# --- 点数を付けた局面の台帳（ADR-0228） ---------------------------------


def ledger_add(args: argparse.Namespace) -> int:
    """局面のハッシュを台帳へ足す。同じ入力を二度足しても台帳は変わらない。"""
    from .tools import ledger

    if not args.inputs:
        raise proc.Fail("--in が要る", proc.USAGE)
    sources = [paths.TRAIN / f"{paths.check_name(n)}.psv" for n in args.inputs]
    if args.dry_run:
        print(f"[dry-run] 台帳 {paths.rel(paths.LEDGER)} へ足す: " + ", ".join(paths.rel(s) for s in sources))
        return proc.OK
    for src in sources:
        if not src.is_file():
            raise proc.Fail(f"入力のpsvがない: {paths.rel(src)}")
    total = sum(src.stat().st_size for src in sources) // 40
    with heartbeat.running("ledger", "add", total=total, unit="局面") as beat:
        stats = ledger.add(paths.LEDGER, sources, progress=beat.update)
    print(f"台帳へ足した: {stats['read']:,}局面を読み、新しい局面 {stats['new']:,}。台帳は {stats['size']:,}局面")
    return proc.OK


def ledger_filter(args: argparse.Namespace) -> int:
    """台帳に無く、入力の中でも初出の局面だけを残す。"""
    from .tools import ledger

    if len(args.inputs) != 1:
        raise proc.Fail(f"--in は1個要る（{len(args.inputs)}個渡された）", proc.USAGE)
    source = paths.TRAIN / f"{paths.check_name(args.inputs[0])}.psv"
    record = [f"ledger filter --in {paths.rel(source)}"]

    def body(part, progress, report):
        report(f"台帳: {ledger.size(paths.LEDGER):,}局面")
        stats = ledger.filter_new(paths.LEDGER, source, part, progress=progress)
        report(f"入力の中の重複 {stats['dup_in_input']:,}、台帳にあった局面 {stats['known']:,}")
        return stats

    return _run_cells(args, "ledger", record, [source], body)


def ledger_show(args: argparse.Namespace) -> int:
    from .tools import ledger

    print(f"台帳: {paths.rel(paths.LEDGER)}  {ledger.size(paths.LEDGER):,}局面")
    log = paths.LEDGER / "added.jsonl"
    if log.is_file():
        for line in log.read_text(encoding="utf-8").splitlines():
            d = json.loads(line)
            print(f"  {d['at']}  +{d['new']:,}（{d['read']:,}局面を読んだ）  " + ", ".join(Path(s).name for s in d["sources"]))
    return proc.OK


def merge_op(args: argparse.Namespace) -> int:
    """シャッフル済みの教師を織り交ぜて1本にする（ADR-0228）。"""
    from .tools import merge

    if len(args.inputs) < 2:
        raise proc.Fail("--in は2個以上要る", proc.USAGE)
    sources = [paths.TRAIN / f"{paths.check_name(n)}.psv" for n in args.inputs]
    record = [
        "merge " + " ".join(f"--in {paths.rel(s)}" for s in sources)
        + f" --seed {args.seed}" + (" --consume" if args.consume else "")
    ]

    def body(part, progress, report):
        stats = merge.merge(sources, part, seed=args.seed, consume=args.consume, progress=progress)
        if args.consume:
            for src in sources:
                src.with_name(src.name + ".done").unlink(missing_ok=True)
        return stats

    return _run_cells(args, "merge", record, sources, body)


def floodgate_op(args: argparse.Namespace) -> int:
    """floodgateの全対局から局面を取り出す（ADR-0228）。"""
    from .tools import floodgate_psv

    lo, _, hi = args.years.partition("-")
    try:
        years = list(range(int(lo), int(hi or lo) + 1))
    except ValueError:
        raise proc.Fail(f"--years は 2011-2026 か 2026 の形で書く: {args.years}", proc.USAGE)
    archive = paths.REPO / "data" / "raw" / "floodgate_archive"
    record = [
        f"floodgate --years {args.years} --tag {args.tag} --min-plies {args.min_plies}"
        + (f" --limit {args.limit}" if args.limit else "")
    ]

    def body(part, progress, report):
        return floodgate_psv.convert(years, archive, part, tag=args.tag, min_plies=args.min_plies,
                                     jobs=args.jobs, limit=args.limit, report=report, progress=progress)

    return _run_cells(args, "floodgate", record, [], body)
