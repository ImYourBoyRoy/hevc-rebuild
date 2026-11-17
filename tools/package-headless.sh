#!/bin/sh
# ./tools/package-headless.sh
# Build the headless server (no webkit/GTK) and assemble an install tree the server finds by itself:
#   hevc-rebuild-studio-headless/bin/hevc-rebuild-studio
#   hevc-rebuild-studio-headless/share/hevc-rebuild-studio/web/   (the built GUI)
# then write hevc-rebuild-studio-headless.tar.gz next to it. Run from the repository root.
#
# Usage: tools/package-headless.sh [--debug] [--no-frontend-build] [output-dir]
#   --debug              use a debug build (fast; the default is --release)
#   --no-frontend-build  reuse desktop/build as it is
set -eu

PROFILE=release; BUILD_FRONTEND=1; OUT=dist
for arg in "$@"; do
  case "$arg" in
    --debug) PROFILE=debug ;;
    --no-frontend-build) BUILD_FRONTEND=0 ;;
    *) OUT=$arg ;;
  esac
done

[ "$BUILD_FRONTEND" = 1 ] && pnpm --dir desktop build
[ -f desktop/build/index.html ] || { echo "desktop/build/index.html is missing; build the frontend first" >&2; exit 1; }

if [ "$PROFILE" = release ]; then
  cargo build --manifest-path desktop/src-tauri/Cargo.toml --locked --release --no-default-features --target-dir desktop/src-tauri/target/headless --bin hevc-rebuild-studio
else
  cargo build --manifest-path desktop/src-tauri/Cargo.toml --locked --no-default-features --target-dir desktop/src-tauri/target/headless --bin hevc-rebuild-studio
fi
BIN=desktop/src-tauri/target/headless/$PROFILE/hevc-rebuild-studio

TREE=$OUT/hevc-rebuild-studio-headless
rm -rf "$TREE"
mkdir -p "$TREE/bin" "$TREE/share/hevc-rebuild-studio"
cp "$BIN" "$TREE/bin/hevc-rebuild-studio"
cp -r desktop/build "$TREE/share/hevc-rebuild-studio/web"
tar -C "$OUT" -czf "$OUT/hevc-rebuild-studio-headless.tar.gz" hevc-rebuild-studio-headless
echo "Wrote $TREE and $OUT/hevc-rebuild-studio-headless.tar.gz"
echo "Run: $TREE/bin/hevc-rebuild-studio serve"
