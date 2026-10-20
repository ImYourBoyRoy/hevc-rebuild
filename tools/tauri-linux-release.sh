#!/usr/bin/env bash
set -euo pipefail

if [ "${1:-}" != "build" ]; then
    echo "This wrapper is only intended for a Tauri build in the release workflow." >&2
    exit 2
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

# The action invokes this file from desktop/.  Run the real CLI first so all
# Tauri signing and target-specific build arguments remain unchanged.
pnpm exec tauri "$@"

"$repo_root/tools/normalize-linux-appimage.sh" \
    "$repo_root/desktop/src-tauri/target/release/bundle/appimage"
