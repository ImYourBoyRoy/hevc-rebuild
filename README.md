# HEVC Rebuild Studio for Ubuntu and Linux Mint

HEVC Rebuild compresses the **video** to H.265/HEVC with CPU x265 while protecting the rest of the media file.

**Audio is never transcoded. Subtitles are never transcoded.** They are copied and verified after the rebuild. If a file contains several real video tracks, the **highest-resolution track is the one that gets compressed**, so 4K automatically wins over 1080p.

The safe defaults are **CRF 21 + preset slow**. The program keeps the source bit depth by default: 8-bit stays 8-bit, 10-bit stays 10-bit, and 12-bit stays 12-bit. It can optionally promote 8-bit video to 10-bit if you turn that setting on.

Your original file stays untouched until the rebuilt MKV passes validation. After success, the original is moved into an `old/` folder.

## Desktop GUI (v1.3.0 preview)

The Ubuntu-first desktop interface is built with **Tauri 2 + Svelte 5** and keeps the existing Python rebuild engine as the media-processing authority. The engine and `.deb` target Ubuntu 24.04+ and Ubuntu-based Linux Mint 22+; Windows and macOS are future targets, not supported builds yet. The current AppImage was built on Ubuntu 26.04.1 and bundles GTK/WebKit libraries requiring glibc 2.43, so this artifact is not compatible with Ubuntu 24.04's glibc 2.39.

The desktop workflow includes:

- read-only CPU, memory, GPU, operating-system, FFmpeg/libx265, and required-tool inventory;
- library folder selection, recursive analysis, minimum-size filtering, per-file media/HDR/stream summaries, disk-space visibility, scan errors, and a file-by-file memory forecast;
- the effective application/user/library settings, loaded from the existing config code rather than duplicated UI defaults;
- cached, short x265 hardware calibration for demanding 4K/large sources, with candidate results and the selected thread layout recorded in reports;
- Check only, 5-second short tests (test CRF 30), and confirmed Full rebuild modes;
- a safe stop request that finishes the active file before stopping the queue;
- explicit Ubuntu APT repair for a fixed package allowlist, plus a separate HDR10+ helper install action;
- FFmpeg executable override, report history, per-file outcomes, and a bounded diagnostic-log viewer.
- Light, Dark, and Hacker appearances plus saved library/app preferences, persisted in native settings rather than browser storage;
- local command-line and MCP entry points that use the same Python engine, effective settings, and validated operations.

The encoder is **CPU libx265** even when a GPU is detected. Hardware inventory informs diagnostics and resource planning; this release does not use GPU encoding. Opening the environment screen and scanning do not install packages or write the user config. System repairs run only after you choose them; APT repair shows the package list and asks for confirmation. Rebuild edits remain run-only until you choose **Save as app defaults**. Those saved values are shared with the CLI and MCP server, layer after the user config, and remain below a library-specific `.hevc-rebuild.toml` override.

The Ubuntu AppImage stores `settings.json`, its engine config, logs, and the managed HDR10+ helper in a writable sibling directory named `<AppImage filename>.data/`. Move that folder with the AppImage to keep the portable profile. If the folder beside the AppImage cannot be written, the app reports that and falls back to the user's XDG config location. The `.deb` installer stores application preferences in `$XDG_CONFIG_HOME/hevc-rebuild-studio/settings.json` (normally `~/.config/hevc-rebuild-studio/settings.json`); it does not attempt to write beside the system executable under `/usr/bin`.

Use the app with local media you own or are authorized to process, such as home media-server libraries and lawful personal backups. HEVC Rebuild does not download media, remove DRM, or distribute content.

### Command line and automation

The packaged desktop executable also exposes the existing engine CLI and a JSON bridge:

```bash
hevc-rebuild-studio cli --help
hevc-rebuild-studio cli "/path/to/Media" --recursive --info-only
hevc-rebuild-studio json environment
hevc-rebuild-studio json defaults --root "/path/to/Media"
hevc-rebuild-studio json scan --root "/path/to/Media" --recursive --min-size-mib 5
hevc-rebuild-studio json reports --limit 20
```

