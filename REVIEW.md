# HEVC Rebuild v1.3.0 - Review / Release Notes

## Local Git and dependency policy

- Initialized a local `main` repository with project-scoped author identity, LF text normalization, and no remote. The root `.gitignore` keeps generated artifacts, local state, and the external source ZIP out of source tracking.
- Updated frontend minimums to the current locked compatible releases, widened TypeScript within supported major 6, recommended Node 24 LTS, declared the supported Node 22.12/24/26 lines, and pinned pnpm 12.6.0. Cargo manifests now use three-part caret floors matching the current stable direct dependency graph; both lockfiles remain authoritative for exact builds.
- TypeScript 7.0.2 is available, but Svelte's normal `svelte-check` path still expects TypeScript 5/6. Its TypeScript 7 path needs separate compiler wiring, so this app does not adopt it in the default build.
- Version basis checked 2026-09-25 against npm/crates.io and official Node, Vite, Tauri, Svelte, and Cargo guidance. Validation is recorded below; no remote was configured and nothing was pushed.
- Validation: `pnpm install --frozen-lockfile`, `pnpm check` (0 errors/warnings), `pnpm build`, and `cargo check --locked` passed. Cargo advanced `smallvec` from 1.16.1 to 1.16.2. `pnpm outdated` found TypeScript 7.0.2 as the sole newer package; it remains outside the default toolchain until the Svelte checker supports a straightforward TS7 path.

## v1.2.6 calibration integration and GitHub source hygiene

- Integrated the archive's short x265 layout calibration into the current memory-aware engine, CLI defaults/options, desktop settings, run reports, and typed MCP settings. Candidate layouts stay within the fresh resource plan; null output, reserve checks, swap/PSI rejection, and live encode guards stay active.
- Default `auto` mode benchmarks eligible uncached 4K/large files; `ask` prompts only in an interactive CLI when uncached; `always` benchmarks every processed file when automatic threading is active and bypasses cache; `off` disables calibration.
- Verified the archive SHA-256 (`5449d33da96b444533ea3eb1ae9c50c9198d2f8faa5ad3af1a6f91e4b1d3d3cb`) and its included `SHA256SUMS`; archive files were not changed. The root `.gitignore` now covers Python, Tauri/Svelte/Rust build output, local state/logs/reports, package artifacts, and root-level source ZIPs. `SHA256SUMS` excludes ignored Tauri-generated schemas.
- Validation: Python compilation; 7 calibration tests; 13 config/content tests; `pnpm check` (0 errors/warnings); `pnpm build`; `cargo check`; and `sha256sum --check SHA256SUMS` passed. No media was scanned or encoded, and no package was built or installed in this work cycle.
- A representative authorized 4K measurement, native GUI interaction, live MCP handshake, and packaged runtime rebuild remain unverified. The local repository has no remote and has not been pushed. A public release still needs repository visibility and license decisions.

## Swap-pressure telemetry and retry guard

- Added current swap used GiB/percentage, FFmpeg process RSS/`VmSwap`, and Linux `/proc/vmstat` swap-in/out rates to resource snapshots, CLI live status, environment JSON, desktop hardware/scan views, per-file run CSV, and report UI.
- Active encode monitoring now retries the temporary output with lower x265 parallelism after at least five consecutive progress samples with swap-in at or above 4 MiB/s and memory PSI `full avg10` at or above 1%. Swap occupancy alone never triggers a retry.
- Rates and PSI are system-wide signals; they can reflect other processes. They are not attributed to FFmpeg's RSS.
- Automatic swap creation/resizing remains out of scope. An 8/16 GiB recommendation can be an optional system-level advisor after measured calibration, never a per-file guarantee. Short null-output x265 layout calibration is now implemented; it measures candidate layouts and memory pressure, stays within the current forecast cap, and leaves the live retry guards active. Representative authorized 4K measurements are still needed to validate the forecast against observed workloads.
- Validation: `python3 -m compileall -q hevc_rebuild`, `pnpm check` (0 errors/warnings), `pnpm build`, and `cargo check --manifest-path src-tauri/Cargo.toml` passed. No engine tests, media scan/encode, native launch, MCP handshake, or package rebuild was run.

