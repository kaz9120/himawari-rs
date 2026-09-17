"""道具の直接実行を止めるhookを検証する（ADR-0208）。"""

import importlib.util
from pathlib import Path

import pytest

HOOK = Path(__file__).resolve().parent.parent / ".claude" / "hooks" / "no_direct_tools.py"
spec = importlib.util.spec_from_file_location("no_direct_tools", HOOK)
hook = importlib.util.module_from_spec(spec)
spec.loader.exec_module(hook)


@pytest.mark.parametrize(
    "command",
    [
        "./target/release/psv head --in a --out b --count 1",
        "target/release/psv shuffle --in a,b --out c",
        "cd repo && ./target/release/selfplay --baseline a --candidate b",
        "PSV=x /abs/path/target/release/psv quiet --in a --out b",
        "nohup ./target/release/selfplay --baseline a &",
        "cargo run --release -q -p himawari-tools --bin psv -- rank --in a",
        "cargo run --release -p himawari-tools --bin selfplay -- --baseline a",
    ],
)
def test_covered_operations_are_blocked(command):
    assert hook.blocked(command)


@pytest.mark.parametrize(
    "command",
    [
        "./bin/hmwr data split a --in b --count 1",
        "hmwr match run x --stop pairs:10",
        "ls -la target/release/psv target/release/selfplay",
        "grep -n rank crates/tools/src/bin/psv.rs",
        "cargo build --release -p himawari-tools --bin psv",
        # hmwr にまだ無い操作は止めない。止めると抜け道が作られる
        "./target/release/psv relabel --in a --out b",
        "./target/release/psv dump --in a --limit 3",
        "./target/release/gensfen --out x --eval y",
        "cargo run --release -p himawari-tools --bin verify -- a b",
    ],
)
def test_everything_else_passes(command):
    assert not hook.blocked(command)
