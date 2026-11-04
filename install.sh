#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
VERSION="$(cat "$HERE/VERSION" 2>/dev/null || echo unknown)"
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
DEST="$BIN_DIR/hevc-rebuild-studio"

usage() {
  cat <<'USAGE'
Usage: ./install.sh [--assume-yes]

Builds the native Rust/Tauri application from this checkout and installs the
launcher under ~/.local/bin (or XDG_BIN_HOME). The flag is accepted for
compatibility with older source installers; this installer does not install
system packages or modify APT state.
USAGE
}

for arg in "$@"; do
  case "$arg" in
    --assume-yes) ;;
    --help|-h) usage; exit 0 ;;
    *) echo "ERROR: unknown option: $arg" >&2; usage >&2; exit 2 ;;
  esac
done

echo "Building HEVC Rebuild Studio v$VERSION from native Rust/Tauri sources..."

if [[ "${OSTYPE:-}" != linux* ]]; then
  echo "ERROR: install.sh is for Linux source installs. Use the Tauri package for Windows or macOS." >&2
  exit 1
fi

for command_name in node pnpm cargo; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "ERROR: '$command_name' is required to build the native application." >&2
    echo "Install the pinned Node/pnpm toolchain and Rust/Tauri Linux prerequisites; see docs/TOOLS.md." >&2
    exit 1
  fi
done

if [[ -r /etc/os-release ]]; then
  # shellcheck disable=SC1091
  . /etc/os-release
  echo "Detected ${PRETTY_NAME:-${ID:-Linux}}."
fi

pnpm --dir "$HERE/desktop" install --frozen-lockfile
pnpm --dir "$HERE/desktop" tauri build --no-bundle --config src-tauri/tauri.conf.json

SOURCE_BINARY="$HERE/desktop/src-tauri/target/release/hevc-rebuild-studio"
if [[ ! -x "$SOURCE_BINARY" ]]; then
  echo "ERROR: the native release binary was not produced at $SOURCE_BINARY" >&2
  exit 1
fi

install -D -m 0755 "$SOURCE_BINARY" "$DEST"
"$DEST" json environment >/dev/null

echo
echo "============================================================"
echo "Native HEVC Rebuild Studio installation is complete."
echo "============================================================"
echo
echo "Launcher:"
echo "  $DEST"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "NOTE: $BIN_DIR is not currently on PATH. Add it to your shell PATH, or run $DEST directly." ;;
esac
echo
echo "The installed executable provides the GUI, CLI, JSON actions, and MCP:"
echo "  $DEST"
echo "  $DEST json environment"
echo "  $DEST --mcp"
echo
echo "External media tools are installed or selected separately; see docs/TOOLS.md."
