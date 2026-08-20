#!/usr/bin/env bash
# 開発環境の構築（Linux / WSL2）。
#
# 新しいマシンでこれを1本流せば、ビルド・SPRT・学習が動く状態になる。
# 教師データと学習済みネットは別途 hmwr data fetch と
# gh release download で取る（README.md 参照）。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./env.sh
source "${SCRIPT_DIR}/env.sh"

SKIP_PYTHON="${SKIP_PYTHON:-0}"

usage() {
	cat <<'USAGE'
使い方:
  scripts/setup.sh

開発環境を構築する（Linux / WSL2向け）。APTパッケージ・gh CLI・Rust・
Python仮想環境を入れ、ビルドとテストまで通す。

環境変数:
  SKIP_PYTHON=1  Python / PyTorch のセットアップを飛ばす
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
	usage
	exit 0
fi

if [[ "$(uname -s)" != "Linux" ]]; then
	die "このスクリプトはLinux（WSL2を含む）向け。macOSではbrewで個別に入れる"
fi

log_step "APTパッケージ"
sudo apt-get update -qq
sudo apt-get install -y --no-install-recommends \
	build-essential curl git pkg-config libssl-dev ca-certificates

log_step "gh CLI"
if ! command -v gh >/dev/null 2>&1; then
	curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg |
		sudo dd of=/usr/share/keyrings/githubcli-archive-keyring.gpg
	sudo chmod go+r /usr/share/keyrings/githubcli-archive-keyring.gpg
	echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" |
		sudo tee /etc/apt/sources.list.d/github-cli.list >/dev/null
	sudo apt-get update -qq
	sudo apt-get install -y gh
else
	log_info "導入済み: $(gh --version | head -1)"
fi

log_step "Rust"
if ! command -v rustup >/dev/null 2>&1; then
	curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y --no-modify-path
	# shellcheck disable=SC1091
	source "$HOME/.cargo/env"
else
	log_info "導入済み: $(rustup --version)"
fi
# rust-toolchain.toml のチャネルとコンポーネントが自動で入る
(cd "$REPO_ROOT" && rustup show active-toolchain)

if [[ "$SKIP_PYTHON" != "1" ]]; then
	log_step "Python と PyTorch"
	sudo apt-get install -y --no-install-recommends python3 python3-pip python3-venv
	if [[ ! -d "${REPO_ROOT}/.venv" ]]; then
		python3 -m venv "${REPO_ROOT}/.venv"
	fi
	# shellcheck disable=SC1091
	source "${REPO_ROOT}/.venv/bin/activate"
	pip install --quiet --upgrade pip
	pip install --quiet -r "${REPO_ROOT}/training/requirements.txt"
	log_info "torch $(python3 -c 'import torch; print(torch.__version__)')"
	python3 -c 'import torch; print("CUDA:", torch.cuda.is_available())'
	deactivate
fi

log_step "ビルド"
(cd "$REPO_ROOT" && RUSTFLAGS="$RUSTFLAGS_NATIVE" cargo build --release)

log_step "テスト"
(cd "$REPO_ROOT" && cargo test --release --quiet 2>&1 | tail -5)

log_step "完了"
cat <<NEXT
次の手順:

1. hmwr コマンドへパスを通す（シェルの設定ファイルへ書く）
     export PATH="${REPO_ROOT}/bin:\$PATH"
   通したら hmwr env で設定を確認する
2. gh auth login          （GitHubの認証。gh release download に要る）
3. 学習済みネットの取得
     gh release list | grep net-v
     gh release download net-v<N> -D data/nets/
4. 教師データの取得（学習を回す場合のみ。約160GBの空きが要る）
     hmwr data fetch all

詳細は README.md を参照。
NEXT
