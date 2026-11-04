# Project Agent Instructions

## Session order and authority

This file is the canonical repository instruction file. At the start of a
session, read this file, run `node tools/tasklist-check.mjs --open`, read the
newest sibling tasklist in `../tasklist/`, then read `ARCHITECTURE.md` and
`MEMORY.md`. Inspect only the source and documentation needed for the task.

The user request and current source/tests take precedence over older notes.
Preserve unrelated dirty work. Do not reset, clean, commit, or push unless the
owner explicitly asks. Do not scan or modify user media during repository
validation.

The dated tasklists live beside this checkout and are tracked work, but are not
part of the repository. The newest list wins when it supersedes older work;
never delete an item. A checked item needs a command and result recorded in the
list or its handoff. Run `node tools/tasklist-check.mjs` before finishing.

## Product boundary

HEVC Rebuild Studio is a Tauri 2/Svelte desktop application with one native
Rust implementation exposed through:

- the GUI;
- the installed CLI and JSON actions; and
- the local stdio MCP server.

These interfaces must use the same settings, discovery, work-directory,
planning, validation, transaction, report, queue, and stop-control contracts.
The former Python source tree and tests are retired. Do not add a Python
runtime, Python subprocess, Python parity suite, compatibility path, or
documentation command that points at a removed Python module.

## Shared behavior that must remain explicit

- `paths.work_root = "auto"` dynamically resolves a writable
  `<source-folder>/.hevc-rebuild-work/` directory for each source. An absolute
  saved setting or `HEVC_REBUILD_WORK_ROOT` is an explicit shared-root
  override. If the adjacent directory cannot be created, the app-state
  `work/` directory is used and reported. No client silently uses `/tmp`.
- `json work-directory --root PATH`, GUI Settings, and MCP
  `inspect_work_directory` expose the resolved path and reason before a run.
- SDR and ordinary static HDR10 use the standard native path. HDR10+ uses
  encode-time metadata when the host supports it or verified post-injection
  otherwise, then validates dynamic metadata. Dolby Vision is skipped by
  default; `--dolby-vision-policy hdr10-fallback` is an explicit conversion
  request and must still pass HDR10 validation.
- YouTube preparation is a separate-output workflow. Upscale may create a
  1440p/4K upload copy from 720p/1080p input to enter YouTube's higher
  resolution/bitrate tier; it does not create source detail. Optimize keeps
  already-large sources when safe and validates every output.
- Existing outputs are never overwritten. Failed late-stage outputs remain
  marked as `.error.mkv` and `.error.log`; originals remain untouched until the
  validated transaction completes.

## Native application control and acceptance

Browser/component contracts are not proof of native Tauri behavior. The
application includes an opt-in, local per-user control channel for maintainer
walkthroughs. Launch the app with `HEVC_REBUILD_AGENT_CONTROL=1`; the app
publishes an endpoint under its private app-state directory and the executable
can issue:

```text
hevc-rebuild-studio agent-control state
hevc-rebuild-studio agent-control snapshot
hevc-rebuild-studio agent-control press Alt+1
```

For development walkthroughs, run `pnpm tauri dev` from `desktop/`; it starts
the Vite server and native Tauri window together. The development URL is
`http://127.0.0.1:1420` because that is Vite's bind address. Do not use a raw
`cargo run` debug binary as a substitute. For recyclable local walkthrough
state, set `HEVC_REBUILD_PORTABLE_DATA_DIR` to a project-local folder; this
keeps the endpoint, settings, logs, and cache out of `/tmp`.

The channel is local-only, has no shell execution, returns bounded text/state,
and is disabled unless explicitly enabled. The global keyboard map is F8 for
Help, Alt+1 Library, Alt+2 Run, Alt+3 YouTube, Alt+4 Settings, Alt+5 Reports,
Alt+6 Help, Alt+M metadata, Alt+S stop-after-current, Alt+Shift+S stop-now
confirmation, and Ctrl+Enter start. Native pickers, OS drag/drop, screen-reader
announcements, close/reopen, and stop-now during a live encode still require a
real-window walkthrough; synthetic browser tests do not close those gates.
System WebDriver may be used by maintainers, but is never a runtime dependency.

## Verification and release gates

Use the pinned Node/pnpm versions from `.nvmrc`, `package.json`, and CI. The
Rust MSRV is controlled by the current Tauri release and the lockfile. Before
claiming a release is ready, run the relevant checks below and record the
environment and result:

```bash
pnpm --dir desktop install --frozen-lockfile
pnpm --dir desktop check
pnpm --dir desktop check:tsgo
pnpm --dir desktop build
pnpm architecture:check
pnpm interaction-map:check
pnpm interaction-waterfall:check

cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked --lib
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets --locked -- -D warnings
cargo build --manifest-path desktop/src-tauri/Cargo.toml --locked --bin hevc-rebuild-studio
sha256sum --check --quiet SHA256SUMS
```

Release/platform gates are separate from source checks: clean installer
installation and launch, Linux AppImage rebuilt on the supported glibc
baseline, native Windows/macOS package/runtime checks, real media for SDR,
static HDR10, HDR10+, and explicit Dolby Vision fallback, native file/folder
pickers and drag/drop, screen-reader workflow proof, live stop-now, and
installed CLI/MCP startup. Do not report any of these as complete without
direct evidence.

## Documentation and generated artifacts

Update `README.md` and the affected `docs/` files whenever commands,
configuration, file layout, interfaces, or release behavior changes. Keep
`MEMORY.md` concise and factual: record material changes, commands run,
remaining warnings, blockers, and the exact next action. Regenerate
`ARCHITECTURE.md` with `pnpm architecture` and regenerate `SHA256SUMS` after
source/documentation changes. Do not hand-edit generated architecture output.

The architecture map is generated from
`tools/architecture-map.mjs` and `docs/architecture-descriptions.json`.
Routes should coordinate; reusable logic belongs in `desktop/src/lib/` or Rust
modules with focused tests. Do not grow the already oversized route without a
characterization test and an extraction plan.

## Safety boundaries

Use explicit dry-run/confirmation boundaries for package repair, media writes,
renames, poster saves, updater installation, and stop/abort actions. Preserve
private settings, reports, credentials, and app-state data during uninstall or
upgrade unless the user explicitly requests their removal. Never expose
secrets in logs, reports, tasklists, or `MEMORY.md`.

When a required native capability is unavailable, report the exact missing
capability and preserve an explicit setup or executable-path route. Do not
paper over a failed test by ignoring it; repair the implementation or record a
real external blocker with evidence.
