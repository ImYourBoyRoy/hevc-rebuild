# Native Rust architecture record

## Status

The product runtime migration is complete in the current worktree. The
supported application is the Rust/Tauri GUI, Rust CLI/JSON surface, and Rust
stdio MCP server. The retired implementation is not required to build, launch,
package, or run media workflows.

This file remains named for historical links. It is now a completion record,
not a live list of missing source modules.

## Native ownership map

| Area | Current owner | Evidence |
| --- | --- | --- |
| Host inspection and tool capabilities | environment.rs, dependencies.rs | GUI, JSON environment, MCP inspection, explicit setup commands |
| Settings and dynamic work roots | settings.rs, settings_arguments.rs | GUI Settings, JSON work-directory, MCP inspect_work_directory |
| Discovery and profiles | media_scan.rs, content.rs | bounded scan, stream profiles, HDR/color facts, YouTube forecasts |
| Rebuild and metadata-only runs | rebuild_run.rs, metadata_run.rs, run_coordinator.rs | shared GUI/CLI/MCP native job dispatch |
| FFmpeg process lifecycle | encode_plan.rs, encode_supervisor.rs, process_output.rs | owned process groups, progress, cancellation, descendant cleanup |
| HDR10+/Dolby Vision policy | hdr_metadata.rs, validation.rs | dynamic metadata verification, safe DV skip, explicit HDR10 fallback |
| Output validation | validation.rs, packet_verify.rs, matroska_verify.rs | stream, payload, timing, chapter, color, HDR, metadata checks |
| Transactions and reports | transactions.rs, reports.rs | no-overwrite install, rollback, retained error artifacts, CSV rows |
| YouTube | youtube.rs, youtube_optimize.rs, youtube_run.rs | source-aware Upscale/Optimize, HDR routing, MP4 validation/install |
| Queue and recovery | media_queue.rs, output_targets.rs | persisted selection, interrupted-state recovery, report association |
| Frontend and native bridge | desktop/src/routes/+page.svelte, desktop/src/lib/, lib.rs | GUI uses the same command and event contracts |
| Maintainer walkthroughs | agent_control.rs and frontend shortcut map | opt-in local control channel and bounded snapshots |

## Capability and policy notes

### Work directories

The default is dynamic, not a single global folder. For every source, auto
resolution tries source-folder/.hevc-rebuild-work/ and falls back to private
app-state work/ only when necessary. A saved absolute path or
HEVC_REBUILD_WORK_ROOT intentionally changes that behavior. Every interface can
inspect the chosen path and fallback reason.

### HDR

SDR and static HDR10 continue on the normal native path. HDR10+ preservation
uses libx265 encode-time metadata when available, otherwise verified
extract/inject/remux. The final validator checks dynamic metadata rather than
only trusting a successful encode. Dolby Vision is safe-skipped by default;
hdr10-fallback is explicit and still requires HDR10 validation.

### YouTube

Upscale creates a separate 1440p/4K output tier for smaller sources. This can
make YouTube process the upload under a higher resolution/bitrate policy, but
it cannot add source detail and is not a guarantee of viewer bandwidth.
Optimize is lossless-first for suitable large sources and validates any
re-encode. HDR10+ uses the same helper-preservation policy as rebuilds.

## What remains open

The remaining work is acceptance and product hardening, not a missing scripting
engine:

- real Tauri window walkthroughs for pickers, drag/drop, queue, reports,
  close/reopen, and live stop-now;
- screen-reader workflow proof;
- native Windows/macOS runtime and clean installer evidence;
- Linux AppImage rebuild and clean installation on the supported glibc
  baseline;
- representative real-media runs for SDR, static HDR10, HDR10+, and explicit
  Dolby Vision fallback on supported hosts;
- route/component extraction, characterization tests, controller tests, and
  route snapshots tracked by the newest tasklist; and
- resolving the known interaction-map and waterfall static selector warnings
  rather than treating them as ignored checks.

These gates must remain visible. Passing Rust/frontend source checks does not
close them.

## Verification recorded for the current worktree

The current migration pass has verified:

- no production Rust/Svelte path launches or imports the retired runtime;
- native short-test rebuilds for static HDR10 and HDR10+ have reached validated
  outputs in the local evidence set;
- HDR10+ helper verification detects dynamic metadata in the existing Doctor
  Sleep evidence;
- YouTube Upscale and HDR10+ output validation are native;
- Cargo lock regeneration and the native agent-control module compile on Linux;
- frontend checks, production build, native library tests, strict Clippy, and
  native binary builds are the required source gates.

The real-window and clean-package items above are intentionally not claimed
until directly executed.