## 2026-09-24 profile-aware memory forecast

- Changed the initial x265 plan to use current available RAM as well as effective total RAM, cgroup-v2 allowance, memory PSI, source dimensions, target bit depth, chroma, and preset lookahead. Auto mode reduces worker/frame concurrency before encoding; `unlimited` skips the reserved-CPU rule but retains the memory cap.
- Added per-file forecast levels and planned thread counts to desktop folder analysis, CLI `--info-only`, JSON scan results, CSV run reports, and MCP folder-analysis pages.
- Kept the existing live RAM guard and quality-neutral temporary-output retry. The forecast is an explicit profile-scaled heuristic, not exact peak RSS; x265 builds and media content vary. Current swap is reported and not counted as fast-memory headroom.
- Swap resizing/configuration was deliberately not added. A future swap planner needs measured workload data and an explicit, reversible host-level action with disk and authorization checks; the current model cannot give a defensible exact per-file swap requirement.
- Validation: `python3 -m compileall -q hevc_rebuild`, `pnpm check` (0 errors/warnings), `pnpm build`, and `cargo check` passed. No engine tests, actual media scan/encode, native launch, MCP handshake, or package rebuild was performed. A real authorized 4K short test is still required to calibrate the heuristic against observed peak RSS.

## Portable Ubuntu outputs and saved settings

- Added an x86-64 AppImage alongside the existing Ubuntu `.deb` installer. The AppImage stores application settings in a writable sibling `<AppImage filename>.data/` folder; if that folder is unavailable, it reports the fallback to the user's XDG config folder. The `.deb` writes to `$XDG_CONFIG_HOME/hevc-rebuild-studio/settings.json` instead of trying to write beside `/usr/bin`.
- Persisted theme, selected media folder, FFmpeg override, scan options, stop-on-error, and an optional rebuild profile in native JSON. The profile is shared with CLI/MCP and layers after user TOML but before a library-specific override. Legacy browser preferences migrate when no native settings file exists.
- Built on Ubuntu 26.04.1: `.deb` installer is 2,594,682 bytes; AppImage is 82,708,984 bytes. `dpkg-deb` verified package metadata, runtime dependencies, executable, icons, and bundled Python modules. `file` and `unsquashfs` verified the AppImage executable and engine resources.
- Static ABI inspection found the app executable requires `GLIBC_2.39`, while bundled AppImage GTK/WebKit libraries require up to `GLIBC_2.43`. The current AppImage therefore does not support Ubuntu 24.04; rebuild from a 24.04 baseline before claiming that target. The `.deb` was not installed or launched.
- `pnpm check`, `pnpm build`, `cargo check`, and final `pnpm tauri build` passed. No engine tests or media operations were run. Native theme/preferences interaction, MCP handshake, and cross-release AppImage launch remain unverified.

## 1.3.0 Ubuntu desktop preview

### Appearance and automation additions

- Added persisted Light, Dark, and Hacker themes to the app-wide top-bar selector.
- Added `hevc-rebuild-studio cli` to forward the existing full engine CLI, plus a `json` mode for machine-readable environment, defaults, scan, reports, and helper-install bridge actions.
- Added an official Rust MCP SDK server using local stdio; the GUI executable also runs as the MCP subprocess without opening a network port.
- Added ten typed MCP tools for environment/settings, asynchronous folder analysis with progress and paginated per-file errors, background run jobs, safe stop, reports, bounded diagnostic logs, exact-list APT repair, and explicit HDR helper installation.
- MCP check-only is the default. Short tests/full rebuilds require explicit confirmation; package installs require the exact missing packages reported by the environment check.
- MCP jobs are tracked and bounded, limit simultaneous scans, serialize read-only analysis against writes, truncate oversized output lines, and preserve safe stop-after-current-file behavior.
- CLI/MCP share engine defaults, the saved application profile, and the existing Rust settings validation. No test suite was run for these additions.

