#!/usr/bin/env bash
set -euo pipefail

BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
removed=0
for launcher in "$BIN_DIR/hevc-rebuild-studio" "$BIN_DIR/hevc-rebuild"; do
  if [[ -e "$launcher" || -L "$launcher" ]]; then
    rm -f -- "$launcher"
    echo "Removed launcher: $launcher"
    removed=1
  fi
done

if (( ! removed )); then
  echo "No HEVC Rebuild Studio launcher was found under $BIN_DIR."
fi

echo "Application settings, reports, logs, credentials, and managed HDR10+ helpers were retained."
