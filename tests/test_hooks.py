"""ツールの直接実行を止めるhookを検証する（ADR-0208）。"""

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
        # 単発の診断も hmwr diag が包んでいる
        "./target/release/psv defend --in a --out b.tsv",
        "./target/release/psv phase --in a --out b.tsv",
        "./target/release/psv oversample --in a --out b --kind defense --times 3",
        # 探索での付け直しも hmwr data rescore が包んでいる
        "./target/release/psv relabel --in a --out b --depth 9 --jobs 8",
        "./target/release/psv thin --in a --out b --threshold 2000",
        "./target/release/psv dedup --in a --out b --count 10",
        "cargo run --release -p himawari-tools --bin psv -- relabel --in a --out b",
        # スレッド数の測定も hmwr threadtune が包んでいる
        "./target/release/himawari threadtune --eval data/nets/x.hmwr --hours 8",
        "data/bin/himawari-pgo threadtune --eval data/nets/x.hmwr",
        "nohup ./data/bin/base-x threadtune --nps-only &",
        "cargo run --release -p himawari-usi --bin himawari -- threadtune --eval x",
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
        "./target/release/psv dump --in a --limit 3",
        "./target/release/gensfen --out x --eval y",
        "cargo run --release -p himawari-tools --bin verify -- a b",
        # エンジンをUSIとして起動するのは止めない。包んだのは threadtune だけである
        "./target/release/himawari",
        "echo 'usi' | ./data/bin/himawari-pgo",
        "hmwr threadtune --current 6 --hours 8",
        "grep -n threadtune crates/usi/src/main.rs",
    ],
)
def test_everything_else_passes(command):
    assert not hook.blocked(command)