`cli` forwards the engine's normal options, including its existing setup and rebuild controls. Like the original engine, a CLI invocation runs a full rebuild unless you pass `--info-only` or `--test`; command invocation is the operator's explicit action. `--info-only` prints each file's resource forecast and planned worker/frame counts without encoding. `json scan` includes a `resource_forecast` object on every successfully analyzed media entry, so MCP/CLI automations can inspect the same plan. `json` supports the bridge actions `environment`, `scan`, `defaults`, `reports`, and the explicit `install-hdr10plus` helper action. JSON results go to stdout; scan progress and diagnostics go to stderr. Successful JSON actions return one JSON document. Environment inspection, defaults, reports, and scanning are read-only. APT setup and HDR helper installation change the host/user environment and are deliberate commands.

### MCP server

HEVC Rebuild Studio includes a local MCP server over stdio in the same executable. Add it to an MCP host configuration using the installed command on `PATH`:

```json
{
  "mcpServers": {
    "hevc-rebuild": {
      "command": "hevc-rebuild-studio",
      "args": ["--mcp"]
    }
  }
}
```

The server exposes typed tools for `inspect_environment`, `get_effective_settings`, `start_folder_analysis`, `get_job`, `start_media_run`, `request_safe_stop`, `list_reports`, `read_diagnostic_log`, `repair_system_dependencies`, and `install_hdr10plus_helper`. Folder analysis is asynchronous and returns a job ID; retrieve per-file results in pages of up to 250 entries, including probe errors. Runs default to check-only. Short tests and full rebuilds require explicit confirmation parameters; safe stop finishes the current file. Package repair is limited to the exact missing-package list reported by environment inspection and uses the normal administrator authentication prompt.

The MCP transport uses stdin/stdout rather than opening a network listener. Keep MCP enabled only in the local AI host where you want to grant access to your machine's media-analysis and run controls. The CLI and MCP server are available from both Ubuntu desktop outputs; Windows and macOS remain future targets.

### Develop or build on Ubuntu

Install Tauri's Linux build prerequisites first. Follow the official [Tauri Linux prerequisites](https://v2.tauri.app/start/prerequisites/) for `libwebkit2gtk-4.1-dev`, GTK, Rust build tools, OpenSSL, and related system libraries. Python 3 is required by the current engine.

For the frontend, Node 24 LTS is the recommended toolchain (`.nvmrc`); the project supports Node 22.12+, 24, and 26. `desktop/package.json` pins pnpm 12.6.0 and declares compatible-major dependency ranges, while the committed pnpm lockfile fixes the exact tested dependency versions.

From the extracted project directory:

```bash
cd desktop
pnpm install --frozen-lockfile
pnpm tauri dev
```

Build the Ubuntu installer and portable AppImage:

```bash
pnpm tauri build
```

The configured outputs are under `desktop/src-tauri/target/release/bundle/deb/` and `desktop/src-tauri/target/release/bundle/appimage/`. The `.deb` declares Python 3 and the normal GTK/WebKit runtime dependencies. The AppImage is a single-file GUI distribution but still requires system Python 3 and the external media tools that the engine inspects/repairs; it does not bundle FFmpeg, MediaInfo, or MKVToolNix. The current AppImage is for Ubuntu releases with glibc 2.43 or newer; build on Ubuntu 24.04 before advertising AppImage support for Ubuntu 24.04. Make it executable and run it directly:

```bash
chmod +x "./<generated-file>.AppImage"
"./<generated-file>.AppImage"
```

The adjacent `<generated-file>.AppImage.data/` folder is the portable profile and should travel with the AppImage. Install the `.deb` with Ubuntu's package manager, for example `sudo apt install ./<generated-package>.deb`.

For source checks and a frontend build:

```bash
pnpm check
pnpm build
```

The frontend is offline-first: it does not fetch fonts, scripts, or remote UI content. The Rust bridge exposes fixed application operations and does not give the frontend a general shell command API. For platform packaging and backend boundaries, see `TECHNICAL.md`.

### Current desktop limitations

