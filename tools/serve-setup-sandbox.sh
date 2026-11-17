#!/bin/sh
# ./tools/serve-setup-sandbox.sh
# Real privileged one-pass setup through `hevc-rebuild-studio serve`, without touching the host.
#
# Runs the server as a namespace root (bubblewrap, uid 0) with PRIVATE COPIES of the package-manager
# state (/var/lib/apt, /var/lib/dpkg, /etc/apt), then asks it for a setup plan and runs
# `provision_toolchain` over HTTP: a real `apt-get update` as root, streamed back as repair events.
# The host's apt state is fingerprinted before and after and the script fails if it changed.
#
# Needs: bwrap (bubblewrap), curl, python3, network access to the Ubuntu mirrors.
# Usage: tools/serve-setup-sandbox.sh [path/to/hevc-rebuild-studio] [path/to/desktop/build] [port]
# Evidence recorded in docs/maintainer/SERVE_MODE.md. This does NOT exercise the `sudo -n` route
# (the sandbox user is root); that route is covered by the command-builder tests.
set -eu

BIN=${1:-desktop/src-tauri/target/debug/hevc-rebuild-studio}
WEB=${2:-desktop/build}
PORT=${3:-8437}
TOKEN=sandbox-token-0123456789abcdef
BASE=http://127.0.0.1:$PORT
WORK=$(mktemp -d "${TMPDIR:-/tmp}/hevc-setup-sandbox.XXXXXX")
trap 'kill "$SERVER" 2>/dev/null || true; rm -rf "$WORK"' EXIT

command -v bwrap >/dev/null || { echo "bubblewrap (bwrap) is required" >&2; exit 2; }
BIN=$(readlink -f "$BIN"); WEB=$(readlink -f "$WEB")

fingerprint() {
  { find /var/lib/apt/lists -maxdepth 1 -type f -printf '%p %s %T@\n'
    stat -c '%n %s %Y' /var/lib/dpkg/status
    find /etc/apt -type f -printf '%p %s %T@\n'; } 2>/dev/null | sort | md5sum
}
BEFORE=$(fingerprint)

R=$WORK/root
mkdir -p "$R/var/lib" "$R/var/cache/apt" "$R/var/log/apt" "$R/etc" "$WORK/state"
cp -r --no-preserve=ownership /var/lib/apt "$R/var/lib/apt" 2>/dev/null || true
cp -r --no-preserve=ownership /var/lib/dpkg "$R/var/lib/dpkg" 2>/dev/null || true
cp -r --no-preserve=ownership /etc/apt "$R/etc/apt" 2>/dev/null || true
mkdir -p "$R/var/lib/apt/lists/partial"; touch "$R/var/lib/apt/lists/lock"; chmod -R u+rwX "$R"
# A one-uid namespace cannot drop to the _apt user; the private copy of apt's config says not to.
echo 'APT::Sandbox::User "root";' > "$R/etc/apt/apt.conf.d/99-no-privilege-drop"
# Make the catalog stale (the copy has fresh mtimes) so the plan has a privileged step.
find "$R/var/lib/apt/lists" "$R/var/cache" -exec touch -h -d '40 days ago' {} + 2>/dev/null || true

HEVC_REBUILD_PORTABLE_DATA_DIR=$WORK/state HEVC_REBUILD_SERVE_TOKEN=$TOKEN \
bwrap --die-with-parent --unshare-user --uid 0 --gid 0 --unshare-pid --dev-bind / / \
  --bind "$R/var/lib/apt" /var/lib/apt --bind "$R/var/lib/dpkg" /var/lib/dpkg \
  --bind "$R/etc/apt" /etc/apt --bind "$R/var/cache/apt" /var/cache/apt --bind "$R/var/log/apt" /var/log/apt \
  --proc /proc --dev /dev "$BIN" serve --web-root "$WEB" --port "$PORT" >"$WORK/serve.log" 2>&1 &
SERVER=$!
for _ in 1 2 3 4 5 6 7 8 9 10; do curl -sf -H "Authorization: Bearer $TOKEN" "$BASE/api/info" >/dev/null && break; sleep 1; done

call() { curl -s -X POST -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d "$2" "$BASE/api/invoke/$1"; }
echo "== plan"; PLAN=$(call provision_plan '{"toolPaths":{}}'); echo "$PLAN" | python3 -c 'import sys,json; d=json.load(sys.stdin); print({k:d[k] for k in ("elevation","password_prompts","refresh_catalog","install_packages","upgrade_packages","steps","blockers")})'
echo "$PLAN" | python3 -c 'import sys,json; d=json.load(sys.stdin); sys.exit(0 if d["elevation"]=="root" and d["password_prompts"]==0 else 1)' || { echo "FAIL: expected the root route" >&2; exit 1; }

curl -sN -H "Authorization: Bearer $TOKEN" "$BASE/api/events" >"$WORK/events.out" 2>&1 &
EVENTS=$!
sleep 1
echo "== provision_toolchain"; call provision_toolchain '{"toolPaths":{}}'; echo
for _ in $(seq 1 60); do grep -q repair-finished "$WORK/events.out" && break; sleep 3; done
kill "$EVENTS" 2>/dev/null || true
grep '^data' "$WORK/events.out" | tail -4 | cut -c1-220
grep -q '"success":true' "$WORK/events.out" || { echo "FAIL: the setup job did not succeed" >&2; exit 1; }

echo "== plan afterwards"; call provision_plan '{"toolPaths":{}}' | python3 -c 'import sys,json; d=json.load(sys.stdin); print({k:d[k] for k in ("refresh_catalog","steps","nothing_to_do")})'
AFTER=$(fingerprint)
[ "$BEFORE" = "$AFTER" ] || { echo "FAIL: the host's apt state changed" >&2; exit 1; }
echo "OK: privileged setup ran for real as namespace root; host apt state unchanged"
