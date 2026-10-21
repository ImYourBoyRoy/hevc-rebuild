# HEVC Rebuild v1.3.0 - Review / Release Notes

## Current worktree validation — 2026-10-19

This section is the current source/documentation status for the worktree. The
dated sections below are historical evidence and retain their original claims.

### Completed in this worktree

- Media Library uses the native Tauri drag lifecycle to show a visible drop
  target and active state, then adds dropped files or folders to the same
  persistent queue used by the native pickers.
- Identify & organize consumes normalized persisted queue suggestions and
  local poster previews from the current Media Library scan. Its movie match
  cards are compact, review states are explicit, and candidate lists remain
  collapsed until requested.
- The Help surface no longer grows its cards beyond a narrow viewport. Its
  implicit grid tracks and safety-note flex text are constrained, while code
  examples retain their intended local horizontal scrolling.
- The project map is documented in
  [`docs/PROJECT-STRUCTURE.md`](../PROJECT-STRUCTURE.md), and the root and
  desktop README entry points link to it.
- The cross-platform contract is now enforced in the source adapters: Windows
  and macOS host checks, native profile/cache/log paths, installed bundled
  engine discovery, native folder opening, host-specific HDR10+ archive
  selection, and Linux-only APT setup are separated from shared GUI/CLI/MCP
  workflows. The contract is documented in
  [`docs/PLATFORM-SUPPORT.md`](../PLATFORM-SUPPORT.md).
- The final source snapshot was built natively through the sibling
  `Remote_Build` macOS guest and the documented Server_Manager Windows guest.
  Windows produced an NSIS installer with no MSI; macOS produced a CI-safe DMG
  and `.app`.

### Checks run

- `pnpm check` — passed with 0 errors and 0 warnings.
- `pnpm check:tsgo` — passed with 0 errors and 0 warnings.
- `pnpm test:metadata-matching` — 9/9 passed.
- `pnpm test:home-help`, `test:library`, `test:rename`, `test:run`,
  `test:youtube`, and `test:support` — passed.
- `python3 -m unittest discover -s tests -q` — 204 tests passed, 33 skipped.
- `cargo fmt --all -- --check` and `cargo metadata --offline --locked
  --no-deps --format-version 1` — passed. Full dependency metadata was not
  available offline because `jiff-static` was not cached.
- Isolated Help layout probe — 30 combinations across 360, 390, 620, 800,
  1024, and 1440px and Light, Dark, Phosphor, Ion, and Breach; document and
  main horizontal overflow were false. Help code blocks still scroll locally
  when their example text is wider than the viewport.
- `git diff --check` and `sha256sum -c --quiet SHA256SUMS` — passed after the
  last checksum regeneration.
- Windows native browser contracts — metadata 9/9, modal, home/help, library,
  rename, run, support, performance, workspace-state, YouTube, and responsive
  300 viewport/theme checks plus the zoom proxy — passed. The responsive pass
  found and verified the narrow Help note overflow fix.
- Windows `pnpm check` — 0 errors and 0 warnings; final native `pnpm build` and
  `pnpm tauri build` — passed. The packaged executable's `--help`, JSON
  environment, and empty synthetic scan exited 0; the NSIS bundle was
  5,055,715 bytes and no MSI was produced.
- macOS `pnpm install --frozen-lockfile`, both Svelte checker lanes, `pnpm
  build`, and final CI-mode `pnpm tauri build` — passed on macOS 26.7.1 Intel.
  The `.app` CLI smoke and read-only mounted DMG passed; the guest had no media
  tools installed, so full media readiness was not established.

### Still not established

No clean installer installation, native OS drag/drop walkthrough, real-media
run, provider/account upload or playback test, screen-reader workflow proof,
Apple Silicon build, signing/notarization, GitHub release, or live updater
installation was performed in this pass. The checked-in source still uses the
documented hybrid Rust/Python runtime; Python is not embedded away yet. Ruff
was not available on PATH. The current AppImage artifact was built on a glibc
2.43 host and must not be presented as Ubuntu 24.04-compatible.

**Current release decision: not ready for publication.** Native Windows x64
and macOS Intel build/package/CLI-smoke gates are now green, but clean install,
native GUI, Apple Silicon, accessibility, real-media, signing/notarization,
GitHub release, and live updater boundaries remain release gates.

## Historical maintenance evidence — 2026-10-12

The following record retains its original date and counts; it is not a current
readiness claim. Its detailed task/evidence ledger remains in the local notes
archive and is intentionally excluded from the public source tree.

Safety/type/UI ownership tasks through T-065 and measured performance T-070–T-073 are implemented and verified in slices. Python has 187 passing configured-native tests before the final policy additions; Rust has 200 passing tests plus one explicit synthetic benchmark excluded from the default suite. Both TS6/TS7 lanes pass. Cross-path fixtures measure settings, profiles, reports, transactions, cancellation and recovery; Python orchestration and all documented fallbacks remain supported. Six shared-modal surfaces and feature callbacks pass isolated real-browser contracts. These are component/native-helper evidence, not a populated native walkthrough.

Frontend dependency batch passes twelve checks, with +80 JS bytes (+142 gzip bytes). The actual debug native app launched with an isolated profile and exposed Home/Library through Linux AT-SPI. Rust dependency checks and baseline package preparation are ongoing; the source, native workflow, assistive-technology, media, package and platform acceptance gates remain separate.

`PERFORMANCE.md`, `DEPENDENCIES.md`, `RUST_MIGRATION.md`, `desktop/UI_OWNERSHIP.md` and `desktop/CSS_CONTRACT.md` define measured performance, controlled updates, owner/caller contracts and UI/CSS boundaries. Current warnings include the large production route chunk, host color-environment notices and classified Clippy advisories. A historical HDR helper ETXTBSY test race passed isolated/full rechecks and stays recorded. No commit, publication, user-media operation, account upload, or host package mutation was performed.

The final source gate records 192 configured-native Python tests and 201 Rust tests, with both TypeScript lanes, frontend build, focused lint and diff checks passing. The Ubuntu 24.04.5/glibc 2.39 acceptance bundle passed packaged `.deb`/AppImage smoke and WebDriver safety-dialog focus return. Synthetic HDR10+ and Dolby Vision fixtures prove detection, safe skip, and explicit HDR10 fallback; this host's x265 lacks HDR10+ dynamic-metadata encoding and Dolby Vision full encode requires VBV, so those paths remain fail-closed or explicit fallback. Screen-reader workflow announcements beyond startup/frame announcements and Windows/macOS runtime proof remain open.

**Release decision: not ready for publication.** The source gate and Ubuntu 24.04.5 package smoke are green, but the documented screen-reader blocker, unsupported Windows/macOS targets, absent account upload/playback evidence, and host HDR10+/Dolby Vision encoder limitations remain open P0 acceptance limits. No commit, upload, or publication was performed.


## 2026-09-28 Conflicting local movie NFO handling

- Local asset inspection now identifies conflicting scalar fields inside one NFO and across same-kind sidecars, names the affected fields/files, and applies neither record. Comparisons normalize whitespace/case, genre ordering, and `year`/`premiered`/`aired` aliases; layered TV-show and episode documents stay distinct. The workbench's existing NFO error surface provides the review state and prevents silent metadata assignment.
- Validation: all 175 Rust library tests pass, including conflicts in one NFO, conflicts across sidecars, equivalent normalized data, and unchanged TV show/episode layering; `cargo fmt --all -- --check` and `cargo check --locked --bin hevc-rebuild-studio` pass. Four pre-existing dead-code warnings remain. This does not replace native queue/provider acceptance; no media was scanned.

## 2026-09-28 Artwork cache controls and MCP schema

- Settings report saved-poster cache count and size, distinguish queue-assigned from unused art, and offer a reviewed purge. The backend serializes cache operations, checks a fresh preview ID, protects queued poster paths, removes only recognized app-managed images and their attribution sidecars, and leaves unknown entries and x265 calibration data untouched.
- The MCP server previously panicked during startup because `cache_local_library_artwork` declared a bare string input. It now accepts an object with `source_path`; startup and tool schemas are verified by a real stdio handshake.
- Validation: 172 Rust library tests passed; Rust formatting, binary check/build, both Svelte checker lanes, frontend build, CLI cache inspection, and an MCP `2025-06-18` handshake/tool listing (32 tools) passed. Four existing Rust dead-code warnings remain. Native Settings, populated cache/purge, and poster-output acceptance remain open; no media or cache items were deleted.