- Ubuntu `.deb` and x86-64 AppImage are configured as distributable targets. The current AppImage includes libraries requiring glibc 2.43 because the build host is Ubuntu 26.04.1; a build from an Ubuntu 24.04 baseline is still needed for AppImage users on 24.04. The `.deb` executable requires glibc 2.39. A built package and a Linux development run establish no Windows or macOS support.
- The engine and automated repair logic remain Ubuntu/Linux Mint-specific. Cross-platform packaging needs an engine/runtime plan and platform-specific dependency repair before those targets can be claimed.
- GPU detection currently reports Linux DRM vendor/device IDs; GPU encoding is intentionally not offered.
- The MCP server currently exposes the application's operational surface as typed tools over local stdio; it does not run an HTTP endpoint.
- Short tests use the engine's fixed five-second duration and CRF 30; full rebuild uses the effective configured CRF (default 21).

## Install once

1. Extract the ZIP.
2. Open the extracted folder.
3. Right-click an empty area and choose **Open in Terminal**.
4. Copy/paste this:

```bash
chmod +x install.sh && ./install.sh
```

If Linux asks for your password, type your normal login password and press **Enter**. Nothing appears while typing the password. That is normal.

If required software is missing or has an update, the installer explains it. Press **Enter** to let it fix the software.

## Start the program

```bash
hevc-rebuild
```

If that says `command not found`:

```bash
~/.local/bin/hevc-rebuild
```

The guided mode asks for the media folder, what kind of run you want, and how to handle Dolby Vision files that cannot be preserved safely.

1. **Check only** - safest first step; nothing is changed.
2. **Short test videos** - creates short samples; originals stay untouched.
3. **Full rebuild** - processes the real files.

For Dolby Vision, Guided Mode now offers **Safe mode** or an explicit **HDR10 fallback**. The fallback is only allowed when FFprobe verifies a Dolby Vision base layer and the selected picture independently identifies as HDR10. It intentionally removes Dolby Vision from the rebuilt video; the untouched original is still retained after a successful full rebuild.

Use **Check only → Short test → Full rebuild** the first time.

## Settings

The first run automatically creates:

```text
~/.config/hevc-rebuild/config.toml
```

The defaults are recommended. You do not need to edit the file unless you want to change quality/speed behavior.

Open the settings file with:

```bash
xdg-open ~/.config/hevc-rebuild/config.toml
```

The important settings are explained directly inside the file. The main ones are:

```text
CRF 21           = recommended high quality
preset slow      = recommended compression/time balance
preset slower    = smaller files, MUCH slower
8 -> 10 bit      = optional; OFF by default
animation tune   = automatic when reliable metadata says Animation
grain tune       = manual option for real film grain
Dolby Vision     = safe preservation by default; Guided Mode can explicitly choose verified HDR10 fallback
```

You can see the active settings without processing anything:

```bash
hevc-rebuild --show-config
```

A specific media library can override the normal settings by putting a file named:

```text
.hevc-rebuild.toml
```

inside that library's root folder. Normal users do not need this.


### CPU, memory, storage, and the live display

#### Quick x265 hardware calibration

The default `[calibration].mode = "auto"` benchmarks only 4K-or-larger sources or files at least 20 GiB, and only when a matching result is not already cached. It encodes a short representative section to FFmpeg's null output, comparing up to two x265 worker/frame layouts by default. It reads the source but creates no media output. The calibration cache is machine- and workload-specific and expires after 24 hours; portable AppImage runs keep it in the writable sidecar cache folder.

Calibration changes only x265 worker/frame concurrency. It does not change CRF, preset, tune, bit depth, HDR behavior, or copied streams. The current profile-aware memory plan caps every candidate, and the same live RAM reserve, swap-pressure, and quality-neutral retry remain active during real encoding. A calibration candidate that crosses the live memory reserve or causes sustained swap-in plus full-memory stalls is stopped and rejected. Calibration is skipped if usable-memory telemetry is unavailable or already below the configured reserve. The first uncached large file may take extra time before its actual encode starts; later matching files reuse the measurement. Select `ask` to prompt in an interactive CLI when an eligible file has no fresh cached result. Select `always` to benchmark every processed file while CPU threading is `auto`; this bypasses the cache and can add repeated overhead. Select `off` to use the profile-aware plan without benchmarking.

The default `auto` mode starts from **all available logical CPU threads except one**, then may lower worker and frame parallelism for the file's resolution, target bit depth, chroma, selected x265 preset, available RAM, memory limit, and current memory pressure. The folder scan shows that starting plan for each file. `unlimited` skips the reserved-CPU policy but still uses the memory-aware cap. CPU utilization and Linux resource pressure are monitored while encoding; high CPU use by itself is expected and is not treated as a failure.

