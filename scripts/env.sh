#!/usr/bin/env bash
# マシンごとに変わる設定をまとめる。各スクリプトから source する。
#
# 値は自動で決める。変えたいときは呼び出し側で環境変数を先に設定する。
#   SPRT_CONCURRENCY=6 hmwr sprt run <名前>

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

# 現行構成（ROADMAP.md の「現行構成」と揃える）
export EVAL_FILE="${EVAL_FILE:-${REPO_ROOT}/data/nets/pairprod_2990M_q1_reorder.hmwr}"
export OPENINGS="${OPENINGS:-${REPO_ROOT}/openings/start_sfens_ply24.txt}"

# SPRTの既定条件（ADR-0028）
export SPRT_TC="${SPRT_TC:-10+0.1}"
export SPRT_ELO0="${SPRT_ELO0:-0}"
export SPRT_ELO1="${SPRT_ELO1:-5}"
export SPRT_ALPHA="${SPRT_ALPHA:-0.05}"
export SPRT_BETA="${SPRT_BETA:-0.05}"
export SPRT_ADJUDICATE="${SPRT_ADJUDICATE:-2000,8}"
export SPRT_MAX_PAIRS="${SPRT_MAX_PAIRS:-3000}"
# 判定が出るまで走らせるときの硬い上限（ADR-0175）。
# ここは収束の判定基準ではなく暴走を止める安全弁である。真のEloが
# 対立仮説の中点ちょうどだと理論上収束しないため、無制限にはしない。
# 60,000ペア＝12万局は、非劣性で真のEloが+0.5のときの必要局数
# （約48,000ペア）を上回る値として置く。ここに達しても判定が出ないなら、
# 局数を積むより仮説の立て方を見直す状況である
export SPRT_HARD_MAX_PAIRS="${SPRT_HARD_MAX_PAIRS:-60000}"

export RUSTFLAGS_NATIVE="${RUSTFLAGS_NATIVE:--C target-cpu=native}"

env_summary() {
	cat <<SUMMARY
物理コア      : ${CORES}
SPRT並列度    : ${SPRT_CONCURRENCY}
評価関数      : ${EVAL_FILE}
持ち時間      : ${SPRT_TC}
SUMMARY
}

# --- 共通ログ関数 -----------------------------------------------------
#
# タイムスタンプは既定で付けない。release-*.sh のように数秒で終わる
# スクリプトでは、出力を目で追うだけなので不要である。長時間ポーリングする
# スクリプトだけが、source後に LOG_TIMESTAMP=1 を立てて使う。
LOG_TIMESTAMP="${LOG_TIMESTAMP:-0}"
export LOG_TIMESTAMP

# 色は端末に出しているときだけ付ける。リダイレクト先がファイルだと
# エスケープシーケンスがそのまま文字として混ざり、後から読むログが
# 逆に読みにくくなるため、[[ -t 1 ]] で端末かどうかを見て決める。
if [[ -t 1 ]]; then
	_LOG_YELLOW=$'\033[33m'
	_LOG_RED=$'\033[31m'
	_LOG_RESET=$'\033[0m'
else
	_LOG_YELLOW=""
	_LOG_RED=""
	_LOG_RESET=""
fi

_log_prefix() {
	if [[ "$LOG_TIMESTAMP" == "1" ]]; then
		printf '[%s] ' "$(date '+%H:%M:%S')"
	fi
}

# 見出し。段落の区切りとして使う
log_step() {
	printf '\n%s=== %s ===\n' "$(_log_prefix)" "$1"
}

# 通常の進捗表示
log_info() {
	printf '%s%s\n' "$(_log_prefix)" "$1"
}

# 異常系は必ずstderrへ出す。呼び出し元での分岐を邪魔しないため
log_warn() {
	printf '%s%s警告: %s%s\n' "$(_log_prefix)" "$_LOG_YELLOW" "$1" "$_LOG_RESET" >&2
}

log_error() {
	printf '%s%sエラー: %s%s\n' "$(_log_prefix)" "$_LOG_RED" "$1" "$_LOG_RESET" >&2
}

# エラーを出して終了する。終了コードは省略時3（実行時エラー、ADR-0122）
die() {
	log_error "$1"
	exit "${2:-3}"
}

# --- 実験ログの置き場（ADR-0149） -------------------------------------
#
# ログは data/logs/<名前>.log に置く。リダイレクト先を呼び出しのたびに
# 決めていた結果、data/ 直下に21本が規則なくたまった（2026-08-08）。
#
# 追記で開く。停止と再開（ADR-0123）で同じ実験を複数回に分けて走らせる
# ため、上書きすると前半が消える。
log_path() {
	local name="$1"
	[[ -n "$name" ]] || die "ログ名が空"
	mkdir -p "${REPO_ROOT}/data/logs"
	printf '%s/data/logs/%s.log' "$REPO_ROOT" "$name"
}