- Validation for these additions: `pnpm check` (0 errors/warnings), `pnpm build`, `cargo check`, and `pnpm tauri build` passed. `dpkg-deb -I/-c` confirmed amd64 package 1.3.0, executable `/usr/bin/hevc-rebuild-studio`, and bundled engine at `/usr/lib/HEVC Rebuild Studio/engine/hevc_rebuild/`. The packaged binary's top-level `--help` displayed the GUI, CLI, JSON, and MCP modes. A real MCP client handshake, installed-package launch, and native theme interaction were not performed. No test suite, host repair, media scan, or rebuild was run.

- Added an Ubuntu-first Tauri 2 + Svelte 5 desktop shell while keeping the Python engine authoritative.
- Added read-only system/hardware/toolchain inspection; folder inventory with per-file stream/HDR profiles, recursive filtering, minimum size, and scan error visibility.
- Added the effective engine configuration as the GUI's initial values, plus run-only controls and an explicit save-as-app-defaults action for all current config settings.
- Added Check only, fixed five-second CRF 30 test samples, and a separately confirmed Full rebuild mode.
- Added safe stop-after-current-file, explicit allowlisted APT repair, selected FFmpeg path, explicit HDR10+ helper setup, report history, and bounded local log viewer.
- Added typed Rust process commands; the webview does not have general shell access. Runs cannot auto-repair dependencies.
- Config/environment reads do not create the engine user TOML; app preferences write only after a user setting change or initial preference migration.
- Ubuntu `.deb` and x86-64 AppImage are configured. Windows and macOS remain unsupported by the Python engine.
- Validation in the current workspace: `pnpm check` (0 errors, 0 warnings), `pnpm build`, `cargo check`, Python bytecode compilation, and `pnpm tauri build` passed.
- The amd64 `.deb` was inspected with `dpkg-deb`: package `hevc-rebuild-studio` version 1.3.0 declares `python3`, `libwebkit2gtk-4.1-0`, and `libgtk-3-0`, and includes the app executable, desktop entry, icons, and Python engine modules. It was not installed or launched from the package.
- The native Tauri bridge compiled. A native desktop interaction/capture has not been verified; a plain-browser preview is intentionally disconnected from system operations.
- Read-only inspection of this Ubuntu 26.04 build host found FFmpeg, FFprobe, Fontconfig, libx265, APT/dpkg, and `pkexec`; MediaInfo and MKVToolNix were absent, so the environment was not ready for a rebuild. No package repair was run, no media folder was scanned, and no user config was created.
- No engine test suite was run in this work cycle. Windows/macOS builds and runtime behavior remain unsupported and unverified.

## Maintenance fixes from v1.2.0

- Fixed misleading HDR10+ probe diagnostics: an expected FFmpeg `Broken pipe` caused by `hdr10plus_tool` closing its input early is now classified as pipeline shutdown noise rather than a media failure.
- Removed a Dolby Vision false-positive risk: generic `RPU` text alone no longer marks a stream as Dolby Vision.
- Added explicit Dolby Vision evidence reporting. FFprobe `DOVI configuration record` is identified separately from the more limited MediaInfo fallback.
- Added a Guided Mode choice between conservative Dolby Vision handling and verified HDR10 fallback. Safe mode remains the default.
- Tightened HDR10 fallback eligibility: Profile 7/8 only, FFprobe-verified base layer, and independent HDR10 identification are required.
- Fixed an important fallback edge case on modern FFmpeg/libx265: fallback now forces `-dolbyvision:v:0 0` when the encoder option exists, preventing the option's default `auto` mode from accidentally re-emitting Dolby Vision RPU data.
- Added output validation for HDR10 fallback: output must remain HDR10 and must no longer report Dolby Vision.
- Added Dolby Vision profile/evidence/action columns to the CSV run report.
- Updated skip diagnostics to tell the operator when verified HDR10 fallback is available and exactly how to select it.

