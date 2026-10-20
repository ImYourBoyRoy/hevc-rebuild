#!/bin/sh
set -eu

desktop_dir=/usr/share/applications
legacy="$desktop_dir/HEVC Rebuild Studio.desktop"
canonical="$desktop_dir/org.hevcrebuild.desktop.desktop"

# Tauri uses the product name for the generated filename.  Move that entry to
# the identifier-derived filename so GTK/Wayland can associate the running
# window with the installed launcher.
if [ -f "$legacy" ]; then
    install -m 0644 "$legacy" "$canonical"
    rm -f "$legacy"
fi

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$desktop_dir" >/dev/null 2>&1 || true
fi
