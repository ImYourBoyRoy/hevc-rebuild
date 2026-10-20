#!/usr/bin/env bash
set -euo pipefail

# Tauri's Linux bundler names the AppImage desktop entry from productName.
# GTK application IDs, however, are matched by desktop-file basename.  This
# normalizes the completed AppDir before the AppImage and updater archive are
# signed, while preserving the human-facing product name in the entry.

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
bundle_dir=${1:-"$repo_root/desktop/src-tauri/target/release/bundle/appimage"}
config_path="$repo_root/desktop/src-tauri/tauri.conf.json"

if [ ! -d "$bundle_dir" ]; then
    echo "AppImage bundle directory does not exist: $bundle_dir" >&2
    exit 1
fi

readarray -t config_values < <(node - "$config_path" <<'NODE'
const fs = require("node:fs");
const config = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
console.log(config.productName);
console.log(config.identifier);
console.log(config.version);
NODE
)
product_name=${config_values[0]}
identifier=${config_values[1]}
version=${config_values[2]}
desktop_name="$identifier.desktop"

appdir="$bundle_dir/$product_name.AppDir"
if [ ! -d "$appdir" ]; then
    echo "Tauri AppDir does not exist: $appdir" >&2
    exit 1
fi

legacy_desktop="$appdir/$product_name.desktop"
legacy_usr_desktop="$appdir/usr/share/applications/$product_name.desktop"
canonical_desktop="$appdir/$desktop_name"
canonical_usr_desktop="$appdir/usr/share/applications/$desktop_name"

if [ ! -f "$legacy_usr_desktop" ]; then
    echo "Tauri desktop entry does not exist: $legacy_usr_desktop" >&2
    exit 1
fi

mkdir -p "$appdir/.legacy-desktop-entry"
if [ -e "$legacy_desktop" ] || [ -L "$legacy_desktop" ]; then
    mv "$legacy_desktop" "$appdir/.legacy-desktop-entry/root.desktop"
fi
mv "$legacy_usr_desktop" "$appdir/.legacy-desktop-entry/usr.desktop"

cp "$appdir/.legacy-desktop-entry/usr.desktop" "$canonical_usr_desktop"

# The root desktop file and icon must have the same basename for AppImage
# desktop integration.  The icon referenced by the desktop entry remains the
# generated hicolor icon named after the executable.
if [ -e "$canonical_desktop" ] || [ -L "$canonical_desktop" ]; then
    rm -f "$canonical_desktop"
fi
ln -s "usr/share/applications/$desktop_name" "$canonical_desktop"

legacy_icon="$appdir/$product_name.png"
canonical_icon="$appdir/$identifier.png"
if [ -f "$legacy_icon" ]; then
    mv "$legacy_icon" "$canonical_icon"
elif [ ! -f "$canonical_icon" ]; then
    echo "Tauri AppImage root icon does not exist: $legacy_icon" >&2
    exit 1
fi

if [ -e "$appdir/.DirIcon" ] || [ -L "$appdir/.DirIcon" ]; then
    rm -f "$appdir/.DirIcon"
fi
ln -s "$identifier.png" "$appdir/.DirIcon"

cache_dir=${XDG_CACHE_HOME:-${HOME:?}/.cache}/tauri
appimage_plugin=$(find "$cache_dir" -maxdepth 1 -type f -name 'linuxdeploy-plugin-appimage*.AppImage' -print -quit)
if [ -z "$appimage_plugin" ]; then
    echo "Tauri's linuxdeploy AppImage plugin was not found in $cache_dir" >&2
    exit 1
fi

appimage=$(find "$bundle_dir" -maxdepth 1 -type f -name '*.AppImage' -print -quit)
if [ -z "$appimage" ]; then
    echo "Tauri AppImage output does not exist in $bundle_dir" >&2
    exit 1
fi

LDAI_OUTPUT="$appimage" ARCH=x86_64 "$appimage_plugin" --appimage-extract-and-run --appdir "$appdir"

# Tauri's updater consumes the compressed AppImage, not the standalone file.
# Rebuild the archive after the desktop identity patch.  Its existing .sig is
# replaced below when an updater private key is available.
updater_archive="$appimage.tar.gz"
if [ -f "$updater_archive" ]; then
    archive_tmp="$updater_archive.rebuilt"
    tar -czf "$archive_tmp" -C "$(dirname "$appimage")" "$(basename "$appimage")"
    mv "$archive_tmp" "$updater_archive"

    if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
        signer=("$repo_root/desktop/node_modules/.bin/tauri" signer sign "$updater_archive" --app-version "$version")
        "${signer[@]}"
    fi
fi

rm -rf "$appdir/.legacy-desktop-entry"
echo "Normalized Linux AppImage desktop identity: $desktop_name"
