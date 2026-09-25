#!/usr/bin/env bash
set -euo pipefail
rm -f "$HOME/.local/bin/hevc-rebuild"
rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/hevc-rebuild/app"
echo "Removed the application launcher and application files. Logs and managed hdr10plus_tool under ~/.local/share/hevc-rebuild are intentionally retained."