# 標準出力と標準エラーをログへ流しつつ端末にも出す。
# 使い方: run_logged <名前> <コマンド> [引数...]
run_logged() {
	local name="$1"
	shift
	local path
	path="$(log_path "$name")"
	log_info "ログ: ${path}"
	{
		printf '\n=== %s ===\n' "$(date '+%Y-%m-%d %H:%M:%S')"
		printf 'cmd: %s\n' "$*"
	} >>"$path"
	# パイプの途中で失敗しても呼び出し元へ伝える
	set -o pipefail
	"$@" 2>&1 | tee -a "$path"
}

# --- 共通の前提チェック -------------------------------------------------

require_file() {
	local path="$1" desc="${2:-ファイル}"
	if [[ ! -f "$path" ]]; then
		die "${desc}がない: ${path}" 3
	fi
}

require_executable() {
	local path="$1"
	if [[ ! -x "$path" ]]; then
		die "実行できない: ${path}" 3
	fi
}

require_command() {
	local cmd="$1"
	if ! command -v "$cmd" >/dev/null 2>&1; then
		die "${cmd} コマンドが要る" 3
	fi
}

# --- GitHub Releaseの共通処理（release-book.sh / release-net.sh） -------
#
# 骨格だけが共通で、ノートの中身（定跡は局面数、ネットはlineage）は
# スクリプトごとに違う。無理に1本へまとめず、ここでは骨格だけを持つ
# （docs/adr/0122-tooling-language-split.md）。

# 引数列から --apply を抜き取る。あれば RELEASE_APPLY=1 を立て、
# 残りを配列 RELEASE_ARGS へ入れる。呼び出し側はこう受ける。
#
#   release_take_apply "$@"
#   set -- ${RELEASE_ARGS[@]+"${RELEASE_ARGS[@]}"}
#
# 配列を戻り値で返せないのでグローバルを使う。macOSの /bin/bash は
# 3.2 で、mapfile も連想配列もない。CIはLinux（bash 5）なので、
# 4以降の機能を使うとCIだけ通ってローカルで落ちる
release_take_apply() {
	RELEASE_APPLY="${RELEASE_APPLY:-0}"
	RELEASE_ARGS=()
	local arg
	for arg in "$@"; do
		if [[ "$arg" == "--apply" ]]; then
			RELEASE_APPLY=1
		else
			RELEASE_ARGS+=("$arg")
		fi
	done
	export RELEASE_APPLY
}

release_validate_version() {
	local version="$1"
	if ! [[ "$version" =~ ^[0-9]+$ ]]; then
		die "バージョン番号は1以上の整数で指定する: ${version}" 3
	fi
}

# ghの存在とタグの重複を確認する。どちらかが不成立なら終了する
release_check_prereqs() {
	local tag="$1"
	require_command gh
	if gh release view "$tag" >/dev/null 2>&1; then
		die "${tag} は既にある。番号を上げる" 3
	fi
}

release_file_size() {
	du -h "$1" | cut -f1
}

# タグ・タイトル・ノートファイル・アセット（複数可）からリリースを作る。
#
# 既定では作らない。走るはずのコマンドとノート本文を出して終わる。
# 実際に作るには --apply を渡すか RELEASE_APPLY=1 を立てる。
#
# リリースの作成は外から見える操作で、消しても「あった」ことは残る。
# 2026-08-01に、動作確認のつもりで book-v99999 を本当に作ってしまった
# （直後に削除）。**予行演習を既定にすれば、思い出さなくても事故が起きない。**
# 忘れて困るのは「作ったつもりが作られていない」ときだけで、そちらは
# 出力を見れば分かる。
release_create() {
	local tag="$1" title="$2" notes_file="$3"
	shift 3
	if [[ "${RELEASE_APPLY:-0}" != "1" ]]; then
		log_warn "予行演習のため作成しない。実行するには --apply を付ける"
		log_info "gh release create ${tag} $* --title ${title} --notes-file ${notes_file} --latest=false"
		log_info "--- ノート本文 ---"
		cat "$notes_file"
		return 0
	fi
	gh release create "$tag" "$@" \
		--title "$title" \
		--notes-file "$notes_file" \
		--latest=false
	log_info "作成した: $(gh release view "$tag" --json url --jq .url)"
}
