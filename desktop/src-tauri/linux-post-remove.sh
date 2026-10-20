#!/bin/sh
set -eu

desktop_dir=/usr/share/applications
rm -f \
    "$desktop_dir/HEVC Rebuild Studio.desktop" \
    "$desktop_dir/org.hevcrebuild.desktop.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$desktop_dir" >/dev/null 2>&1 || true
fi
