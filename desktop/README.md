# HEVC Rebuild Studio desktop app

This is the Ubuntu desktop front end for the root Python HEVC Rebuild engine. Start with the root [README](../README.md) and [project instructions](../AGENTS.md) for supported systems, build prerequisites, engine behavior, and safety boundaries.

From this directory:

```bash
pnpm install --frozen-lockfile
pnpm tauri dev
```

## Toolchain and dependency versions

Use Node 24.21.0 from the repository `.nvmrc` for the recommended LTS build. `package.json` supports Node 22.12+, 24, and 26 and pins pnpm 12.6.0 with its `packageManager` field. Enable Corepack when available, or install that pnpm version directly. The dependency ranges use tested compatible-major floors; `pnpm-lock.yaml` records the exact resolved versions used by frozen installs.

Rust dependencies use caret requirements with full lower-bound versions, and `src-tauri/Cargo.lock` pins the resolved graph. Rustup's stable toolchain is the supported build channel. TypeScript 6 is used by the normal Svelte check path; TypeScript 7 support in `svelte-check` currently requires a separate native-compiler mode and a TypeScript 6 + 7 alias setup, so it is not enabled in the standard build.

The Python engine currently has no third-party Python package dependencies. FFmpeg, FFprobe, MediaInfo, and MKVToolNix remain explicit system tools. The longer-term Rust-engine migration is tracked in the root `todos.md`.

To build the configured Ubuntu Debian installer and AppImage:

```bash
pnpm tauri build
```

The UI obtains media defaults through `hevc_rebuild.bridge`; it does not maintain a separate default settings table. Choose Light, Dark, or Hacker appearance from the global top bar. Application preferences and saved rebuild defaults are stored in a native JSON file. The AppImage uses a sibling `<AppImage filename>.data/settings.json`; the `.deb` uses the user's XDG config folder. Saved rebuild defaults are also read by CLI and MCP, after user config and before a library-specific `.hevc-rebuild.toml` override.

The AppImage is not a self-contained media-processing runtime: Python 3 and the external FFmpeg, MediaInfo, and MKVToolNix tools remain system dependencies. Keep the `.AppImage.data/` directory beside the AppImage when moving a portable copy. If that location is not writable, the application reports and uses the per-user config folder.

The built executable also supports the existing engine CLI and a JSON bridge:

```bash
hevc-rebuild-studio cli --help
hevc-rebuild-studio cli "/path/to/Media" --recursive --info-only
hevc-rebuild-studio json environment
hevc-rebuild-studio json scan --root "/path/to/Media" --recursive
```

Start its local MCP stdio server with `hevc-rebuild-studio --mcp`. Register it in an MCP host as command `hevc-rebuild-studio`, args `--mcp`. Tools expose hardware/environment, effective settings, paged folder analysis, tracked runs/safe stop, reports and bounded logs, and explicit fixed-list system repair. Full rebuild and test output require explicit tool confirmation fields. Keep the media scope to local content the user owns or is authorized to process.

Folder analysis returns current effective memory totals/availability and a profile-aware x265 concurrency forecast on each supported media file. The forecast is a scheduling heuristic rather than a peak-RAM guarantee; scan again to refresh it after system memory or run settings change. `json scan` and the engine CLI `--info-only` expose the same plan. In automatic mode, a first 4K/large-media run can benchmark up to two short x265 thread layouts, cache the lower-memory near-fastest result for 24 hours, and record the selected layout in the report. Calibration creates no media output and changes only worker/frame counts; the profile-aware cap and live RAM/swap/PSI retry remain active. During an encode, the live status shows system swap occupancy/rates plus FFmpeg's own RSS and `VmSwap`; completed reports retain peak measurements. Sustained swap-in plus full-memory PSI stalls can stop the temporary attempt and retry with reduced x265 concurrency while preserving quality settings. Swap is never automatically resized. Optional 8/16 GiB system-level swap advice remains future work.

Ubuntu `.deb` and x86-64 AppImage outputs are configured. This AppImage build came from Ubuntu 26.04.1 and bundles GTK/WebKit libraries requiring glibc 2.43; it will not run on Ubuntu 24.04. Build the AppImage on Ubuntu 24.04 before claiming compatibility with that release. CLI/MCP packaging and native runtime are not supported or verified for Windows/macOS yet.
