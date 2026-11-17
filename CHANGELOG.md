# Changelog

All notable changes to HEVC Rebuild Studio. The newest version is first. Version numbers are chosen at
release time; everything under "Unreleased" has been committed since `v1.4.1` and is not yet in a published
package.

## Unreleased (since v1.4.1)

### Added
- **Headless server mode.** `hevc-rebuild-studio serve` runs the app without a desktop and shows the same GUI in a
  web browser, on the same jobs. Token sign-in (a login link, or a bearer token), `--read-only`, `--root` to keep the
  folder chooser and media paths inside chosen folders, a server-side file and folder chooser, live events, and the
  browser's right-click menu. Listening beyond the local computer needs `--allow-remote` and `--allowed-host`, and
  HTTPS comes from a reverse proxy. See `docs/maintainer/SERVE_MODE.md`.
- **MCP over HTTP.** A running server offers the whole MCP tool set at `/mcp` with the same token. A job an agent starts
  appears live in every open browser tab, and only one run, scan or setup job is active at a time across both.
- **Self-describing MCP.** New tools `explain_settings` (what CRF, presets, tune, HDR and the other settings mean, with a
  rough size guide) and `app_guide` (workflow, run modes, safety rules, headless notes), and fuller server instructions.
  They read the same file as the app's Help, so they cannot disagree with it.
- **A build with no GUI libraries.** `cargo build --no-default-features` builds the server, CLI and MCP with no window and
  no webkit or GTK. `tools/package-headless.sh` makes a tarball that finds its GUI files by itself.
- **Setup without a desktop.** The setup plan reports a privilege route (`root`, `sudo-n`, or none with the exact fix); the
  API never accepts a password. An elevated server on Windows counts as root.
- **One-pass tool setup.** One administrator prompt installs missing tools and updates packages, then the HDR10+ helper;
  updates are checked at launch and every six hours, and password-free updates can run automatically (off by default).
- **Unattended library batches** over MCP (`plan_library_batch`, `start_planned_batch`) with a storage forecast and a
  protected free-space floor; a picture quality check (SSIM) before any original is replaced; originals kept in a hidden
  `.hevc-rebuild-originals` folder.
- **Eight themes.** Latest-outcome-only failure lists.
- **Plain-language settings help** beside CRF, presets, tune, 10-bit, HDR10+, Dolby Vision, tracks, copy verification, the
  quality check and the YouTube options, a rough CRF size guide, and Help sections for headless use and safety.

### Changed
- The short test now encodes at the run's CRF (21 by default) instead of a fixed 30, so it previews the real result.
- The default CRF is 21 everywhere (the engine and planner had drifted to 22).
- The build is smaller: the library crate is `rlib` only and dev builds keep line tables only. A clean debug build went
  from 38 GB to 3.5 GB.

### Fixed
- Windows: the default media folder could not be found (the app read only `HOME`), which broke the GUI's first call.
- Windows: folder paths in the chooser showed a `\\?\` prefix.
- The stdio MCP tools `plan_system_setup` and `provision_system` crashed (a blocking network call on the async runtime).
- Help popovers inside settings rows were permanently open, and annotated controls had a cluttered accessible name.
- macOS and Windows build and test fixes found by running the full suite natively on both.

## 1.4.1
Published release: the Rust-only migration, per-source work-directory discovery, native HDR10+ validation, YouTube
preparation updates, cross-platform stop-now reporting and responsive layout fixes. See `docs/RELEASE.md`.
