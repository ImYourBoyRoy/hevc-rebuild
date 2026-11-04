# Technical architecture

This document describes the current product, not the retired implementation
that preceded the native migration. HEVC Rebuild Studio is a Tauri 2 desktop
application with a Svelte frontend and a Rust library/binary. The GUI, JSON
CLI, installed CLI, and stdio MCP server enter the same Rust services.

## Runtime topology

- desktop/src/routes/+page.svelte coordinates the desktop shell, page state,
  queue events, and Tauri listeners. Reusable UI belongs in desktop/src/lib/.
- desktop/src-tauri/src/lib.rs owns Tauri commands and shared job dispatch.
- desktop/src-tauri/src/main.rs routes GUI launch, CLI/JSON actions, MCP, and
  the opt-in agent-control client mode.
- Rust modules own discovery, profiles, settings, resource planning, process
  supervision, HDR policy, validation, transactions, reports, and queue state.
- desktop/src-tauri/tauri.conf.json contains no script-runtime bundle or
  hidden media engine. External media tools are resolved explicitly from
  configured paths or the host PATH.

## Shared interfaces

The GUI, CLI, and MCP contracts share:

1. settings precedence and validation;
2. dynamic source-specific work-directory resolution;
3. bounded media discovery and FFprobe/MediaInfo profiles;
4. resource forecasts and quality-neutral retry plans;
5. owned process groups and safe stop/abort behavior;
6. stream, color, HDR, timing, metadata, and payload validation;
7. no-overwrite output installation and rollback;
8. report rows and retained failure artifacts; and
9. queue persistence and recovery.

A client-specific adapter may translate arguments or presentation, but it must
not reimplement policy or silently select a different work root.

## Work-directory policy

settings.rs resolves paths.work_root = "auto" independently for each source.
The resolver tries:

    <source-folder>/.hevc-rebuild-work/

and records the source-adjacent path when it is writable. An absolute saved
path or HEVC_REBUILD_WORK_ROOT is an explicit shared-root override. If the
adjacent directory cannot be created, the resolver uses the private app-state
work/ folder and reports the fallback. json work-directory, the GUI Settings
page, and MCP inspect_work_directory expose the result before a mutating
operation. Temporary outputs never default to /tmp.

## Media and rebuild pipeline

media_scan.rs discovers bounded candidates, profiles streams and chapters,
and supplies the same source facts to the GUI, JSON, MCP, and native run
coordinators. encode_plan.rs, encode_supervisor.rs, and rebuild_run.rs
construct and supervise selected-track rebuilds. metadata_run.rs handles
metadata-only remuxes. run_coordinator.rs handles shared job lifecycle,
progress, reports, cancellation, and retained failures.

validation.rs verifies selected stream identity and order, copied payload and
extradata fingerprints, codec parameters, timing, dispositions, chapter
invariants, color, HDR10/HLG mastering/light levels, Dolby Vision policy, and
HDR10+ output state. transactions.rs installs only validated outputs and
preserves the original until the no-overwrite transaction succeeds.

## HDR policy

- SDR and static HDR10 follow the normal native encode/remux path.
- HDR10+ preservation prefers libx265 encode-time dhdr10-info when the
  capability is present.
- Otherwise hdr_metadata.rs extracts dynamic metadata, the encode preserves
  static HDR signaling, and the helper injects metadata before the final
  remux. Output verification is mandatory.
- The helper is optional for ordinary SDR/static HDR10 jobs, but a job that
  promises HDR10+ preservation fails closed when dynamic metadata cannot be
  verified or preserved.
- Dolby Vision is skipped by default. hdr10-fallback is an explicit policy
  and must produce a validated HDR10/HDR10+ result.

The same policy is used for Matroska rebuilds and YouTube MP4 re-encodes.

## YouTube pipeline

youtube.rs owns source-aware dimensions, bitrate/size policy, color/HDR
routing, FFmpeg arguments, temporary MP4 validation, and no-overwrite
installation. youtube_optimize.rs and youtube_run.rs add the Optimize policy
and batch lifecycle.

Upscale can create a separate 1440p or 4K upload copy from a smaller source.
That changes the upload resolution tier used by YouTube's processing policy;
it does not recover detail or guarantee a particular viewer bitrate. Optimize
keeps suitable 1440p/4K/8K sources and re-encodes only when its shrink and
quality checks permit it. Every export is separately validated and the source
is untouched.

## Queue, reports, and recovery

media_queue.rs persists scan snapshots, selections, reviewed metadata, posters,
output plans, and per-file outcomes. Active stages are converted to
interrupted on reopen. Interrupted or needs-review items require explicit
report/output review before a mutating retry.

reports.rs reads and appends fixed-schema rows in the private log directory.
Late failures retain .error.mkv and .error.log; early temporary files are
removed. process_output.rs and the native supervisors own child groups and
must terminate descendants on explicit stop or read failure.

## Maintainer walkthrough control

With HEVC_REBUILD_AGENT_CONTROL=1, agent_control.rs starts a private per-user
local socket on Unix or named pipe on Windows. It accepts bounded JSON-line
requests for state, snapshot, press, and screenshot. The frontend handles key
dispatch and returns bounded page/DOM text. The channel does not execute
commands, is disabled by default, and is not required by normal users.

Native development must be launched through `pnpm tauri dev` from `desktop/`.
The Tauri dev URL is `http://127.0.0.1:1420`, matching Vite's bind address;
raw `cargo run` does not launch the frontend dev server. Maintainer smoke
profiles may set `HEVC_REBUILD_PORTABLE_DATA_DIR` to a project-local folder so
the endpoint file and recyclable state do not use `/tmp`. Linux uses an
abstract local socket name recorded in that project-local endpoint file;
Windows uses a named pipe, and other Unix hosts use the app-state socket path.
Endpoint cleanup checks the owning PID before removing the record, preventing
an older process from deleting a newer process's endpoint during a restart.

Keyboard acceptance uses F8 Help; Alt+1 Library; Alt+2 Run; Alt+3 YouTube;
Alt+4 Settings; Alt+5 Reports; Alt+6 Help; Alt+M metadata; Alt+S
stop-after-current; Alt+Shift+S stop-now confirmation; and Ctrl+Enter start.
Native pickers, OS drag/drop, screen-reader announcements, close/reopen, and
live stop-now still need direct native-window evidence.

## Current validation boundary

Automated Rust and frontend checks prove contracts, compilation, and static
behavior. They do not prove a clean installer, supported-glibc AppImage
rebuild, native Windows/macOS runtime behavior, real-media acceptance on every
HDR class, screen-reader workflow announcements, or an installed updater.
Those remain release gates and are tracked in the newest sibling tasklist.

## Useful checks

~~~bash
pnpm --dir desktop check
pnpm --dir desktop check:tsgo
pnpm --dir desktop build
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked --lib
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets --locked -- -D warnings
cargo build --manifest-path desktop/src-tauri/Cargo.toml --locked --bin hevc-rebuild-studio
~~~