## 2026-09-28 Imported cleanup and artwork settings

- Editable text-blacklist patterns clean imported title clues before public searches, as well as proposed renames and readable output metadata. The supplied RenameTheThings archive contains removable filename/folder exclusions but no separate editable word-blacklist list, so the example-based `Movie*Group` rule remains the starter text pattern. Legacy extra, hidden-name, sample, and trailer scan exclusions are seeded in Settings and can be removed individually or cleared; app-owned state folders remain excluded for safety.
- Poster storage and MKV embedding are independent Settings choices: save a confirmed poster beside media using the media stem plus `-poster`, use the private app artwork cache, embed the chosen art in metadata-only/rebuilt MKVs, or enable both.
- Validation: all 170 Rust library tests passed; Rust format and binary checks passed (binary check reports four dead-code warnings); seven Python artwork tests passed; both Svelte checker lanes reported zero errors and warnings. These source checks do not confirm native settings reload, provider image downloads, or actual output-MKV attachment.

## 2026-09-28 TV NFO association guard

- The local `tvshow.nfo` remains the primary series record. If readable episode NFOs disagree with its show title, the workbench reports affected filenames and blocks local episode metadata assignment. When no show NFO exists, conflicting episode `<showtitle>` values or unreadable episode NFOs also block choosing the first available local record; the user can review the NFOs or explicitly compare public sources.
- Validation: both Svelte checker lanes reported zero diagnostics and the production frontend build passed. This confirms static/type/build validity; no native Tauri metadata review or user-library scan was performed.

## 2026-09-28 Rename companion sidecars

- Rust rename previews now include matching same-base NFOs, recognized local artwork names (basename, poster, cover, fanart, and thumb), artwork attribution, common external subtitles, and external audio. Suffixes are preserved, and all companions use the same source recheck, collision checks, and staged no-overwrite transaction as their video. The test confirms a lowercase-basename subtitle and user artwork follow their video while a similarly prefixed unrelated filename stays in place.
- The organizer test scratch paths now include a per-process sequence as well as a timestamp so parallel tests cannot share a temporary root.
- Validation: all 169 Rust library tests passed, including the alternate-artwork, attribution, subtitle, and audio rename cases; Rust format check and binary check passed; both Svelte checker lanes reported zero diagnostics; and the production frontend build passed. This verifies source/package build behavior, not native Tauri dialog or crash-recovery behavior. No library media, provider request, credentials, or package-manager operation was used.

## 2026-09-28 TV organizer preflight review

- Rust organizer scans now return bounded, per-season examples of possible episode-number gaps in observed `SxxEyy` filenames. The UI calls them clues rather than proof of missing media and skips this heuristic for seasons with a multi-episode filename.
- A filename/provider year mismatch remains visible in the TV detail review and blocks rename preview until the user explicitly acknowledges it. Full-series renames continue to fail closed for truncated/unreadable scans and unselected supported episodes.
- Validation: `pnpm ui:check`, `pnpm ui:check:tsgo` (zero diagnostics), `pnpm ui:build`, `cargo fmt --all -- --check`, all 169 Rust library tests, `cargo check --locked --bin hevc-rebuild-studio`, and `git diff --check` passed. The browser preview remains disconnected from Tauri-native picker actions; no native UI acceptance is claimed.

## 2026-09-28 MCP per-file queue result synchronization

- Single-root MCP `start_media_run` now marks matching saved-queue paths as running and synchronizes final per-file states from the exact CSV report generated by the job. Recognized successful report statuses may become complete; absent rows stay `needs_review`. The compact queue inspector reads an unmodified snapshot so a poll during a live job does not report it as interrupted. Report reads are confined to regular CSV files in the application log directory and capped at 64 MiB.
- Synthetic tests cover exact queue-path matching, preserving global selection, live `running` visibility, report-row mapping, missing-row review, and report-path confinement. Validation: `cargo fmt --all -- --check`, all 156 Rust library tests, and `cargo check --locked --bin hevc-rebuild-studio` passed.
- This change is not yet verified in a fresh packaged MCP run or native Tauri window. Multi-source CLI/MCP execution, CLI result sync, native queue lifecycle, and real report association remain open. No user media, provider credentials, or package-manager operations were used.

## 2026-09-28 Metadata-only validation and multi-source GUI queue

- Completed and documented metadata-only MKV transaction validation using a generated fixture with two video streams (one HDR10), audio, subtitles, chapters, a preserved non-cover attachment, replacement poster art, reviewed tags, and an unchanged source hash. The selected Matroska cover is validated separately when FFprobe exposes it as an attached-picture video stream.
- The GUI now schedules selected files across ready sources, or all files from ready queue scans when nothing is selected. It processes sources sequentially using each source's saved recursive/minimum-size/settings snapshot, deduplicates canonical paths, and carries safe-stop/stop-on-error across the source sequence. Run setup shows an expandable source and path plan, limited to 250 rendered paths for large libraries.
- Fresh Rust MCP stdio smoke negotiated protocol 2025-06-18 and listed 24 tools; `start_media_run` includes metadata-only mode plus reviewed metadata and poster override schemas. Frontend `pnpm check`, `pnpm check:tsgo`, and `pnpm build` pass with zero Svelte/TypeScript diagnostics.
- Native Tauri verification is still open. The available `127.0.0.1:1420` page is a browser preview with file/folder actions disabled, and no native app window is exposed. Durable per-file run recovery, whole-queue output collision preflight, native close/reopen/run validation, and multi-source CLI/MCP queue APIs remain open. No user media, package transaction, or credential was accessed.

## 2026-09-27 Accessible browser, CLI, and MCP operational checks

- Used accessible browser controls to open Settings and the TMDB setup action. The running page is the browser preview rather than a Tauri shell: native file dialogs, environment commands, and credential saves are disabled there, so this does not verify native-window accessibility or OS keyring behavior.
- Built the Rust executable and verified CLI JSON environment/settings, public movie search, and read-only JSON scan. The CLI scanned 30 sample-library files with zero per-file errors. A trailing-year movie query initially returned no results; Rust CLI/MCP now retry Wikidata without a final year only when the exact query has no candidates. The same query returns three suggestions with the 2011 film ranked first.
- MCP initialized at protocol 2025-06-18 with 24 tools. Read-only environment/settings calls, film and series search/details, and asynchronous folder analysis completed. Film details included one artwork candidate; TVmaze returned six seasons and 64 episodes for the selected test series. MCP analysis scanned 30 sample files with zero per-file errors.
- Host environment reports full rebuild and YouTube readiness false: FFmpeg, FFprobe, Python, and libx265 are present; MediaInfo and MKVToolNix are missing, and `hdr10plus_tool` is not detected. No package repair or credential access was performed. FFprobe-only scans succeed, while the legacy `can_analyze` flag remains tied to FFprobe plus MediaInfo; the Environment panel and Help now explain that scans remain available with partial details while processing jobs stay blocked on their required tools.
- Validation: `cargo fmt --all`, `cargo test --locked --lib` (127 passed), `cargo build --locked --bin hevc-rebuild-studio`, `pnpm check`, and `git diff --check` passed. No encode or media write occurred. Live TMDB/TheTVDB authentication, native Tauri dialogs/keyring, dependency repair, and packaged-app launch remain unverified.

## 2026-09-27 Filename-assisted film and series metadata

- Added a bounded Rust filename-clue command that accepts basenames only and extracts title, year/range, and certification locally. The metadata workbench now lists analyzed films and detected TV series folders, supports batch and per-item public-source lookup, uses year clues only for local ranking, and opens each reviewed candidate against that film or series' own editable metadata assignment. No match is applied automatically; NFO sidecar and artwork writes remain separate preview-and-confirm actions.
- Updated movie/series responsive layouts and README, technical, migration, TODO, and agent guidance. The user-provided sample was previously inspected metadata-only with FFprobe: 1920×1088 H.264/BT.709, AC-3 5.1, a data stream, and attached MJPEG artwork. MediaInfo was not found on PATH. No provider request, encode, or media write was performed.
- Validation: `pnpm check`, `pnpm check:tsgo`, `pnpm build`, and `cargo check --locked --bin hevc-rebuild-studio` passed. No tests were run. Native Tauri interaction and live provider results remain unverified; Python still owns Matroska/NFO mutation and broader run orchestration.

