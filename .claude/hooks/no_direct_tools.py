#!/usr/bin/env python3
"""hmwr がラップするツールの直接実行を止める（ADR-0208）。

実験のチェーンが psv と selfplay を直接叩き、既定値がチェーンごとにずれた。
入口を `hmwr` に限れば、既定値・ログの置き場・完了マーカーが1か所で決まる。

**止めるのは、hmwr に同じ操作があるものだけである。** 先に止めると作業が
詰まり、抜け道が作られる。psv の未被覆のサブコマンド（relabel・thin など）と
gensfen（Issue #509）は通す。hmwr へ包んだら、ここの一覧へ足す。

PreToolUseのhookとして、Bashのコマンドを標準入力のJSONで受け取る。終了コード
2で止め、標準エラーの文がエージェントへ返る。
"""

import json
import re
import sys

# hmwr が包んだ psv のサブコマンド。data と diag の両方を見る
PSV_COVERED = "head|shuffle|quiet|rank|stats|phase|defend|oversample"
# コマンドの位置にあるものだけを見る。ls や grep の引数に出てくるパスは止めない
COMMAND_START = r"(?:^|[;&|(\n]|\$\(|\bexec\s|\bnohup\s|\btime\s)\s*(?:\w+=\S+\s+)*"
BINARY = r"(?:\S*/)?target/release/"
DIRECT = re.compile(
    COMMAND_START + BINARY + rf"(?:selfplay(?=\s|$)|psv\s+(?:{PSV_COVERED})\b)"
)
VIA_CARGO = re.compile(
    rf"\bcargo\s+run\b[^;&|\n]*--bin\s+(?:selfplay\b|psv\s+--\s+(?:{PSV_COVERED})\b)"
)

MESSAGE = """\
この操作は直接実行しない。hmwr 経由で実行する（ADR-0208）。
  psv      → hmwr data shuffle / split / mix / quiet / rank / oversample / stats / openings
  psv      → hmwr diag phase / defend（単発の診断）
  selfplay → hmwr match run（固定ペア数は --stop pairs:N）
足りない操作は hmwr/dataops.py の対応表へ足すか、micro Issueを切る。
"""


def blocked(command: str) -> bool:
    return bool(DIRECT.search(command) or VIA_CARGO.search(command))


def main() -> int:
    try:
        command = json.load(sys.stdin).get("tool_input", {}).get("command", "")
    except (json.JSONDecodeError, AttributeError):
        return 0
    if blocked(command):
        sys.stderr.write(MESSAGE)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
