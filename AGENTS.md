# Project Agent Instructions

## Project boundary

This extracted project contains the HEVC Rebuild Python CLI/engine and an Ubuntu-first Tauri/Svelte desktop preview. The source archive is kept one directory above this project and is the untouched reference. Media libraries and files are user data; do not scan or process them during build checks.

## Authoritative behavior

- `hevc_rebuild/config.py` owns application defaults and config precedence. Do not duplicate default values in the frontend; load effective config through `hevc_rebuild.bridge`. Saved GUI rebuild values live in the native `settings.json` profile layer shared by the GUI, CLI, and MCP.
- `hevc_rebuild/app.py` and the rest of `hevc_rebuild/` own media discovery, encoding, validation, transactions, logs, and reports.
- `desktop/src-tauri/src/lib.rs` exposes narrow native commands. Keep argument vectors fixed/validated; do not add a general shell command API or broad shell-plugin capability.
- `desktop/src-tauri/src/mcp_server.rs` exposes fixed, typed MCP tools over local stdio. Keep tools scoped to engine/bridge operations; never expose arbitrary commands or unrestricted filesystem access. Long scans/runs must be tracked jobs with bounded output.
- The executable's `cli` mode forwards the established Python engine CLI. The `json` mode accepts only the Python bridge's defined actions; preserve machine-readable stdout and progress on stderr.
- Full rebuild must require an explicit in-app confirmation. Dependency repairs must stay opt-in. Safe stop must finish the current file before stopping.
- MCP short tests and full rebuilds require explicit confirmation inputs. MCP APT repair must validate the exact missing package list against the engine and the Rust allowlist. HDR10+ helper setup remains separate and explicit.
- GPU detection is informational. The current encoder is CPU libx265.
- `hevc_rebuild.encode.choose_resource_plan()` applies a profile-aware memory scheduling heuristic using current available/effective total memory, limits, memory PSI, video profile, and preset. It is not an exact peak-RSS predictor; keep the live RAM/swap-pressure guards and quality-neutral retry. Report swap occupancy and activity, but do not count it as fast RAM or resize/configure it automatically. Swap rates and PSI are system-wide signals, not encoder-attributed RSS.

`hevc_rebuild.calibration` provides short null-output x265 layout comparisons for demanding files. It may adjust only worker/frame counts, must stay within the freshly computed plan, and must not weaken live pressure guards. Calibration must remain controllable, visible in progress/reports, skip when memory telemetry is missing or below reserve, and never write media output.

## Toolchain and dependency versions

- Root `.nvmrc` selects the recommended Node LTS build; `desktop/package.json` records its supported Node ranges and pins the project package manager through `packageManager`.
- Keep npm dependencies on tested caret ranges and commit `pnpm-lock.yaml` so installs stay reproducible. Do not use a bare `>=` or wildcard for libraries when it can admit a breaking major; use a tested lower bound plus a compatible-major range.
- Keep Rust dependency requirements as full three-component caret floors and commit `Cargo.lock`. Update versions from current stable releases after checking upstream compatibility and running the desktop checks.
- The Python engine currently has no third-party Python package dependencies. FFmpeg, FFprobe, MediaInfo, and MKVToolNix are inspected/managed as explicit host tools; do not add Python packages without an architectural need.
- Prefer supported stable/LTS toolchains over release-candidate/nightly versions. Keep the normal Svelte checker on the compiler version it supports; test experimental compiler paths separately before adopting them.

## Platform and data safety

- Current supported engine platforms are Ubuntu 24.04+ and Ubuntu-based Linux Mint 22+. Ubuntu `.deb` and x86-64 AppImage targets are configured. The current AppImage bundles libraries requiring glibc 2.43 (built on Ubuntu 26.04.1); rebuild on the oldest supported Ubuntu baseline before claiming compatibility there. Windows/macOS support is not implemented.
- Environment checks and folder analysis must remain read-only. The Python bridge uses `load_config(..., create_user=False)` for UI reads. A run may retain the CLI's existing config-creation behavior.
- AppImage portable state belongs in the writable sibling `<AppImage filename>.data/` directory. If it is unavailable, report the fallback to the user's XDG config location. Never try to write preferences inside the mounted AppImage or beside the system-installed `/usr/bin` executable.
- APT installs may use only fixed package names from the engine's map and Rust-side allowlist, after the UI displays packages and gets confirmation. Keep media-tool repair separate from HDR10+ helper installation.
- Never install packages, run media rebuilds, or alter files under a user's media folder as part of source/build validation.
- Do not change or regenerate the original ZIPs. Keep build artifacts/dependencies under `desktop/`.
- Hardware calibration may read a short section from an input media file, but must send output only to FFmpeg's null muxer; never benchmark by creating a media file in a user's library.

## Documentation and validation

- Keep `README.md`, `MEMORY.md`, `TECHNICAL.md`, and `REVIEW.md` current when behavior or platform support changes.
- Keep `VERSION`, `hevc_rebuild.__version__`, `desktop/package.json`, `desktop/src-tauri/Cargo.toml`, and Tauri's version aligned.
- After source/document changes, regenerate `SHA256SUMS` from the tracked project files, excluding that checksum file, generated output, dependency caches, and build targets.
- Desktop checks: from `desktop/`, use `pnpm check`, `pnpm build`, and `pnpm tauri build`; use `cargo check` from `desktop/src-tauri/` for Rust bridge checks. Inspect both the `.deb` and AppImage outputs when a release build is reported. Do not call a frontend/package build evidence of native UI or cross-platform support.
- Use the committed lockfiles for normal builds (`pnpm install --frozen-lockfile`, `cargo check --locked`).
- Existing engine tests live under `tests/`; inspect applicable tests when changing engine behavior. Do not claim test results unless run in this task.
- Before reporting a release build, inspect its actual package metadata and bundled resources. Distinguish frontend/Rust compilation from a packaged runtime launch.
