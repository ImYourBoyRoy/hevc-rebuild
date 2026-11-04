# Review and release evidence

Last reviewed: 2026-11-04.

This is the current evidence summary for the native Rust/Tauri product. A
passing source check is not a claim that a packaged application was installed
or that a native window was walked through.

## Current source evidence

The following checks have passed in the current worktree:

- Rust library tests: 266 passed, 0 failed, 0 ignored after the current
  agent-control/version maintenance pass;
- strict locked Clippy with warnings denied;
- native Rust binary build;
- Svelte check and TypeScript-native check;
- production frontend build;
- focused GUI/browser contracts for YouTube, support, library, Run, Home/Help,
  and responsive layout; the isolated browser harness uses free ports and
  project-local ignored evidence/cache paths;
- native Tauri dev smoke through the opt-in control channel: ready state,
  bounded snapshot, YouTube navigation, and Help navigation on
  `http://127.0.0.1:1420`;
- dynamic work-directory resolution through JSON CLI, including source-adjacent
  and environment-override paths;
- architecture generation/check and checksum verification in this pass; and
- real local HDR evidence for static HDR10, HDR10+ helper verification, and
  explicit Dolby Vision HDR10 fallback.

The current pass regenerates Cargo and pnpm locks after the direct dependency
refresh, adds the opt-in native walkthrough control channel, and reruns the
source, contract, generated-artifact, and checksum gates after documentation
and harness changes.

## Native behavior boundary

Still requiring direct evidence:

- clean Linux source installation using install.sh;
- clean installation and launch of the published Windows NSIS, macOS DMG, and
  Linux AppImage/deb packages;
- Linux AppImage rebuilt on the supported glibc baseline;
- native Settings/work-root discovery walkthrough;
- file and folder pickers;
- OS drag/drop;
- queue add/remove/reorder and report association;
- close/reopen recovery;
- stop-now during a live encode;
- screen-reader navigation and announcements;
- installed CLI and MCP startup on each supported host;
- real media on representative SDR, static HDR10, HDR10+, and Dolby Vision
  fallback inputs; and
- signed updater installation and rollback behavior.

The embedded agent-control channel supports keyboard navigation and bounded text
snapshots, but it does not make OS dialogs, drag/drop, or screen-reader output
synthetic. The remaining native walkthrough must use the launched Tauri window.

## Static interaction checks

The generated interaction map and waterfall are useful source inventories.
Their selector resolution is currently clean: both checks report zero warnings.
They still do not replace the separate real-Tauri journey acceptance below.

## Release gates

The first public release's CI package and updater-manifest evidence is not
equivalent to clean-install acceptance. Before the next release, attach:

1. host, glibc, package filename, install command, launch result, and uninstall
   result for Linux;
2. the same evidence for Windows and macOS;
3. a real-window walkthrough log using the embedded control channel;
4. representative media validation reports for every HDR policy; and
5. a final checksum and release-manifest check.

## Reproduction commands

~~~bash
pnpm --dir desktop install --frozen-lockfile
pnpm --dir desktop check
pnpm --dir desktop check:tsgo
pnpm --dir desktop build
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked --lib
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets --locked -- -D warnings
cargo build --manifest-path desktop/src-tauri/Cargo.toml --locked --bin hevc-rebuild-studio
sha256sum --check --quiet SHA256SUMS
node tools/tasklist-check.mjs
~~~

No release or publish action is authorized by this document.