Memory protection uses both current available RAM and total effective RAM, including finite cgroup-v2 limits where available. It keeps the larger of **3 GiB** or **8% of usable RAM** available by default, then uses a profile-scaled allowance to choose initial x265 concurrency. This forecast is a conservative scheduling heuristic, not an exact peak-memory prediction: x265 memory use also varies by build, content, and encoding details. If the live reserve is crossed, only the temporary encode is stopped and retried with lower x265 parallelism. The live monitor also shows swap occupancy and system-wide swap-in/out rates. Sustained swap-in traffic alongside full-memory stalls triggers the same quality-neutral retry; allocated swap alone does not, because old pages can remain in swap after pressure has passed. **CRF, preset, HDR handling, bit depth, and picture quality never change during a resource retry.** Swap is not counted as fast RAM or resized automatically; heavy paging can make 4K encoding extremely slow.

Storage is checked before **every file** and continuously while encoding. The program keeps an 8 GiB disk reserve by default, estimates a conservative temporary-output budget, and watches the projected final output size so it can stop safely before `ENOSPC`. Remember: the untouched original is kept under `old/`, so a successful rebuild initially uses *more* disk space until you later decide to remove the backup.

The generated config contains short explanations:

```toml
[calibration]
mode = "auto"
cache_hours = 24.0
large_file_gib = 20.0
sample_seconds = 3.0
max_candidates = 2
throughput_tolerance_percent = 3.0

[resources]
threading = "auto"
reserved_cpu_threads = 1
max_worker_threads = 0
frame_threads = 0

min_free_memory_gib = 3.0
memory_reserve_percent = 8.0

min_free_disk_gib = 8.0
preflight_output_ratio = 1.30
disk_projection_margin_percent = 15.0
```

Normal users should leave these values alone. `0` for the advanced x265 thread caps means automatic.

The CLI also accepts `--calibration-mode`, `--calibration-cache-hours`, `--calibration-large-file-gib`, `--calibration-sample-seconds`, `--calibration-max-candidates`, and `--calibration-tolerance-percent`; these same effective settings are accepted through the GUI profile and typed MCP run settings.

For a batch or TV series, files are processed **one at a time**. The terminal shows a queue summary, then a compact per-file card and a two-line live HUD that updates in place: file/queue progress, speed and ETA on the first line; CPU load, RAM, swap used and active swap traffic, encoder memory, free disk space, temperature, and meaningful I/O pressure on the second. Detailed FFmpeg/x265 output remains in the log instead of scrolling across the terminal.

The utility also records Linux PSI (Pressure Stall Information) for CPU, memory, and I/O in the diagnostic log. Per-file reports retain minimum available RAM, peak encoder RSS, peak system swap use, peak FFmpeg/x265 `VmSwap`, peak swap-in/out rates, and peak full-memory PSI across successful and memory-retried attempts. If FFmpeg receives `SIGKILL`, the utility distinguishes **confirmed kernel OOM evidence** from an **unexplained external kill** instead of guessing. Every failed file gets a concise `FAILURE SUMMARY` block in the log with the phase, exception, and resource snapshot.


## What Full Rebuild does

For each media file it:

- selects the highest-resolution real video track;
- encodes only that primary video with **CPU libx265**;
- defaults to **CRF 21, preset slow**;
- keeps the **source bit depth and chroma format** whenever x265 supports them exactly;
- refuses unusual bit depths it cannot reproduce exactly rather than silently changing precision;
- automatically uses x265's **animation tune** when trustworthy Kodi/embedded genre metadata identifies Animation;
- allows manual `animation`, `grain`, normal, or automatic tuning in the config;
- **copies every audio stream without transcoding**;
- **copies every subtitle stream without transcoding**;
- SHA-256 verifies copied compressed audio/subtitle packet data and codec-private data;
- copies secondary real video tracks by default;
- preserves chapters, attachments, dispositions, language/title metadata, and timing;
- preserves existing Matroska global metadata and overlays Kodi `.nfo` metadata where available;
- uses poster artwork only when a matching local `-poster`/season poster exists;
- never invents generic poster artwork;
- preserves HDR10 mastering metadata, MaxCLL/MaxFALL, color primaries, transfer function, matrix, range, and chroma location;
- preserves **HDR10+ dynamic scene/frame metadata**, then verifies it after encoding;
- preserves HLG signaling;
- uses modern FFmpeg/libx265 native Dolby Vision RPU transcoding for **verified single-layer Profile 8** sources when the installed FFmpeg supports it;
- skips unsafe/unsupported Dolby Vision profiles in safe mode instead of silently damaging them;
- can explicitly rebuild verified Profile 7/8 HDR10-compatible base layers as HDR10 while stripping Dolby Vision;
- validates that HDR10 fallback outputs remain HDR10 and no longer carry Dolby Vision signaling;
- validates the completed MKV before touching the original.

