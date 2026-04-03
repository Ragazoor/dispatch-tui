#!/usr/bin/env bash
set -euo pipefail

REPO="Ragazoor/dispatch-agent"
BINARY="dispatch"
INSTALL_DIR="${HOME}/.local/bin"

# ── Platform check ────────────────────────────────────────────────────────────

OS="$(uname -s)"
ARCH="$(uname -m)"

case "${OS}-${ARCH}" in
  Linux-x86_64)   TARGET="x86_64-unknown-linux-gnu" ;;
  Darwin-arm64)   TARGET="aarch64-apple-darwin" ;;
  *)
    echo "error: no pre-built binary for ${OS}/${ARCH}" >&2
    echo "       Build from source: cargo install dispatch-agent" >&2
    exit 1
    ;;
esac

# ── Resolve version ───────────────────────────────────────────────────────────

if [[ -n "${VERSION:-}" ]]; then
    TAG="${VERSION}"
else
    echo "Fetching latest release..."
    TAG="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep '"tag_name"' \
        | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')"
    if [[ -z "${TAG}" ]]; then
        echo "error: could not determine latest release. Set VERSION= to install a specific version." >&2
        exit 1
    fi
fi

echo "Installing ${BINARY} ${TAG}..."

# ── Download ──────────────────────────────────────────────────────────────────

ARTIFACT="${BINARY}-${TARGET}"
URL="https://github.com/${REPO}/releases/download/${TAG}/${ARTIFACT}"
TMP="$(mktemp)"
trap 'rm -f "${TMP}"' EXIT

echo "Downloading ${URL}..."
curl -fsSL --progress-bar -o "${TMP}" "${URL}"

# ── Install ───────────────────────────────────────────────────────────────────

mkdir -p "${INSTALL_DIR}"
install -m 755 "${TMP}" "${INSTALL_DIR}/${BINARY}"
echo "Installed to ${INSTALL_DIR}/${BINARY}"

# Warn if ~/.local/bin is not in PATH
if [[ ":${PATH}:" != *":${INSTALL_DIR}:"* ]]; then
    echo ""
    echo "  Note: ${INSTALL_DIR} is not in your PATH."
    echo "  Add this to your shell profile:"
    echo "    export PATH=\"\${HOME}/.local/bin:\${PATH}\""
fi

# ── Configure Claude Code ─────────────────────────────────────────────────────

echo ""
echo "Configuring Claude Code..."
"${INSTALL_DIR}/${BINARY}" setup

# ── Prerequisites checklist ───────────────────────────────────────────────────

echo ""
echo "Prerequisites checklist:"

check_dep() {
    local cmd="$1"
    local note="$2"
    if command -v "${cmd}" &>/dev/null; then
        echo "  [x] ${cmd}"
    else
        echo "  [ ] ${cmd}  ← ${note}"
    fi
}

check_dep tmux   "required — dispatch must run inside a tmux session"
check_dep git    "required — dispatch creates git worktrees for agents"
check_dep claude "required — Claude Code CLI (https://claude.ai/code)"
check_dep gh     "optional — needed for the Review Board (gh auth login)"

echo ""
echo "Done. Run 'dispatch tui' inside a tmux session to start."
