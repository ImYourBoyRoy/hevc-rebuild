#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
VERSION="$(cat "$HERE/VERSION" 2>/dev/null || echo unknown)"
echo "Installing HEVC Rebuild v$VERSION..."
DEST="${XDG_DATA_HOME:-$HOME/.local/share}/hevc-rebuild/app"
BIN_DIR="$HOME/.local/bin"
ASSUME_YES=0
for arg in "$@"; do
  [[ "$arg" == "--assume-yes" ]] && ASSUME_YES=1
done

prompt_install() {
  local message="$1"
  if (( ASSUME_YES )); then
    return 0
  fi
  if [[ ! -t 0 ]]; then
    echo "ERROR: Interactive installation is required: $message" >&2
    echo "Re-run from a terminal or pass --assume-yes." >&2
    exit 1
  fi
  while true; do
    read -r -p "$message [ENTER=continue, Q=quit] " answer
    case "${answer,,}" in
      "") return 0 ;;
      q) echo "Installation cancelled."; exit 130 ;;
      *) echo "Press ENTER to continue or Q to quit." ;;
    esac
  done
}

sudo_cmd() {
  if (( EUID == 0 )); then
    "$@"
  else
    command -v sudo >/dev/null 2>&1 || { echo "ERROR: sudo is required for package installation." >&2; exit 1; }
    sudo "$@"
  fi
}

if [[ -r /etc/os-release ]]; then
  # shellcheck disable=SC1091
  . /etc/os-release
  case "${ID:-}" in
    ubuntu)
      echo "Detected Ubuntu ${VERSION_ID:-unknown}."
      ;;
    linuxmint)
      if [[ " ${ID_LIKE:-} " == *" ubuntu "* ]]; then
        echo "Detected Ubuntu-based Linux Mint ${VERSION_ID:-unknown}."
      else
        echo "WARNING: This Linux Mint edition does not identify as Ubuntu-based; the application will perform a stricter compatibility check." >&2
      fi
      ;;
    *)
      echo "WARNING: This bundle targets Ubuntu and Ubuntu-based Linux Mint. Detected ID=${ID:-unknown}." >&2
      ;;
  esac
fi

if ! command -v python3 >/dev/null 2>&1; then
  prompt_install "Python 3 is missing. Press ENTER to install it before continuing."
  sudo_cmd apt-get update
  sudo_cmd apt-get install -y python3
fi
command -v python3 >/dev/null 2>&1 || { echo "ERROR: Python 3 is still unavailable after installation." >&2; exit 1; }

mkdir -p "$DEST" "$BIN_DIR"
rm -rf "$DEST/hevc_rebuild" "$DEST/assets"
cp -a "$HERE/hevc_rebuild" "$DEST/"
cp -a "$HERE/hevc_rebuild.py" "$DEST/"

cat > "$BIN_DIR/hevc-rebuild" <<EOF
#!/usr/bin/env bash
exec python3 "$DEST/hevc_rebuild.py" "\$@"
EOF
chmod 0755 "$BIN_DIR/hevc-rebuild"

echo "Installed launcher: $BIN_DIR/hevc-rebuild"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "NOTE: $BIN_DIR is not currently on PATH. Add it to your shell PATH, or run $BIN_DIR/hevc-rebuild directly." ;;
esac

"$BIN_DIR/hevc-rebuild" --setup-only "$@"

echo
echo "============================================================"
echo "HEVC Rebuild installation for Linux Mint / Ubuntu is complete."
echo "============================================================"
echo
echo "To start the guided program, run:"
echo "  $BIN_DIR/hevc-rebuild"
echo
echo "The safest first choice in Guided Mode is: 1) Check only"
echo
echo "Settings are stored here:"
echo "  ${XDG_CONFIG_HOME:-$HOME/.config}/hevc-rebuild/config.toml"
echo "Open them later with:"
echo "  xdg-open ${XDG_CONFIG_HOME:-$HOME/.config}/hevc-rebuild/config.toml"
echo
