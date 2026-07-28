#!/usr/bin/env bash
# 開発環境の構築（Linux / WSL2）。
#
# 新しいマシンでこれを1本流せば、ビルド・SPRT・学習が動く状態になる。
# 教師データと学習済みネットは別途 fetch-dataset.sh と
# gh release download で取る（SETUP.md 参照）。
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SKIP_PYTHON="${SKIP_PYTHON:-0}"

log() { printf '\n=== %s ===\n' "$1"; }

if [[ "$(uname -s)" != "Linux" ]]; then
	echo "このスクリプトはLinux（WSL2を含む）向け。macOSではbrewで個別に入れる" >&2
	exit 1
fi

log "APTパッケージ"
sudo apt-get update -qq
sudo apt-get install -y --no-install-recommends \
	build-essential curl git pkg-config libssl-dev ca-certificates

log "gh CLI"
if ! command -v gh >/dev/null 2>&1; then
	curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg |
		sudo dd of=/usr/share/keyrings/githubcli-archive-keyring.gpg
	sudo chmod go+r /usr/share/keyrings/githubcli-archive-keyring.gpg
	echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" |
		sudo tee /etc/apt/sources.list.d/github-cli.list >/dev/null
	sudo apt-get update -qq
	sudo apt-get install -y gh
else
	echo "導入済み: $(gh --version | head -1)"
fi

log "Rust"
if ! command -v rustup >/dev/null 2>&1; then
	curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y --no-modify-path
	# shellcheck disable=SC1091
	source "$HOME/.cargo/env"
else
	echo "導入済み: $(rustup --version)"
fi
# rust-toolchain.toml のチャネルとコンポーネントが自動で入る
(cd "$REPO_ROOT" && rustup show active-toolchain)

if [[ "$SKIP_PYTHON" != "1" ]]; then
	log "Python と PyTorch"
	sudo apt-get install -y --no-install-recommends python3 python3-pip python3-venv
	if [[ ! -d "${REPO_ROOT}/.venv" ]]; then
		python3 -m venv "${REPO_ROOT}/.venv"
	fi
	# shellcheck disable=SC1091
	source "${REPO_ROOT}/.venv/bin/activate"
	pip install --quiet --upgrade pip
	pip install --quiet -r "${REPO_ROOT}/training/requirements.txt"
	echo "torch $(python3 -c 'import torch; print(torch.__version__)')"
	python3 -c 'import torch; print("CUDA:", torch.cuda.is_available())'
	deactivate
fi

log "ビルド"
(cd "$REPO_ROOT" && RUSTFLAGS="-C target-cpu=native" cargo build --release)

log "テスト"
(cd "$REPO_ROOT" && cargo test --release --quiet 2>&1 | tail -5)

log "完了"
cat <<'NEXT'
次の手順:

1. gh auth login          （GitHubの認証。gh release download に要る）
2. 学習済みネットの取得
     gh release list | grep net-v
     gh release download net-v<N> -D data/nets/
3. 教師データの取得（学習を回す場合のみ。約160GBの空きが要る）
     scripts/fetch-dataset.sh all
4. マシンに合わせた設定
     scripts/env.sh を編集する（SPRTの並列度など）

詳細は docs/SETUP.md を参照。
NEXT