## 2026-09-27 Rust release-group suffix policy

- App-launched cleanup in `metadata_text.rs` now applies both configured text blacklist patterns and the existing conservative built-in release-group suffix heuristic. It preserves ordinary title endings, technical codec tokens, and trailing whitespace, and returns the same `release-group suffix` rule in the change record. Python keeps its direct-engine reference and retains XML traversal, Matroska mutation, and report orchestration. Follow-up validation: `cargo check --locked --bin hevc-rebuild-studio` and all 108 Rust library tests passed; all 17 metadata Python tests passed with the native helper and with direct fallback (2 native-only skips).
- Validation: 10 focused Rust metadata-text tests passed; 17 metadata Python tests passed using the Rust helper and 17 passed in direct Python mode (2 native-only tests skipped). `cargo build --locked --bin hevc-rebuild-studio` succeeded. No user media was read or written.

## 2026-09-27 Metadata workbench uses the active library analysis

- The metadata workbench now receives the current Media Library `ScanResult`. Movie files appear directly in the workbench; a selected filename can seed the provider search. TV episodes are grouped by detected series folder, and the selected folder scopes episode mapping, NFO destinations, and artwork targets. The user still reviews every provider match and confirms metadata writes.
- Full-series rename remains gated on a complete organizer scan because the Media Library analysis applies a configurable minimum-size filter. The workbench can scan the currently associated series folder when the user wants the full-tree rename flow.
- Validation: pnpm 12.7.0 `pnpm check` and `pnpm check:tsgo` completed with 0 errors and 0 warnings; `pnpm build` completed successfully. No tests were run, no media was scanned or modified, and native Tauri interaction was not verified.

## 2026-09-27 Rust-launched source profile construction

