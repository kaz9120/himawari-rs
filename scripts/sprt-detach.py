#!/usr/bin/env python3
"""SPRTを親から切り離して起動する（ADR-0175）。

``sprt-run.sh`` をそのまま実行すると、呼び出し元のプロセスグループに属する。
エージェントがツールのバックグラウンド実行として走らせた場合、数十分で
グループごと回収されて止まる（2026-08-18に2回。標準出力を捨てても起きたので
出力量とは無関係）。棋譜は残るので結果を失いはしないが、そのたびに再開が要る。

``start_new_session=True``（setsid相当）で新しいセッションへ移し、親の終了と
無関係に走らせる。**切り離してよいのは、状態がファイルにあるからである。**
進捗は ``data/logs/sprt-<名前>.log``、完了は ``data/sprt/<名前>.result`` で
分かるので、プロセスを手元で掴んでおく必要がない（ADR-0175）。

使い方:
  scripts/sprt-detach.py <baseline> <candidate> <名前> [KEY=VALUE...]

例:
  scripts/sprt-detach.py data/bin/base-adr0174 data/bin/cand-adr0174 adr0174
  scripts/sprt-detach.py base cand adr0174 SPRT_ELO0=-5 SPRT_ELO1=0

終了コード: 0=起動した、2=引数エラー、3=実行時エラー（ADR-0122）。
"""

import os
import pathlib
import subprocess
import sys

USAGE = """\
使い方:
  scripts/sprt-detach.py <baseline> <candidate> <名前> [KEY=VALUE...]

sprt-run.sh を新しいセッションで起動し、すぐに戻る。判定が出るまで走り続ける。
KEY=VALUE は環境変数として渡す（例: SPRT_ELO0=-5 SPRT_ELO1=0）。

進捗: python3 scripts/sprt-summary.py data/logs/sprt-<名前>.log
完了: data/sprt/<名前>.result があれば判定済み
"""


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    if argv and argv[0] in ("-h", "--help"):
        print(USAGE, end="")
        return 0
    if len(argv) < 3:
        print(USAGE, end="", file=sys.stderr)
        return 2

    baseline, candidate, name = argv[0], argv[1], argv[2]
    env = dict(os.environ)
    for item in argv[3:]:
        if "=" not in item:
            print(f"エラー: KEY=VALUE の形式でない: {item}", file=sys.stderr)
            return 2
        key, value = item.split("=", 1)
        env[key] = value

    repo = pathlib.Path(__file__).resolve().parent.parent
    result = repo / "data" / "sprt" / f"{name}.result"
    if result.is_file():
        # 判定済みなら起動しない。sprt-run.sh 側も冪等だが、ここで返すほうが速い
        print(f"判定済み: {result}")
        print(result.read_text(encoding="utf-8"), end="")
        return 0

    runner = repo / "scripts" / "sprt-run.sh"
    if not runner.is_file():
        print(f"エラー: {runner} がない", file=sys.stderr)
        return 3

    try:
        with open(os.devnull, "wb") as devnull:
            proc = subprocess.Popen(
                [str(runner), baseline, candidate, name],
                cwd=str(repo),
                env=env,
                stdout=devnull,
                stderr=devnull,
                stdin=devnull,
                start_new_session=True,
            )
    except OSError as e:
        print(f"エラー: 起動できない: {e}", file=sys.stderr)
        return 3

    print(f"起動した pid={proc.pid}（新しいセッション）")
    print(f"進捗: python3 scripts/sprt-summary.py data/logs/sprt-{name}.log")
    print(f"完了: data/sprt/{name}.result の出現を見る")
    return 0


if __name__ == "__main__":
    sys.exit(main())
