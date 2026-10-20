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

The current media engine also requires Python 3, FFmpeg/FFprobe, MediaInfo, and
MKVToolNix. Host setup is documented in [TOOLS.md](../docs/TOOLS.md).

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

## UI scope

The app provides:

- a persistent Media Library queue with native file/folder pickers and drops;
- read-only media analysis before any output is written;
- title, poster, TV episode, rename, and metadata review workflows;
- Check only, Metadata-only MKV, Short test, Full rebuild, and YouTube prep;
- Light, Dark, Phosphor, Ion, and Breach appearances;
- reports, bounded diagnostics, safe stop, retry review, and retained failed outputs;
- a signed in-app update banner for published Tauri updater releases.

## Native boundary

Rust owns the Tauri commands, app settings, queue persistence, tool inspection,
media discovery, output planning, resource guards, native process supervision,
and validated-output commit/rollback. Python remains the compatibility and
run-orchestration layer for stages not yet ported to Rust. The ownership ledger
is [python-rust-convert.md](../python-rust-convert.md).

The UI must not gain a general shell-command bridge. New native behavior should
cross through a typed Rust command and be covered by the relevant frontend,
Rust, Python, and release checks.

## Related documentation

- [Human usage](../docs/USAGE.md)
- [Platform support](../docs/PLATFORM-SUPPORT.md)
- [Release and updates](../docs/RELEASE.md)
- [UI ownership](UI_OWNERSHIP.md)
- [CSS contract](CSS_CONTRACT.md)
- [Technical notes](../TECHNICAL.md)