- Rust-launched rebuild jobs now obtain source and selected-video-track profiles from private Rust helpers in `media_scan.rs`. `engine-profile` runs configured FFprobe and optional MediaInfo once; `engine-profile-tracks` profiles only selected additional video streams from the same bounded metadata payload. `probe.py` validates and adapts responses into the compatibility engine structures; direct Python runs retain their profiler.
- The project package-manager pin and pnpm lock metadata now use pnpm 12.7.0, the latest stable release on the official [pnpm release list](https://github.com/pnpm/pnpm/releases). Tauri 2.12.0 and Rust 1.98.1 were checked through their [official Tauri releases](https://github.com/tauri-apps/tauri/releases) and [Rust release announcement](https://blog.rust-lang.org/releases/latest/).
- `rustfmt --edition 2021 --check desktop/src-tauri/src/media_scan.rs desktop/src-tauri/src/lib.rs`, `cargo check --locked --bin hevc-rebuild-studio`, Python bytecode compilation of `app.py` and `probe.py`, pnpm 12.7.0 frozen lockfile validation, `git diff --check`, and `sha256sum --check --quiet SHA256SUMS` passed. `pnpm outdated --long` shows TypeScript 7.0.2 as the only newer direct npm candidate; TypeScript 6 remains the Svelte checker default with the separate TypeScript 7 `--tsgo` lane. `cargo update --dry-run --verbose` reports zero compatible Rust dependency updates and four graph-constrained older transitives. No tests, media scan/encode, package operation, native GUI interaction, or cross-platform build was run. Source-profile runtime/parity validation remains open; Python is still required for app-launched workflow orchestration and direct CLI runs.

## 2026-09-27 Rust discovery and toolchain preflight

- Rust-launched rebuild and YouTube jobs now get their input batches from `media_scan.rs`; a reached 10,000-file cap fails the run clearly. The app-launched no-repair resolver in `environment.rs` now resolves configured/PATH tools, checks required binary versions and encoder capabilities, and runs bounded libx265 or H.264/AAC smoke encodes. `discovery.py` and `deps.py` keep direct Python fallbacks; Python still launches and orchestrates media jobs.
- Validation: `cargo fmt --all -- --check`, `cargo check --locked`, Python compileall, and `git diff --check` passed. No tests, user-media scan/encode, package repair, or cross-platform build was run. No dependency versions changed.
- Freshness: Node 26.9.0, pnpm 12.6.0, and rustc/cargo 1.98.1 are installed. `pnpm outdated --long` found only TypeScript 7.0.2 above the default 6.0.3; the TS7 alias is already present and its `svelte-check` lane remains opt-in while both versions are required for the documented checker setup. `cargo update --dry-run --verbose` reports zero compatible upgrades, with four transitive crates graph-constrained. The Tauri 2.12.0 release (Sep 26) is already pinned. See the [Tauri releases](https://github.com/tauri-apps/tauri/releases), [svelte-check TypeScript 7 guidance](https://github.com/sveltejs/language-tools/blob/master/packages/svelte-check/README.md), and [TypeScript releases](https://github.com/microsoft/TypeScript/releases).
- Remaining Python responsibilities include app/run orchestration, progress/retry/error policy, report-value generation/logs, Matroska edits and parts of final validation, HDR10+ reinsertion, YouTube outcomes/consent, and direct CLI/reference paths. Do not remove the Python runtime/package requirement until these reach parity.

## 2026-09-27 Rust output stream validation and package inspection

- Added `validation.rs` as the native stream-level half of output validation for Rust-launched GUI/CLI/MCP rebuilds. It checks selected video identity/order/count and output format, copied stream metadata/dispositions and payload hashes, audio/subtitle parameters and timing, attachments/data, duration, and chapter invariants. Python still checks output HDR profiles, expected Matroska tags/poster identity, and coordinates reports and the final gate.
- Validation: all 95 Rust library tests passed; all 102 Python tests passed with the native helper and all 102 passed in direct mode (16 native-only skips). A synthetic FFmpeg run accepted copied FLAC/audio attachment streams and rejected re-encoded audio. `pnpm check` reported 0 errors and warnings; the source-built Tauri release produced both Ubuntu bundles. `SHA256SUMS` verification and `git diff --check` passed.
- Package inspection: the 5.27 MiB amd64 `.deb` declares `python3` and contains the Python engine. The 86.45 MiB x86-64 AppImage also contains `engine/hevc_rebuild/*.py`. Neither artifact was installed or launched; Ubuntu 24.04 baseline compatibility and Windows/macOS remain unverified.
- Freshness review: `pnpm outdated --long` reports TypeScript 7.0.2 as newer than 6.0.3. The standard Svelte check passed on TypeScript 6.0.3; retain that supported default and the separate TS7 `--tsgo` lane. Registry checks confirm pnpm 12.6.0, Tauri JS API/CLI 2.12.0, and dialog plugin 2.8.0 are current stable; Rust 1.98.1 is up to date, and Node 24.21.0 is the configured LTS line. Cargo dry-run found no compatible lockfile updates; four transitive crates remain graph-constrained. No dependency changes were needed.
- Remaining: port output HDR-profile and Matroska metadata/poster validation, run/error/report orchestration, YouTube output, and the direct engine CLI with parity before removing Python from package dependencies.

## 2026-09-27 Rust per-file CSV report appends

- Added a private `reports.rs` append action and routed Rust-launched engine report rows through it. Rust owns the established fixed columns, CSV escaping, and append operation; it restricts writes to the active HEVC Rebuild log directory and expected report filename. Python still prepares row values, creates the log, and retains the direct-engine writer.
- Validation: `cargo fmt --all -- --check`, `cargo build --locked --bin hevc-rebuild-studio`, `python3 -m compileall -q hevc_rebuild`, and `git diff --check` passed. No unit or integration tests, report append smoke, or media run was performed. Only the Ubuntu x86_64 target was built; the Windows/macOS path is not runtime-verified. No dependency or lockfile changes.
- Version review: the official releases list shows Tauri 2.12.0, Rust 1.98.1 is installed, and Node 24.21.0 remains the configured LTS line. `pnpm outdated --long` lists only TypeScript 7.0.2 above the pinned 6.0.3; keep 6.0.3 for the default Svelte checker and the existing 7.x `--tsgo` lane. `cargo update --dry-run --verbose` found zero compatible lockfile upgrades; four older transitive crates remain graph-constrained.
- Remaining: move report-value construction, run orchestration, output validation/HDR/metadata, and direct-engine fallback behavior into Rust with parity before removing the packaged Python engine.

## 2026-09-27 Rust x265 calibration benchmark and cache

- Added `desktop/src-tauri/src/calibration.rs` and routed Rust-launched calibration cache reads/writes, null-output candidate supervision, Linux RAM/swap/PSI sampling, unsafe-candidate termination, and throughput-versus-memory selection through the Rust executable. Python supplies analyzed profile/options; Rust computes eligibility/workload fingerprints, builds fixed bounded null-output FFmpeg vectors, and supervises/selects them. Direct Python CLI runs retain the full reference path. Candidate layouts remain within the existing resource plan and real encodes keep the live guard.
- Validation: `cargo fmt --all`, `cargo check --locked --bin hevc-rebuild-studio`, `python3 -m compileall -q hevc_rebuild`, and `git diff --check` passed. No tests were run and no media was opened or benchmarked. Only the Ubuntu x86_64 target was compiled; calibration runtime behavior and Windows/macOS branches remain unverified. No dependencies or lockfiles changed.
- Remaining: port calibration cancellation/report orchestration and the direct engine/run workflow; prove source/HDR-option parity before removing the Python runtime. The Rust module currently depends on Linux resource telemetry, so this work does not establish Windows/macOS calibration support.

## 2026-09-27 Rust validated-output transactions

- Added `desktop/src-tauri/src/transactions.rs` and routed Rust-launched GUI/CLI/MCP rebuild commits through the native helper after Python's current output validator succeeds. The helper syncs the temporary file, rejects occupied output/backup paths, uses no-overwrite moves, moves the original into `old/`, installs the validated result, and attempts to restore the original if installation fails. If syncing the installed output directory fails, it keeps both the rebuilt output and the original backup. Python still chooses the backup filename and retains the legacy direct-Python fallback.
- Validation: `cargo check --locked --bin hevc-rebuild-studio` passed. No tests were run and no media transaction was executed in this turn. Linux was the only compiled target; macOS/Windows implementations are source-only and remain unverified. There is no crash journal or power-loss recovery guarantee.
- No dependency or lockfile changes were made. Broader Python removal is incomplete: run orchestration, calibration, full output comparison/HDR/metadata, reports, YouTube output, and direct CLI still need Rust parity.

## 2026-09-27 Rust FFmpeg child supervision

- Added `desktop/src-tauri/src/encode_supervisor.rs` and connected Python `run_encode()` to the helper whenever the Rust GUI, CLI, or MCP executable launches the compatibility engine. This also covers the YouTube exporter, which uses the same encoder path. Rust now creates and reaps the FFmpeg child, relays the existing progress key/value stream through a bounded queue (16 lines; 64 KiB maximum per line), forwards diagnostics, preserves ordinary exit codes and signal status, and stops the child on explicit cancellation or Python-adapter pipe EOF. On Linux, `PR_SET_PDEATHSIG` also stops FFmpeg if the supervisor is force-closed. Python remains responsible for interpreting progress, requesting Rust guard decisions, retry/report orchestration, calibration, and the direct-engine process fallback.
- Validation: `cargo test --locked` (74 Rust tests), `cargo build --locked --bin hevc-rebuild-studio`, `cargo fmt --all -- --check`, and Python compileall passed. All 96 Python tests passed with the native helper; 96 direct-Python tests passed with 10 Rust-only cases skipped. Five new supervisor tests cover progress/stderr relay, nonzero and signal exits, explicit cancel, and adapter-pipe EOF cleanup; the existing synthetic FFmpeg integration also passed through the supervisor. No user media, native window, live MCP session, installer, or Windows/macOS target was exercised.
- Fresh dependency review: Node 24.21.0 remains the selected LTS in `.nvmrc`; this host's Node 26.9.0 is on the Current line. Pinned pnpm 12.6.0 showed TypeScript 7.0.2 as the only newer direct npm candidate; retain TypeScript 6.0.3 for the standard Svelte checker and the existing TS7 `--tsgo` lane. `cargo update --dry-run --verbose` found zero compatible lock updates; four older transitive crates remain graph-constrained. No dependencies or lockfiles changed.
- Remaining: Python still runs the workflow, interprets progress and guard decisions, applies tunes, calibrates, and writes reports; HDR10+/Dolby Vision processing, Matroska metadata, full output validation, transaction/rollback, and YouTube policy are not Rust-complete. The process-supervisor lifecycle has only been exercised on Linux with synthetic children; cross-platform builds and media runs remain open.

## 2026-09-27 Rust-sampled encoder resources

- Moved host sampling for Rust-launched per-file plans into `resource_plan.rs`: it collects affinity-aware CPU count, effective RAM/cgroup allowance, swap availability/usage, and memory PSI before calculating the current profile's thread plan. Updated the persistent `runtime_guard.rs` protocol to receive process/output/progress context, collect CPU/load, RAM/swap/PSI, process-tree RSS/VmSwap, temperature, and disk state through the shared `resource_monitor.rs` sampler, then return both the telemetry snapshot and unchanged stop/retry decision. Python validates/maps those values into its existing UI/report types and continues supervising FFmpeg. Direct Python keeps its matching sampling and policy fallbacks.
- Validation: all 67 Rust tests, `cargo build --locked --bin hevc-rebuild-studio`, Python compileall, 15 focused resource tests with the helper (including a synthetic FFmpeg encode that rejects Python snapshot calls), all 90 Python tests with the helper, and all 90 tests in direct-Python mode (4 native-only skips) passed. The initial default unittest discovery command found no tests at the project root; rerunning with `-s tests` discovered and passed the suite. No user media was opened, and no APT catalog, installed packages, or system swap was changed.
- Fresh version review: the pinned pnpm 12.6.0 `outdated --long` check found TypeScript 7.0.2 as the only newer direct package than TypeScript 6.0.3. Keep TypeScript 6 for Svelte's standard checker and TypeScript 7 in the existing `--tsgo` lane. `cargo update --dry-run --verbose` found zero compatible upgrades; four older transitive crates remain graph-constrained. No version or lockfile change was justified.
- Remaining Python resource paths include direct engine execution, application preflight, calibration, YouTube planning/export, and some failure diagnostics. FFmpeg supervision/progress/cancellation/retry orchestration, metadata/HDR validation, transactions, reports, and Windows/macOS telemetry are not Rust-complete. No user-media end-to-end run, native Tauri interaction, MCP session, or cross-platform artifact was tested.

## 2026-09-27 Rust run-resource planning and guard policy

- Added `resource-plan` and `resource-retry` internal Rust helpers so Rust-launched runs use the same starting and quality-neutral retry thread plans as Rust scans. Added `runtime_guard.rs` for stateful memory-reserve, active-swap-pressure, configured disk-reserve, and projected-output decisions. Python supplies current host samples, applies selected limits, supervises FFmpeg, and maps decisions into the existing retry/stop behavior. Direct Python runs retain their planner and guard fallbacks; process supervision and calibration are still Python-owned.
- Validation: `cargo test --locked` (66 passed), Rust executable build, Python full suite with the native helper (89 passed) and direct-Python fallback (89 passed, three native-only cases skipped), `python3 -m compileall -q hevc_rebuild tests`, both Svelte TypeScript-check lanes (zero errors/warnings), and `pnpm build` passed. The focused native resource suite passed 14 tests, including Rust planner/retry selection, persistent guard policy, and a synthetic FFmpeg encode. Targeted Rust formatting, `git diff --check`, and regenerated `sha256sum --check SHA256SUMS` passed. No user media or package state was touched.
- Freshness review: the project pins pnpm 12.6.0; the host's global pnpm is 12.4.2, so use the pinned version through `npm exec`. `pnpm outdated` shows only the TypeScript 6.0.3 to 7.0.2 candidate. Keep TypeScript 6 for Svelte's default programmatic checker and TypeScript 7 in the existing `--tsgo` lane, as the upstream Svelte and TypeScript documentation describe the dual-version setup. Cargo dry-run finds no compatible lockfile updates; four transitive crates remain below their latest releases because of graph constraints. No version changes are justified.

## 2026-09-27 Rust packet verification stage

- Added `desktop/src-tauri/src/packet_verify.rs`, a Rust helper that streams FFmpeg framehash output and folds compressed packets into fixed-size per-stream SHA-256 digests. It also performs the existing stream-copy SHA-256 check for attachments. Rust-launched GUI and CLI engine runs pass their own executable path to the Python compatibility engine; the MCP engine subprocess uses the same Rust adapter. Direct Python invocation retains the Python reference implementation.
- The helper has a bounded stderr tail and never accumulates all framehash output or one object per packet. It is used only for digest calculation. Python still performs stream/metadata/timing/HDR comparisons and controls the fail-before-commit gate. If the configured helper cannot start or returns malformed/incomplete results, the run fails; there is no silent fallback inside a Rust-launched run.
- Added RustCrypto `sha2` 0.11.0 at the full caret floor and locked its graph. The official RustCrypto changelog lists 0.11.0 as the current stable release, and the installed Rust 1.98 toolchain exceeds its 1.85 minimum. See the [SHA-2 changelog](https://github.com/RustCrypto/hashes/blob/master/sha2/CHANGELOG.md).
- Validation: `cargo check --locked`, four focused Rust packet-verifier unit tests, all 61 Rust library tests, `cargo build --locked`, Python compileall, four fallback copy-hash tests, five Rust-helper copy-hash tests including exact Python/Rust digest equality, and the synthetic multi-video/audio/subtitle encode integration passed. No user media, package state, live native UI, MCP session, or installer/AppImage was exercised. Full validator parity and the Python runtime migration remain open.

## 2026-09-27 Rust pressure-retry forecast

- Extended `resource_plan.rs` to match Python's first `ResourcePlan.reduced()` plan and return it with the native scan forecast. Tauri/GUI, JSON scan, and MCP share the value; the Media library identifies the first lower-thread retry or explains when the plan is already at minimum parallelism. The Python direct CLI's check-only output now presents the same first-retry or no-reduction guidance. This is a forecast only. Python still controls live guard decisions, retries, and encoder execution.
- The shared seven-case fixture now checks starting and retry plans in Python and Rust, including the minimum-parallelism/no-reduction case. Validation passed: focused Python parity test, all 57 Rust library tests, `cargo check --locked`, `cargo fmt --all -- --check`, `pnpm check`, `pnpm check:tsgo` (0 errors/warnings), `pnpm build`, `git diff --check`, and `sha256sum --check SHA256SUMS`. No media was scanned or encoded. Native window rendering and a live run remain unverified.
- No dependency changed. The 2026-09-27 npm/Cargo freshness check found TypeScript 7.0.2 as the only newer direct npm candidate and no compatible Cargo updates; the supported TypeScript 6 and opt-in TypeScript 7 lanes both pass.

## 2026-09-26 Rust live resource observation

- Added `desktop/src-tauri/src/resource_monitor.rs` as a Linux Rust observer for CPU/load/PSI, effective RAM/cgroup limits, swap occupancy/rates, process-tree RSS/VmSwap, FFmpeg RSS, temperature, and total/used/free storage on the run volume. Tauri emits `run-resources`; the UI displays a compact live resource strip; MCP `get_job` returns the latest `resource_snapshot`. YouTube runs observe the chosen output directory, falling back to the nearest existing ancestor when it has not been created yet.
- This moves observation into Rust but not control: the Python engine still owns x265 worker selection, memory/disk enforcement, calibration, and pressure retries. No safety guard was disabled. Non-Linux runtime telemetry is not implemented.
- Validation: `cargo check --locked`, `cargo build --locked`, `cargo fmt --all -- --check`, `pnpm check`, `pnpm check:tsgo` (both 0 diagnostics), and `pnpm build` passed. No tests, live Tauri/MCP event smoke, native UI interaction, media processing, package install, or runtime resource measurement was performed. Full Python-to-Rust parity and Windows/macOS support remain open.
- Dependency freshness: `cargo update --dry-run --verbose` found no compatible lockfile changes. `pnpm outdated --long` listed only TypeScript 7.0.2 above the default 6.0.3; the official Svelte checker guide still requires both versions with `--tsgo` for TypeScript 7, and both project lanes pass. No package or lockfile was changed.

## 2026-09-26 Rust content-tune analysis

- Added `desktop/src-tauri/src/content.rs` as the Rust counterpart for Python's NFO/MediaInfo genre-based tune resolution. The shared scanner exposes genres, evidence sources, animation detection, and the tune resolved from current global settings to GUI, JSON CLI, and MCP clients. The Media library shows the live global-tune preview and per-file animation evidence beside the Animated/Normal control.
- Two focused Rust tests cover direct NFO genre parsing/entity decoding and tune-policy cases. Validation: `cargo test --locked content::tests` (2 passed), `cargo check --locked`, `cargo build --locked`, `pnpm check`, `pnpm check:tsgo` (both 0 diagnostics), and `pnpm build` passed. `rustfmt --check` passes for `content.rs`; whole-crate `cargo fmt --check` still flags an existing argument-wrapping difference in `lib.rs`. A read-only scan of the supplied sample returned no genre metadata because MediaInfo is unavailable and no matching NFO genre was found; the profile is partial/Tools required. No media was modified or encoded. No dependency was added and lockfiles were unchanged.
- This ports scan-time analysis, not the engine. Python still applies content tunes during encoding and owns resource planning, FFmpeg execution, HDR/output metadata, validation, run reports, and media transactions. Continue with runtime resource planning/process supervision, then parity-check before switching execution.

## 2026-09-26 Native Rust read-only media analysis

- Added `desktop/src-tauri/src/media_scan.rs` and routed Tauri folder/file scans, Rust CLI `json scan`, and MCP `start_folder_analysis` through one native implementation. It discovers local media with bounded filters, calls configured FFprobe and optional MediaInfo, retains raw stream/container/chapter fields, reports partial profiles/errors/warnings, and computes scan-time memory and YouTube forecasts. The GUI now exposes raw source data in expandable details, displays scan notices, and distinguishes unconfirmed HDR10+ indicators. Missing MediaInfo is a tool-readiness warning rather than a failed FFprobe probe.
- Validation: `pnpm check` (0 errors/warnings), `pnpm build`, `cargo check --locked`, `cargo build --locked`, and `git diff --check` passed. The built Rust JSON scan read the supplied sample without modifying it: H.264 1920×1088 video, AC-3 audio, a data stream, an attached MJPEG image, and 12 chapters were exposed. MediaInfo is missing, so this source profile correctly remains partial and marked Tools required. Forecast was HIGH with 6 workers / 2 frame threads during observed memory PSI pressure. No encode, package install, automated test suite, native GUI, or MCP runtime validation was performed. Scanner parity fixtures and Windows/macOS adapters remain open.

## 2026-09-26 Native cached APT checks, report history, and current toolchain

- Added `desktop/src-tauri/src/dependencies.rs` and routed the GUI check, Rust CLI `json apt-updates`, and MCP package-check/upgrade preflight through the native local-only APT report. It preserves the fixed allowlist, installed/candidate Debian version comparison, catalog timestamp, and 12-hour stale signal. The direct Python bridge retains old package actions for compatibility.
- Added `desktop/src-tauri/src/reports.rs` and routed GUI `list_reports`, MCP `list_reports`, and Rust JSON `reports` through one native CSV history reader. It handles quoted commas, embedded newlines, escaped quotes, malformed rows, bounded history, and the portable log directory; Python remains the report writer. MCP diagnostic-log validation now shares the active portable-aware log directory.
- Updated Tauri Rust crates, JS API/CLI, and dialog plugin to the current compatible stable versions: 2.12.0, 2.12.0, and 2.8.0 respectively. The Rust lockfile advanced 54 compatible packages. `pnpm-workspace.yaml` records exact-release exceptions created by pnpm for these direct version updates. Added TypeScript 7.0.2 through the upstream alias with an opt-in `pnpm check:tsgo` lane; the standard checker remains on TypeScript 6 for compatibility.
- Validation at the APT/toolchain checkpoint: all 40 Rust tests passed; `cargo build --locked`, `pnpm check` (0 errors/warnings), and `pnpm build` passed. The built Rust CLI and the Python compatibility bridge returned matching APT package entries, catalog timestamp, stale flag, and source on this host; no updates were available. The subsequent report-history port adds three Rust tests; final validation is recorded in `MEMORY.md`. No package index refresh, software installation, or media operation occurred.
- Final report-history validation: `cargo test --locked` (43 passed), `cargo build --locked`, `pnpm check` (0 errors/warnings), `pnpm build`, focused `rustfmt --check`, and `git diff --check` passed. The built CLI returned `{"reports":[]}` with an isolated nonexistent `XDG_STATE_HOME`. The first smoke attempt used an uninstalled command name and failed with command-not-found; rerunning the same native action through the built executable succeeded. No user report directory, media, or system package state was accessed.
- Ported JSON CLI APT refresh and exact-list upgrades to Rust. `apt-refresh` now requires `--confirm`; `apt-upgrade` rechecks the current report, simulates the exact confirmed package list, rejects removals or extra changes, and runs the install with `--no-remove`. Command output is sent to stderr to keep JSON parseable. `cargo test --locked` passes 45 tests; `cargo build --locked`, `cargo check --locked`, `pnpm check` (0 errors/warnings), `pnpm build`, and focused Rust formatting checks pass. Built CLI refusal checks exited before any package action when confirmation was missing; isolated `json reports` returned an empty list. APT update/install operations have not been executed.
- Remaining migration work: scanner parity fixtures and direct CLI migration; run orchestration, encoding policy, HDR/container operations, validation, report-value generation/direct-Python CSV fallback, YouTube output, Python-free package gates, and Windows/macOS validation.

## 2026-09-26 Native Rust settings and environment slices

- Added `desktop/src-tauri/src/settings.rs` for read-only effective settings resolution and validation. It applies built-in defaults, user TOML, saved app profile, then folder TOML. Tauri `load_settings`, CLI `json defaults`, MCP `get_effective_settings`, and MCP default media runs now use the Rust resolver; it never creates a config file. Four Rust tests cover precedence, null profiles/default preservation, invalid TOML/table shapes, ranges, and duplicate blacklist patterns.
- Validation: `cargo check --locked`, `cargo build --locked`, and all Rust tests (37 passed) passed. Rust and Python `defaults` payloads matched with both empty config and all three overlay layers; a local MCP stdio smoke completed `initialize` and returned effective settings. Python scan/direct CLI config parsing remains active.

## 2026-09-26 Native Rust environment checks and migration inventory

- Added `desktop/src-tauri/src/environment.rs` as the first native engine-support port. The GUI environment screen, MCP `inspect_environment`, Rust `json environment`, and environment preflights for Ubuntu maintenance now share native OS, CPU, effective RAM/swap, DRM GPU, binary-version, and encoder-capability checks. Tool output is bounded, individual checks time out, and the operation remains read-only. The current Svelte/automation response fields stay compatible.
- Added [`RUST_MIGRATION.md`](RUST_MIGRATION.md), mapping each production Python module to its current Rust owner and remaining capability gaps. This is a migration inventory, not a claim of Python parity: direct Python CLI discovery/probing, run setting application, encoding/resource policy, HDR/container metadata, validation, reports/transactions, and YouTube output still depend on Python. The Python implementation remains the reference until normal, skip, error, report, and rollback behavior are matched.
- Dependency freshness check: `pnpm outdated` found Vite 8.3.0 -> 8.3.1 and TypeScript 6.0.3 -> 7.0.2. Updated Vite and `pnpm-lock.yaml` to 8.3.1. Kept TypeScript 6 as the standard checker and then added TypeScript 7.0.2 as an upstream-supported alias with an opt-in `--tsgo` path. `cargo info` confirmed the locked Rust direct dependencies are current in their configured compatible stable major lines; Tauri 3 appeared only as alpha, so no Rust dependency upgrade was appropriate. See [Vite's release list](https://github.com/vitejs/vite/releases), [TypeScript 7's official announcement](https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/), and the [Svelte checker TypeScript guidance](https://github.com/sveltejs/language-tools/blob/master/packages/svelte-check/README.md).
- Validation: `cargo test --locked` (33 passed at the environment-slice checkpoint; 37 pass after settings integration), `cargo check --locked`, native `cargo run --locked --quiet -- json environment`, current-host comparison with `python3 -m hevc_rebuild.bridge environment`, the full `python3 -m unittest discover -s tests -v` suite (82 passed, including both `Movie*Group` examples), `pnpm install --frozen-lockfile`, `pnpm check` (0 errors/warnings), and `pnpm build` passed. The Python/Rust reports agreed on tool states, capabilities, readiness and missing-package recommendations; memory values were sampled at different times and the Python launcher resolved its own runtime path differently after adjusting `PATH`.
- A whole-crate `cargo fmt --check` still reports existing formatting drift in the already-dirty `lib.rs` and `mcp_server.rs`; the new `environment.rs` is rustfmt-formatted. A direct MCP stdio smoke completed `initialize`, listed 23 tools, and returned the native 17-check environment payload. Native-window interaction, package installation/launch, Windows/macOS adapters, media processing, and full Rust parity remain unverified. No user media or APT operation was touched.

## 2026-09-26 Reviewed poster sidecars

- Added explicit poster saving for selected film, show, and season artwork. The GUI previews exact targets; Rust CLI (`metadata artwork save REQUEST.json`) and MCP (`save_library_artwork`) expose the same typed operation. It downloads only from approved provider HTTPS image hosts, validates image bytes/type and bounded attribution fields, limits image/batch size and target count, confines writes to existing folders under the selected library root, refuses overwrites, and records source/license details in adjacent JSON sidecars.
- Matching NFO, movie-poster, and attribution sidecars are included in rename previews and move with the selected video. On a later rebuild, the Python engine recognizes film/show/season poster layouts and attaches the local artwork to the new Matroska output with available attribution in the attachment description. Synopsis, ratings, certifications, and episode text are still not written to the container.
- The confirmed blacklist rule is case-insensitive glob substring matching: `Movie*Group` matches `movieReleaseGroup`, `Movieshowgroup`, and any other text spanning from `Movie` through `Group`; only that matched text is removed. `*` may span punctuation, spaces, or an empty sequence. Existing UI/README documentation and the current Python test cover both named examples.
- Validation: `python3 -m unittest discover -s tests -v` (80 passed), `python3 -m compileall -q hevc_rebuild`, `cargo test --locked` (26 passed), `cargo check --locked`, `rustfmt --check --edition 2021 src/library_artwork.rs`, `pnpm check` (0 errors/warnings), and `pnpm build` passed. Source checksums were refreshed and verified; `git diff --check` passed. No poster was downloaded and no media or packages were touched. Native UI interaction, provider requests, live MCP handshake, actual MKVToolNix poster attachment, and packaged runtime remain unverified.
- Remaining: local-NFO-first matching, provider summary/rating/certification/episode tag writing, multi-episode/special/date-based TV mapping, crash recovery/undo, pre-run blacklist change review, full Rust media-engine parity, representative media validation, Ubuntu 24.04-baseline packaging, and Windows/macOS support.

## 2026-09-26 Rebuild-time metadata cleanup and blacklist patterns

- Added case-insensitive wildcard patterns to the structured settings list, engine CLI, and MCP `settings_json.metadata.text_blacklist`. A pattern such as `Movie*Group` matches `movieReleaseGroup` and `Movieshowgroup`; `*` spans any sequence, and only the matched text is removed. The GUI stores one pattern per removable chip in Settings with validated Add, individual Remove, and Clear all actions; Save app defaults persists the list. The supplied RenameTheThings archive contributes filename/folder exclusions, not a word-removal blacklist.
- Rebuild cleanup now applies configured patterns alongside the conservative release-group suffix rule to readable NFO-derived values, Matroska segment/global/track text, track names, and chapter titles. Encoder/statistics tags and stream payloads remain intact. Per-file reports show before/after text and rules; failure to safely extract/edit tags or chapters aborts before commit.
- The Rust settings adapter passes an explicit replacement-list boundary for current GUI/MCP settings, avoiding duplicate patterns when Python loads the same config; old settings objects without `text_blacklist` continue to use their existing config layers.
- Validation: `python3 -m unittest discover -s tests -v` (77 passed), `cargo test --locked` (21 passed), `pnpm check` (0 errors/warnings), `pnpm build`, `cargo check --locked`, `python3 -m compileall -q hevc_rebuild`, and engine CLI help passed. Final `git diff --check` and `sha256sum --check SHA256SUMS` are run after this review update. No media was processed. The open browser preview showed a blank page, so GUI appearance and native settings persistence remain unverified; real MKVToolNix execution and representative media are unavailable here.
- Remaining: pre-run text-change preview/editing, full Rust metadata behavior parity, and representative-media verification.

## 2026-09-26 Rust organizer and saved naming patterns

- Added per-file movie match assignments with editable title/year/certification. TV rename previews now require a reviewed provider-episode assignment for every selected episode instead of silently omitting unmatched selections.
- Added four persisted movie/series/season/episode naming patterns, local filename/episode scanning, exact preview paths, normalized duplicate/occupied destination checks, explicit preview-ID confirmation, and staged no-replace apply with best-effort rollback. Fixed preview rows to show a path change when a parent directory moves even if the child's filename is unchanged.
- Fixed Wikidata year extraction for timestamps with either a leading plus or minus sign; the full Rust test run exposed the leading-plus bug.
- Exposed the organizer through the existing Tauri commands, Rust CLI (`organize scan|preview|apply`), and typed MCP (`scan_library_names`, `preview_library_rename`, `apply_library_rename`). Updated README, technical notes, agent rules, TODOs, and memory. The user's clarification is reflected: manual, user-directed TMDB lookup stays available in-app; no AI/chatbot limitation is applied to this non-chat utility feature.
- Validation: `pnpm check` (0 errors/warnings), `pnpm build`, `cargo check --locked`, all Rust library tests (18/18), `rustfmt --check` on the new organizer module, `sha256sum --check SHA256SUMS`, and `git diff --check` passed. A whole-crate `cargo fmt --all -- --check` still reports broad formatting drift in existing `lib.rs` and `mcp_server.rs`; those unrelated files were left untouched.
- Not verified: native window interaction/settings persistence, MCP handshake, CLI end-to-end invocation, package rebuild/runtime, provider requests, or any media library. Rename rollback is operation-time best effort only; no crash journal/undo exists. Poster sidecars can be saved and are attached by a later rebuild, but synopsis/rating/certification/episode fields are not written into files yet. Remaining work includes local-NFO-first matching, multi-episode/special/date-based TV cases, a pre-run release-group cleanup preview, provider metadata writing, Python/Rust parity, sample-media validation, and Ubuntu packaging gates.

## 2026-09-25 Rust metadata lookup workbench

- Added bounded Rust provider clients and a dedicated **Identify & organize** screen for explicit movie and TV title lookup, candidate review, poster choices, certifications/ratings where available, and season/episode details. Public Wikidata/Wikipedia/Commons and TVmaze providers are available in the GUI, Rust metadata CLI, and typed public-source MCP tools.
- Added optional manual TMDB search and details inside the app. Per the user's direction, the in-app lookup is treated as a normal user-directed media-utility feature; no AI/chatbot limitation is applied to that GUI path. The access token is entered per call and remains in window memory, is sent only as an HTTPS Bearer token, and is not persisted or sent through CLI/MCP. Search transmits the query phrase only, never a local file name/path or media content.
- Lookups remain review-only: confidence is a hint and no match, filename, artwork, or container metadata is applied. Attribution and source/license information are shown. The TMDB attribution logo is loaded from its official static URL and can be unavailable offline.
- Validation: `cargo check --locked`, `pnpm check` (0 errors/warnings), `pnpm build`, and `git diff --check` passed. No tests, live provider request, media inspection, native packaged UI, or live MCP handshake were run.
- Remaining: local NFO/embedded-first matching, editable proposed metadata, confirmed collision-safe rename transactions, artwork download/embedding, PGS OCR, complete Python-to-Rust behavior parity, user-sample validation, and Windows/macOS support. This is a validated lookup stage, not a finished 10/10 release.

## 2026-09-25 Per-file stream selection and executable overrides

- Added per-file stream include/exclude controls to the Media library. With no override, the existing secondary-video and data-stream defaults remain authoritative. A customized selection sends absolute source paths and unique FFprobe indices through the Tauri request and MCP `start_media_run`; selected real-video streams are encoded to HEVC, selected non-video streams are copied, and the existing output validator checks the selected tracks. The UI prevents removing the final usable video stream and describes how to keep 4K and 1080p in one MKV.
- Added typed Tauri and MCP path settings for FFmpeg, FFprobe, MediaInfo, `mkvpropedit`, `mkvmerge`, and `mkvextract`. The Environment panel persists the paths, rechecks versions/readiness, and supports clearing an override to return to PATH. Legacy FFmpeg preferences/parameters remain supported.
- Hardened request boundaries: paths must be absolute/canonical files contained by the selected input, stream indices must be unique and bounded, and the serialized stream map is limited to 64 KiB. YouTube requests reject rebuild-only stream maps.
- Follow-up source checks are recorded in the newer metadata-workbench entry above. Native-window behavior, package runtime, live MCP, and a representative 4K+1080p media run remain unverified.
- Remaining user-facing gaps include native app interaction. The full Python-to-Rust parity plan, HDR10+ YouTube preservation, poster/metadata matching, PGS-to-SRT, and Windows/macOS support remain open. Ubuntu package update checks were added in the later entry above.

## 2026-09-25 Dedicated YouTube prep settings

- Added a dedicated YouTube prep page to the desktop sidebar. It reuses the existing source selector, read-only analyzer, per-file metadata table, resource forecasts, report trail, and safe-stop run controls.
- Moved output-tier, Balanced/recommended bitrate, and destination controls into that workflow. Added a saved 0.5–4.0 Balanced source-size multiplier (default 2.0); the YouTube recommended video bitrate remains the ceiling. The GUI recalculates per-file and aggregate estimates when the multiplier changes; estimates remain non-binding.
- Added the same option to the engine CLI, typed Tauri request, and MCP `start_media_run`; reports record the chosen multiplier. Run settings remain in app preferences, and CLI/MCP retain their existing explicit output-confirmation behavior.
- Validation: `python3 -m compileall -q hevc_rebuild`, `python3 -m unittest discover -s tests -v` (52 passed), `pnpm check` (0 errors/warnings), `pnpm build`, `cargo check --locked`, and `pnpm tauri build` passed. The release binary completed MCP `initialize`, listed all 10 tools, and returned `inspect_environment`. The generated `.deb` and AppImage metadata/resources were inspected; neither package was installed or launched. The AppImage was built on Ubuntu 26.04.1 and needs glibc 2.43, so it is not an Ubuntu 24.04 artifact.
- Remaining: a native-window interaction and a representative authorized media run are still needed. Per-track selections and all six tool paths were added in the later review entry above; opt-in package updates, broader Python-to-Rust parity, HDR10+ retention, metadata enrichment, PGS-to-SRT, and Windows/macOS support remain open. This is not a complete 10/10 release.

## 2026-09-25 YouTube size-aware output and tier planning

- Changed GUI, CLI, and MCP YouTube defaults to target 1440p for lower-resolution sources, preserve aspect ratio including ultrawide inputs, and never downscale a larger source. The 1440p choice may enable a higher-resolution playback rendition but is not a guaranteed YouTube codec/quality bypass.
- Added a default Balanced size mode that targets total output bitrate near twice the source's average rate, bounded by the current YouTube video-rate recommendation. The optional YouTube bitrate guidance mode uses YouTube's published target and can create much larger files. Folder analysis now returns per-file size and output-dimension memory forecasts, the UI shows aggregate/per-file estimates, and the report records estimated/actual sizes and rates. Estimates are not hard byte caps.
- Made HDR10+ dynamic metadata and omitted subtitle tracks visible before export and in reports; source captions must be added separately. Added output-dimension-aware resource planning and applies planned x265 pools/frame threads in the HDR export path.
- Remaining YouTube gaps: HDR10+ dynamic metadata is not preserved, Dolby Vision is skipped, subtitles are omitted, output size is estimated but not strictly capped, and this is still a file-preparation workflow rather than an authenticated uploader. The Python engine remains active pending the broader Rust parity migration.
- Validation: `python3 -m compileall -q hevc_rebuild`, `git diff --check`, `pnpm check` (0 errors/warnings), `pnpm build`, `cargo check --locked`, and CLI help passed. No tests or media scan/encode were run; native GUI/MCP runtime and packaged app were not exercised.

## 2026-09-25 Source-aware exports and per-file content tunes

- Extended the YouTube preparation path to select SDR H.264/AAC or HDR 10-bit HEVC Main 10/AAC from each source's color metadata. HDR requires 10-bit source depth, Rec. 2020 primaries, Rec. 2020 non-constant matrix, PQ/HLG tags, and an FFmpeg/libx265 build advertising `yuv420p10le`; unsupported/Dolby Vision signals and output below 720p are skipped with a per-file reason. Output validation checks codec, 10-bit pixel format, color tags, expected dimensions, container, and duration. HDR10+ dynamic metadata is not carried through.
- Expanded scan JSON with all FFprobe stream metadata plus selected MediaInfo track fields and expanded the library row to show video, audio, subtitle, attachment, and general-container details.
- Added a run-scoped per-file content-tune control and typed CLI/MCP run map. `animation` forces x265 Animation tune, `none` forces normal, and absent entries use global tune and metadata detection. Reports record the applied tune.
- Changed the additional disk reserve default from 8 GiB to 0 GiB. Per-file storage preflight still derives minimum working space from the source, and live projected-output protection now runs even when the extra floor is zero. The overview displays a largest-file estimate.
- Remaining requested work: connect per-track stream selection to the output map/validation (including safely transcoding selected 4K + 1080p video tracks into one MKV), add path overrides for tools beyond FFmpeg, and add explicit checks/upgrades for installed Ubuntu packages. No media sample was processed, so HDR output behavior is only source/build checked here.
- Validation: `python3 -m compileall -q hevc_rebuild`, CLI help, `pnpm check` (0 errors/warnings), `pnpm build`, and `cargo check --locked` passed after the code changes. `sha256sum --check SHA256SUMS` and `git diff --check` passed. No tests, media scan/encode, package repair, native app interaction, or MCP handshake were run.

## 2026-09-25 YouTube export and v1.2.7 memory-safety integration

- Rewrote the Overview headlines and descriptions in plain language and fixed the compressed-width heading layout. Increased workflow step copy from 12/13 px to 13/14 px. The SVG workflow art no longer uses tiny decorative copy.
- Added a separate SDR YouTube export for a file or recursive folder: H.264 High + AAC-LC/48 kHz MP4, source dimensions or 1440p/4K frame bounds, aspect-preserving Lanczos scaling, separate `YouTube Uploads` output, per-file progress/results, existing-output skips, and an HDR/Dolby Vision safety skip marked `SKIPPED`. Validation checks expected output dimensions against the selected profile. Sources are not moved or replaced. Upscaling increases dimensions only and cannot guarantee YouTube's playback codec or quality tier.
- Exposed YouTube-specific readiness (Python, FFmpeg, FFprobe, MediaInfo, libx264, AAC) independently of full HEVC rebuild readiness, and added `resolve_youtube_toolchain()` so uploads do not require x265, MKVToolNix, Fontconfig, or `hdr10plus_tool`. Added CLI flags and a confirmed typed MCP `youtube` run with resolution/output-directory options.
- Reviewed `HEVC-Rebuild-LinuxMint-Ubuntu-v1.2.7-Keller.zip` without extracting or executing it. Its embedded `SHA256SUMS` verified; archive SHA-256: `24b650ad4b506832281b8c86cf7bda2731683a65afe4eea41063798ebc3917dd`. Integrated its bounded-memory packet-validation digest, smaller FFmpeg probe defaults, reduced short-test stream mapping, removal of the oversized mux queue, FFmpeg process labeling, and finalizing/muxing status. Kept the current five-second short-test default rather than adopting the archive's longer test duration.
- Final source review fixed a missing return from upload validation, prevented duplicate report rows when an output already exists, and accounted for FFmpeg's automatic 90/270-degree display-matrix rotation when checking output dimensions.
- Validation: `python3 -m compileall -q hevc_rebuild`, `python3 -m hevc_rebuild --help`, `pnpm check` (0 errors/warnings), `pnpm build`, and `cargo check --locked` passed. Environment inspection was read-only: Ubuntu platform supported; FFmpeg, FFprobe, libx265, libx264, and AAC available; MediaInfo and MKVToolNix absent. Therefore full rebuild and YouTube export readiness are false on this host until MediaInfo is installed. No packages were installed, no media was scanned/encoded, no test suite was run, and no native package/MCP handshake was exercised. The in-app browser is only a disconnected visual preview.

## Local Git and dependency policy

- Initialized a local `main` repository with project-scoped author identity, LF text normalization, and no remote. The root `.gitignore` keeps generated artifacts, local state, and the external source ZIP out of source tracking.
- Updated frontend minimums to the current locked compatible releases, widened TypeScript within supported major 6, recommended Node 24 LTS, declared the supported Node 22.12/24/26 lines, and pinned pnpm 12.6.0. Cargo manifests now use three-part caret floors matching the current stable direct dependency graph; both lockfiles remain authoritative for exact builds.
- TypeScript 7.0.2 is available, but Svelte's normal `svelte-check` path still expects TypeScript 5/6. Its TypeScript 7 path needs separate compiler wiring, so this app does not adopt it in the default build.
- Version basis checked 2026-09-25 against npm/crates.io and official Node, Vite, Tauri, Svelte, and Cargo guidance. Validation is recorded below; no remote was configured and nothing was pushed.
- Validation: `pnpm install --frozen-lockfile`, `pnpm check` (0 errors/warnings), `pnpm build`, and `cargo check --locked` passed. Cargo advanced `smallvec` from 1.16.1 to 1.16.2. `pnpm outdated` found TypeScript 7.0.2 as the sole newer package; it remains outside the default toolchain until the Svelte checker supports a straightforward TS7 path.

## Visual hierarchy and themes

- Reworked the overview around three explicit stages: choose a library, review the read-only analysis, then choose a run mode. Replaced the CSS landscape decoration with a custom SVG showing source media, x265 processing, and the validated copy.
- Increased body, table, settings, navigation, and control text sizes and enlarged common interaction targets. Light and Dark use coordinated surface palettes; Hacker keeps a restrained terminal green palette without scanlines or a full-screen monospace treatment.
- Validation: `pnpm check` (0 errors/warnings) and `pnpm build` passed. Inspected browser captures at 1440x1100 for all three palettes, plus 1024x768 and 390x844 layouts; fixed a narrow-width illustration overlap found during inspection. This confirms browser rendering only. Native Tauri theme interaction, settings persistence, and host-dependent controls remain unverified.

### 2026-09-25 Light/Dark palette refinement

- Replaced the saturated green feature panel in everyday Light mode with low-contrast sage/paper surfaces and dark foregrounds. Dark mode now uses a more neutral graphite hierarchy with muted green accents.
- Made workflow-stage cards, vector illustration colors, and status/mode icon tiles adapt to each everyday palette; instrument skins continue to own their stronger colors and effects.
- Browser review covered Overview, Media library, and Run & settings in Light and Dark. Native Tauri rendering has not been captured in this session.
- Validation: `pnpm check` reported 0 errors/warnings; `pnpm build` passed. Browser visual review confirms the webview styling only; native Tauri rendering and persisted native preferences were not rechecked.

## Browser settings and native utility context menu

- Fixed browser-preview preference persistence: preview theme and other UI preferences save to a versioned browser-local key, without calling Tauri `invoke`. The native desktop settings path remains unchanged.
- Replaced the generic app-surface webview context menu with Tauri's native menu, offering choose/analyze library, app navigation, environment refresh, and context-aware library/media path or diagnostic-log copying. Selected text can use the native Copy item; editable text controls retain normal editing menus.
- Validation: `pnpm check` reported 0 errors/warnings; `pnpm build` and `cargo check --locked` passed. In the browser preview, changing to Hacker mode produced no save error and remained selected after reload; right-clicking the app surface showed no browser context menu. A native development binary compiled and started with temporary XDG config/cache paths, then was stopped; native menu actions and native preference persistence were not interacted with because this session's computer-use interface cannot control native windows.

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

## 1.3.0 package engine check-only and AppImage Python startup

- Check-only resolves only FFmpeg and FFprobe and never invokes dependency repair; absent `hdr10plus_tool` is reported as a non-blocking HDR verification warning.
- Rust-launched Python module commands remove inherited `PYTHONHOME`. linuxdeploy assigns the AppImage mount directory to that variable, which hides the system Python standard library when the app intentionally relies on host Python.
- Rebuilt the Ubuntu `.deb` (7.00 MiB) and x86-64 AppImage (87.99 MiB). A generated 2,436-byte MKV passed an engine-level MCP check-only run in both packages. Each negotiated protocol `2025-06-18`, wrote an `INFO` report, and updated the matching isolated queue state to `completed`; the AppImage test used a copied executable and sibling `.data` folder. No helper was installed, and no media output was created.
- Validation: `cargo fmt --all -- --check`, `cargo test --locked --lib` (159 passed), and the Python suite (129 passed). These checks do not establish native-window behavior, APT repair/update behavior, compatibility of this AppImage with Ubuntu 24.04, or Windows/macOS support.