Example after success:

```text
Movies/
├── Movie.mkv          <- rebuilt HEVC version
└── old/
    └── Movie.mkv      <- untouched original
```

**Do not delete `old/` until you have personally checked the rebuilt files.**

## Easy copy/paste examples

Check a folder only:

```bash
hevc-rebuild "/home/$USER/Videos" --recursive --info-only
```

Make short test encodes:

```bash
hevc-rebuild "/home/$USER/Videos" --recursive --test
```

Full rebuild:

```bash
hevc-rebuild "/home/$USER/Videos" --recursive
```

Force animation tuning for one run:

```bash
hevc-rebuild "/home/$USER/Videos" --recursive --tune animation
```

Use the slower preset for one run:

```bash
hevc-rebuild "/home/$USER/Videos" --recursive --preset slower
```

Temporarily promote 8-bit video to 10-bit:

```bash
hevc-rebuild "/home/$USER/Videos" --recursive --promote-8bit-to-10bit
```

Explicitly use a verified HDR10 base layer when unsupported Dolby Vision would otherwise be skipped:

```bash
hevc-rebuild "/home/$USER/Videos" --recursive --dolby-vision-policy hdr10-fallback
```

## Poster rules

For a movie:

```text
Movies/
├── Dune Part Two.mkv
└── Dune Part Two-poster.jpg
```

`.jpg`, `.jpeg`, `.png`, and `.webp` are accepted.

For TV seasons, season poster layouts from the original utility remain supported, for example:

```text
Show Name/
├── season01-poster.jpg
└── Season 01/
    └── S01E01.mkv
```

If no matching local poster exists, the tool does nothing to invent artwork. Existing embedded artwork copied from the source remains intact.

## Dolby Vision behavior

The default setting is:

```toml
dolby_vision = "safe"
```

This means:

- verified **single-layer Profile 8 + RPU** can be preserved when the installed FFmpeg/libx265 exposes native Dolby Vision RPU encoding;
- Profile 5, Profile 7/FEL/MEL, enhancement-layer sources, unknown DV layouts, or systems with older FFmpeg are **skipped**;
- the tool does not pretend that throwing away an enhancement layer is lossless;
- an expert can explicitly choose `hdr10-fallback`, and Guided Mode exposes the same choice;
- fallback requires FFprobe-verified Dolby Vision base-layer presence plus independent HDR10 identification;
- on FFmpeg builds that expose `-dolbyvision`, fallback forces `-dolbyvision 0` so the encoder's `auto` mode cannot accidentally carry RPU metadata into the HDR10 output;
- validation rejects fallback output if Dolby Vision signaling remains.

Safe mode remains the default. The fallback is intentional metadata reduction, not a lossless Dolby Vision conversion.

## If something fails

The program is designed to fail safely. If encoding, HDR/HDR10+/Dolby Vision preservation, stream-copy verification, metadata verification, or final validation fails, the original is not replaced.

At the end it prints the log and report locations. Keep those files if you need help diagnosing a problem.

## Supported systems

- Ubuntu 24.04 LTS or newer
- Ubuntu-based Linux Mint 22 or newer

For deeper implementation details, see `TECHNICAL.md` and `REVIEW.md`.

## License and project ownership

The supplied source archive did not include a license file or maintainer/repository contact. The desktop package is marked private and is not intended for publication; confirm the project's license and release ownership before redistributing it.
