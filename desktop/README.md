# HEVC Rebuild Studio desktop app

The desktop client is a Tauri 2 + Svelte 5 application. Start with the root
[README](../README.md) for the product overview and [project structure](../docs/PROJECT-STRUCTURE.md)
for repository boundaries.

## Start the app

Install the pinned frontend dependencies, then run the native development shell:

```bash
pnpm install --frozen-lockfile
pnpm tauri dev
```

Check only, integrity, Metadata-only MKV, and YouTube upload preparation run
natively in Rust and need FFmpeg/FFprobe and MediaInfo (plus MKVToolNix for
metadata-only). Short tests and full rebuilds still use the Python 3 engine. Host setup is documented in [TOOLS.md](../docs/TOOLS.md).

## Build

Build a native package on the target operating system:

```bash
pnpm tauri build
```

Configured package targets are:

| Host | Target |
| --- | --- |
| Linux x64 | AppImage and `.deb` |
| Windows x64 | NSIS `setup.exe` |
| macOS Intel / Apple Silicon | `.dmg` / `.app` |

MSI is intentionally excluded. Release-only updater artifacts are enabled by
`src-tauri/tauri.release.conf.json`; normal local builds do not create signed
updater metadata. See [RELEASE.md](../docs/RELEASE.md).

## Frontend checks

Run these from `desktop/`:

```bash
pnpm check
pnpm check:tsgo
pnpm build
pnpm test:metadata-matching
```

The focused UI contracts are available as `pnpm test:home-help`,
`pnpm test:library`, `pnpm test:rename`, `pnpm test:run`, `pnpm test:youtube`,
`pnpm test:support`, and `pnpm test:responsive`.

The pure modules extracted from the route have Node characterization tests:
`pnpm test:queue-paths`, `test:poster-urls`, `test:review-factories`,
`test:filename-template`, `test:format`, `test:outcomes`, `test:normalizers`,
`test:queue-run-state`, `test:queue-document`, `test:queue-persistence`, `test:queue-controller`, `test:run-controller`, `test:run-launch`, `test:run-overrides`, `test:review-notes`, `test:library-review`, `test:browser-preferences`, `test:window-state-controller`, `test:tool-setup`, `test:setup-updates`, `test:explanations`, `test:app-version`, `test:library-cards`, `test:artwork-controller`, `test:credentials-controller`, `test:settings-controller`, `test:app-boot`, `test:app-input`, `test:library-rename`, `test:entry-display`, `test:file-settings`, `test:reports-controller`, `test:file-pickers`, `test:tool-paths`, `test:app-navigation`, `test:library-actions`, `test:organizer-analysis`, `test:metadata-candidates`, `test:local-assets`, `test:provider-search`, `test:search-panel`, `test:match-assignments`, `test:provider-details`, `test:match-lookup`, `test:artwork-save`, `test:organizer-rename`, `test:poster-selection`, `test:organizer-session`, `test:report-view`, and `test:workspace-state`.
Route state lives in rune classes under `src/lib/state/` (app, scan, review, library UI,
run UI, YouTube, reports, credentials, settings, tool setup, artwork); the route builds one
`composeHost` over them and hands it to each controller as its typed host. Derived values live in
`AppDerived` and the effects in `installAppEffects` (both in `src/lib/state/`).
The metadata workbench keeps its 52 reactive fields in six rune classes (`src/lib/state/workbench-*-state.svelte.ts`: search, lookup, organizer, local assets, rename, artwork save); its derived values and ten effects are in `workbench-derived.svelte.ts` and `workbench-effects.svelte.ts`; the pure helpers and controllers extracted from it are `organizer-analysis`, `metadata-candidates`, `local-assets`, `provider-search`, `search-panel`, `match-assignments`, `provider-details`, `match-lookup`, `artwork-save`, `organizer-rename`, `poster-selection` and `organizer-session`, and its two largest panels are the views `WorkbenchDetailPanel` and `WorkbenchMoviesPanel`. The stylesheets are split into parts under `src/styles/app/`, `src/styles/instrument/` and `src/styles/components/` (each under 500 lines).
The route's small helpers live in focused modules (`entry-display`, `file-settings`, `reports-controller`, `file-pickers`, `tool-paths`, `app-navigation`, `library-actions`), each created in the route with a typed host and tested in node.

`pnpm test:route-golden` mounts the real route in browser-preview mode (no Tauri
bridge) and, with a fake Tauri bridge fed by `tests/golden/route-fixtures.json`
(a real `json scan` of a synthetic library, sanitized), in a populated desktop
state with queue, reviews, posters, film/series/season review flows, queue edits (reorder, hide a file, remove a failed source), a refresh scan, and an integrity run played through progress, safe stop, finish and failure with Tauri events emitted by the contract, plus the output-preview, collision, full-rebuild, stop-now and interrupted-file recovery dialogs, the series, season, episode and film review dialogs with typed edits, window restore/resize/maximize and theme changes through a fake native window, older browser-preview preference shapes, and the queue writes, start/stop calls and per-file poster/metadata overrides they trigger. It
compares the accessibility tree, tag counts and image sources for every page,
light/dark, 360/1024 px with `tests/golden/route-golden.json` (448 snapshots);
refresh it only for an intended UI change with
`pnpm test:route-golden -- --update`. The fake bridge fails the run on any native
command it does not know. It is browser evidence only: live runs, native dialogs,
drag/drop and real IPC are not covered, and it is not native-window evidence.
The contract is one function serialized for the browser, so its parts live in
`tests/route-golden/` (fake bridge, page helpers, one file per seeded scenario area, and the
preview-preference scenarios) and `route-golden-contract.mjs` embeds them; each section must stay
self-contained (it receives its state through a `ctx` object). The `test:run-controller` and
`test:tool-setup` scripts run every `tests/run-controller-*.test.mjs` / `tests/tool-setup-*.test.mjs`
file; their shared fakes are in `tests/support/`. The native payload types are split by area under
`src/lib/native-types/` and re-exported from `src/lib/types.ts`, so import from `types.js` as before.

## UI scope

The app provides:

- a persistent Media Library queue with native file/folder pickers and drops;
- read-only media analysis before any output is written;
- title, poster, TV episode, rename, and metadata review workflows;
- Check only, Metadata-only MKV, Short test, Full rebuild, and YouTube prep;
- Light, Dark, Cinema, OLED, DOS, Phosphor, Ion, and Breach appearances;
- reports, bounded diagnostics, safe stop, retry review, and retained failed outputs;
- a signed in-app update banner for published Tauri updater releases.

## Native boundary

Rust owns the Tauri commands, app settings, queue persistence, tool inspection,
media discovery, output planning, resource guards, native process supervision,
and validated-output commit/rollback. Python remains the compatibility and
run-orchestration layer for stages not yet ported to Rust. The ownership ledger
is [python-rust-convert.md](../docs/maintainer/python-rust-convert.md).

The UI must not gain a general shell-command bridge. New native behavior should
cross through a typed Rust command and be covered by the relevant frontend,
Rust, Python, and release checks.

## Related documentation

- [Human usage](../docs/USAGE.md)
- [Platform support](../docs/PLATFORM-SUPPORT.md)
- [Release and updates](../docs/RELEASE.md)
- [UI ownership](UI_OWNERSHIP.md)
- [CSS contract](CSS_CONTRACT.md)
- [Technical notes](../docs/maintainer/TECHNICAL.md)
