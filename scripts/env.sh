#!/usr/bin/env bash
# マシンごとに変わる設定をまとめる。各スクリプトから source する。
#
# 値は自動で決める。変えたいときは呼び出し側で環境変数を先に設定する。
#   SPRT_CONCURRENCY=6 scripts/sprt.sh base cand

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export REPO_ROOT

# 物理コア数。ハイパースレッドの論理プロセッサは数えない。
# SPRTは1局1スレッドで回すため、論理数まで積むと持ち時間の消化が
# 不安定になり、測定がぶれる。
detect_physical_cores() {
	if [[ "$(uname -s)" == "Darwin" ]]; then
		# Apple Siliconは高性能コアだけを数える。効率コアは大きく遅く、
		# 混ぜると同じ持ち時間でも到達深さがばらつく
		sysctl -n hw.perflevel0.physicalcpu 2>/dev/null || sysctl -n hw.physicalcpu
	elif command -v lscpu >/dev/null 2>&1; then
		lscpu -p=Core,Socket 2>/dev/null | grep -v '^#' | sort -u | wc -l
	else
		nproc 2>/dev/null || echo 4
	fi
}

CORES="$(detect_physical_cores)"
# 1コアはOSと計測用に空ける。上限8はADR-0028の既定条件に合わせる。
# 過去の測定と条件を揃えるため、コアが余っていても8を超えない
_conc=$((CORES > 2 ? CORES - 1 : 1))
export SPRT_CONCURRENCY="${SPRT_CONCURRENCY:-$((_conc < 8 ? _conc : 8))}"

# 現行の最強構成（ROADMAP.md の「現行の最強構成」と揃える）
export EVAL_FILE="${EVAL_FILE:-${REPO_ROOT}/data/nets/halfkp_1900M_fact.hmwr.best}"
export OPENINGS="${OPENINGS:-${REPO_ROOT}/openings/start_sfens_ply24.txt}"

# SPRTの既定条件（ADR-0028）
export SPRT_TC="${SPRT_TC:-10+0.1}"
export SPRT_ELO0="${SPRT_ELO0:-0}"
export SPRT_ELO1="${SPRT_ELO1:-5}"
export SPRT_ALPHA="${SPRT_ALPHA:-0.05}"
export SPRT_BETA="${SPRT_BETA:-0.05}"
export SPRT_ADJUDICATE="${SPRT_ADJUDICATE:-2000,8}"
export SPRT_MAX_PAIRS="${SPRT_MAX_PAIRS:-3000}"

export RUSTFLAGS_NATIVE="${RUSTFLAGS_NATIVE:--C target-cpu=native}"

env_summary() {
	cat <<SUMMARY
物理コア      : ${CORES}
SPRT並列度    : ${SPRT_CONCURRENCY}
評価関数      : ${EVAL_FILE}
持ち時間      : ${SPRT_TC}
SUMMARY
}