## Safety posture

The default `dolby_vision = "safe"` behavior is unchanged: unsupported Dolby Vision is skipped rather than silently reduced. `hdr10-fallback` is an explicit choice because it intentionally discards Dolby Vision metadata/enhancement information. The original source is still not moved until the rebuilt output passes validation.

## Validation status

The v1.2.2 suite covers the previous v1.2.0 invariants plus:

- FFprobe DOVI evidence-source parsing;
- rejection of generic `RPU` text as sufficient Dolby Vision evidence;
- Profile-7 HDR10 fallback eligibility checks;
- expected HDR10+ producer broken-pipe classification;
- explicit `-dolbyvision 1` for preservation and `-dolbyvision 0` for fallback on capable FFmpeg builds.

The final ZIP is acceptance-tested again after clean extraction.


## 1.2.2 resource hardening

A real 4K 10-bit `slow` test on Linux Mint exited with return code `-9` (SIGKILL) after x265 auto-created a 16-thread worker pool and 4 frame threads. FFmpeg/x265 itself emitted no codec error. This is consistent with an external Linux kill, most commonly memory pressure/OOM.

v1.2.2 adds:
- Linux `MemAvailable`-aware x265 worker/frame-thread caps;
- conservative 4K laptop defaults;
- configurable `[resources]` controls;
- a free-RAM preflight threshold;
- SIGKILL-specific diagnostics and best-effort kernel OOM evidence;
- non-destructive migration of older config files to include the new resource section.


## 1.2.3 adaptive resources + terminal UI

The 1.2.1 Keller failure proves an external SIGKILL, not an HDR encoder failure. HDR10/Main10 initialization completed normally and x265 emitted no codec error before termination. OOM remains a strong possibility, but without kernel OOM evidence it cannot be stated as certain.

v1.2.3 changes the resource strategy again:
- `auto` starts at all logical CPU threads minus one;
- automatic frame parallelism uses two frame encoders for normal HD/4K work;
- effective RAM includes Linux `MemAvailable` and finite cgroup-v2 memory allowance;
- RAM is monitored during the encode;
- sustained pressure stops only the temporary output and retries with reduced parallelism;
- quality/HDR/bit-depth settings are never reduced as part of a memory retry;
- SIGKILL is called OOM only when kernel evidence supports that diagnosis;
- terminal output is now a compact media/settings card plus a two-line in-place progress/resource HUD;
- raw x265 diagnostics remain available in the log.


## 1.2.4 full resource/storage hardening

- Sequential processing remains mandatory: exactly one media file is encoded at a time.
- CPU auto policy is now explicit/configurable with `reserved_cpu_threads = 1`; the default therefore uses all logical CPUs except one.
- Added live aggregate CPU utilization from `/proc/stat`.
- Added Linux PSI diagnostics for CPU, memory, and I/O pressure.
- Dynamic RAM reserve is now `max(min_free_memory_gib, physical_RAM * memory_reserve_percent)`, defaulting to max(3 GiB, 8%).
- Existing adaptive memory retries remain quality-neutral.
- Added disk preflight with a hard safe-start floor and a separate conservative CRF output budget.
- Added live disk free-space guarding and projected remaining-output checks; temporary encoding stops before consuming the configured free-space reserve.
- Explicitly accounts for the fact that retaining originals under `old/` increases disk usage until backups are removed.
- Added optional best-effort temperature display from Linux thermal/hwmon sensors.
- Added batch/run summary UI, per-file queue position, queue progress, CPU/RAM/disk live status, and quiet non-TTY progress milestones.
- Normal console logging is quieter; raw x265/FFmpeg diagnostics remain in the file log.
- Every failed file receives a structured `FAILURE SUMMARY` with phase, exception and resource snapshot.
- Existing config files are migrated non-destructively to add new resource/storage keys without overwriting user-selected values.
