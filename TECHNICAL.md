# HEVC Rebuild v1.3.0 - Technical Notes

## Design goal

Recompress the selected primary video while changing as little else as possible. The tool treats audio/subtitle mutation, silent HDR metadata loss, unsafe Dolby Vision conversion, accidental poster substitution, and replacing an unverified output as hard failures.

## Desktop Studio boundary (1.3.0 preview)

`desktop/` contains the Ubuntu desktop shell: Tauri 2 owns native dialogs, narrow typed commands, packaging, and the child-process boundary; Svelte 5 presents environment, library, run, and report workflows. The existing Python package remains authoritative for config parsing, media discovery/probing, tool validation, encoding, transaction safety, and reports. The archive's original CLI remains available.

The Rust bridge starts only fixed `python3 -m hevc_rebuild.bridge` and `python3 -m hevc_rebuild` entry points with argument arrays; the webview has no general shell capability. Its default Tauri capability grants the folder/file picker only. The bundle copies Python source modules as resources; the `.deb` declares `python3`, and the AppImage still requires system Python 3. FFmpeg, MediaInfo, and MKVToolNix are inspected/repaired separately. No standalone Python runtime or media toolchain is bundled.

The packaged executable routes `cli` to the existing engine CLI, `json` to the fixed JSON bridge actions, and `--mcp` to a local MCP server. Standalone modes locate bundled engine resources (including the AppImage `APPDIR`, or the explicit `HEVC_REBUILD_ENGINE_ROOT` override) and prepend that directory to `PYTHONPATH` while preserving the caller's working directory. The JSON CLI filters scan progress markers from stdout and forwards progress/diagnostics to stderr. CLI mode retains the engine's existing default of full rebuild unless the caller selects check-only or test mode; MCP has an explicit confirmation boundary around output-writing modes.

`desktop/src-tauri/src/mcp_server.rs` uses the official Rust MCP SDK and stdio transport. Its typed tools cover environment/settings inspection, async paginated folder analysis, tracked media runs and safe stop, report/log reading, allowlisted Ubuntu repair, and verified HDR10+ helper installation. Each MCP process owns a bounded job registry; it allows at most two simultaneous read-only analyses and serializes analysis with run/repair/install jobs to match the GUI's conflict boundaries. Scan entries are returned in pages capped at 250; recent run output is capped at 120 lines and 4096 characters per line. MCP run actions default to check-only; test/full modes require explicit confirmation fields. The host configuration points directly to `hevc-rebuild-studio --mcp`; no network listener is started.

The native preference file is `settings.json`: for AppImages it is in the writable sibling `<AppImage filename>.data/` directory; for installed `.deb` builds it is under `$XDG_CONFIG_HOME/hevc-rebuild-studio/` (normally `~/.config/hevc-rebuild-studio/`). It stores the appearance, media root, FFmpeg override, scan filters, stop-on-error preference, and optional saved rebuild profile. AppImage sidecar write failure is surfaced and falls back to per-user XDG config. Existing browser local-storage preferences are migrated once when no native settings file exists.

Saved rebuild values in `settings.json` are validated by Rust and read by the Python config loader as an application profile layer. Precedence is built-in defaults, user TOML, saved app profile, then library `.hevc-rebuild.toml`; CLI and MCP use the same effective config. Run-only edits remain in memory until the user saves them as app defaults. Python user config, logs, and the managed HDR10+ helper also use the AppImage sidecar when portable mode is active; all are kept beside the AppImage without modifying its read-only mounted image.

`hevc_rebuild.environment.inspect_environment()` is read-only. It checks supported OS, Python, APT/dpkg prerequisites, FFmpeg and libx265 pixel formats, MediaInfo, MKVToolNix, Fontconfig, HDR10+ helper, CPU/cgroup memory, and Linux DRM GPU IDs. GPU presence is informational: `libx265` remains the CPU encoder. Folder scans use FFprobe and MediaInfo where available and do not write media or user config.

`hevc_rebuild.bridge` returns effective config and report data as JSON. Its reads call `load_config(..., create_user=False)`, preserving the source precedence above without creating a user config during UI startup or a scan. Run-specific controls are validated in Rust and passed as CLI arguments; they do not rewrite TOML config. A run passes `--no-dependency-repair` and `--skip-update-check`, so startup cannot silently install or update dependencies.

The APT repair command accepts only package names returned by the engine's fixed missing-package map and additionally checks a Rust-side allowlist. After an in-app package review/confirmation it uses `pkexec` to refresh APT metadata and install those packages with `--no-upgrade`. HDR10+ helper installation is a distinct explicit action routed through the existing upstream/digest-verifying installer. Neither repair path is run by environment inspection or folder scan.

Runs stream stdout and stderr through Tauri events. Safe stop uses the engine's stop-request file: finish the current file, then stop before the next one. The report view reads CSV history and only opens canonicalized `.log` paths inside the current user's HEVC Rebuild log directory; it displays at most the final 1 MB. Logs/reports remain local.

The configured Linux bundle targets are Ubuntu `.deb` and x86-64 AppImage. Both include the Tauri application and Python engine source but depend on system Python 3; media tools remain external. The inspected AppImage includes Ubuntu 26.04.1 GTK/WebKit libraries requiring `GLIBC_2.43`, so it cannot run on Ubuntu 24.04. The packaged app executable requires `GLIBC_2.39`; AppImage must be rebuilt on Ubuntu 24.04 before claiming 24.04 compatibility. The Python engine, dependency manager, environment probes, log paths, and repair commands remain Linux-specific. Windows/macOS need separate implementation and packaged runtime work before support is claimed.

## Default encode

```text
encoder:                  FFmpeg libx265 (CPU)
CRF:                      21
preset:                   slow
bit depth:                source-exact (8/10/12)
8 -> 10 promotion:        disabled by default
chroma:                   source-exact when supported
content tune:             auto
animation detection:      Kodi NFO / embedded Genre metadata only
secondary video:          stream-copy
HDR10+:                   preserve + verify
Dolby Vision:             safe Profile-8-native path only
```

x265's preset model explicitly trades CPU time for compression efficiency; slower presets use more search to obtain lower bitrate at the selected CRF quality. `slow` is the shipped balance while `slower` and `veryslow` remain configurable.

## Bit depth

The tool no longer imposes a 10-bit minimum. It requests the pixel format corresponding to the source bit depth/chroma:

```text
8-bit  4:2:0 -> yuv420p
10-bit 4:2:0 -> yuv420p10le
12-bit 4:2:0 -> yuv420p12le
```

The same policy applies to 4:2:2 and 4:4:4. Unsupported unusual source depths are rejected rather than rounded silently. `promote_8bit_to_10bit=true` is an explicit user choice; it can change encoder quantization behavior but does not restore information absent from an 8-bit source.

## Content tuning

`content.tune="auto"` uses reliable local metadata only. If Kodi NFO or embedded general metadata contains the genre `Animation`, FFmpeg passes `-tune animation` to libx265. It does not infer animation or grain from filenames.

`grain` is manual because preserving real film grain is bitrate-expensive and should not be guessed.

## Audio/subtitle invariants

The FFmpeg command globally establishes:

```text
-c copy
```

and overrides only:

```text
-c:v:0 libx265
```

There is no production audio or subtitle encoder. Validation compares stream counts, codecs, codec parameters, dispositions, language/title metadata, timing, codec-extradata hashes, and compressed packet SHA-256 fingerprints. Any mismatch rejects the rebuilt MKV.

## Multiple video streams

Real video streams are ranked by resolution before default disposition. Attached pictures are excluded. The highest-resolution stream is mapped first and encoded; other real video streams are copied unless configured to drop.

## HDR10 / HLG

The selected primary stream is probed with FFprobe and MediaInfo. The encoder preserves/declares color primaries, transfer characteristic, matrix, range, chroma location, mastering-display metadata, MaxCLL/MaxFALL, HDR10 signaling, and HLG alternative-transfer/picture-structure signaling where applicable.

## HDR10+

Dynamic HDR10+ metadata remains an explicit extract/reinsert/verify pipeline:

```text
selected source HEVC
 -> FFmpeg Annex-B stream-copy
 -> hdr10plus_tool extract JSON
 -> x265 dhdr10-info + dhdr10-opt
 -> output HEVC/MKV
 -> hdr10plus_tool verify
```

FFmpeg primary-video frame-rate mode is passthrough to avoid casual duplicate/drop behavior that could de-align frame-associated dynamic metadata.

The verifier is a producer/consumer pipeline. `hdr10plus_tool` can legitimately finish and close stdin before FFmpeg has finished writing; FFmpeg then reports `Broken pipe` and a non-zero producer status. v1.2.2 classifies that specific shutdown as expected pipeline behavior instead of surfacing it as a media failure. Unexpected FFmpeg errors are still logged and treated conservatively.

## Dolby Vision - modern path

FFmpeg added native Dolby Vision metadata coding to the libx265 wrapper in 2024. Current capable builds expose the `dolbyvision` encoder option. When safe preservation is selected, HEVC Rebuild requires all of the following:

```text
DV profile == 8
RPU present == true
Enhancement layer present == false
Base layer present == true
FFmpeg/libx265 exposes native Dolby Vision coding
no deinterlacing/filter path that would make RPU/frame alignment unsafe
```

The encode forces:

```text
-dolbyvision:v:0 1
```

rather than relying on `auto`. FFmpeg therefore errors if encoded frames do not carry Dolby Vision frame metadata. Output validation then requires a Dolby Vision configuration record, the same profile, RPU presence, no unexpected enhancement layer, and matching base-layer compatibility ID when both sides expose it.

Older Ubuntu/Linux Mint FFmpeg builds may not expose native DV coding. In that case the safe policy skips Dolby Vision instead of using an unverified manual conversion. Profile 5 and Profile 7 are also skipped by the safe policy because their signal/enhancement-layer semantics are not equivalent to simple Profile-8 RPU preservation.

### Dolby Vision evidence and HDR10 fallback

Dolby Vision detection now records the evidence source. An FFprobe `DOVI configuration record` is preferred because it exposes profile, level, RPU, enhancement-layer, base-layer, and compatibility flags. MediaInfo remains a conservative fallback when it explicitly advertises Dolby Vision. Generic text containing only `RPU` is no longer sufficient to label a stream Dolby Vision.

The explicit `hdr10-fallback` path is allowed only when the source is Profile 7 or 8, FFprobe verifies a Dolby Vision base layer, and the selected picture independently identifies as HDR10. The encode intentionally discards Dolby Vision enhancement/RPU data. On FFmpeg/libx265 builds that expose the Dolby Vision encoder option, the fallback forces:

```text
-dolbyvision:v:0 0
```

because FFmpeg's libx265 option defaults to `auto`. Output validation then requires HDR10 to survive and rejects any output that still identifies as Dolby Vision.

## Config precedence

```text
built-in safe defaults
 -> ~/.config/hevc-rebuild/config.toml
 -> <media-root>/.hevc-rebuild.toml
 -> command-line arguments
```

The user config is generated automatically on first run. It uses Python's standard-library TOML parser; no pip dependency is needed.

## Dependency policy

APT manages FFmpeg/FFprobe, MediaInfo, MKVToolNix, fontconfig and CA certificates. The utility checks candidate updates and asks before installing/upgrading.

`hdr10plus_tool` is managed from the official quietvoid GitHub release when not available locally. The release asset must expose a SHA-256 digest/sidecar; unverifiable downloads are rejected.

The utility capability-tests libx265 and its advertised 8/10/12-bit formats. Dolby Vision is enabled only when `ffmpeg -h encoder=libx265` explicitly exposes the native option.

## Metadata/artwork

Kodi NFO fields are merged into Matroska global tags while unrelated existing global tags are retained. Track-specific tags and chapter metadata remain independent. Expected global values are read back and verified after writing.

No bundled/default artwork exists. A matching adjacent movie `-poster` or recognized season poster may replace cover art. Without matching local art, copied embedded artwork remains untouched.

## Transaction

```text
source untouched
 -> encode hidden temporary MKV
 -> apply tags/artwork
 -> validate all invariants
 -> fsync temporary output
 -> move source to old/
 -> atomically install rebuilt file
```

A collision in `old/` or final output aborts. If final installation fails after moving the source, rollback is attempted immediately.

## Adaptive CPU, memory, storage, and Linux pressure policy

`auto` mode begins at `available_logical_cpu_count - reserved_cpu_threads`; the shipped default reserves one logical thread. `unlimited` skips that CPU reserve, but neither mode skips the memory cap. Per-file planning uses current `MemAvailable`, effective total memory, cgroup-v2 allowance, memory PSI, dimensions, target bit depth, chroma, and the configured x265 preset. x265's own documentation notes that each frame encoder allocates a large amount of memory and that over-allocation can reduce performance; the planner therefore caps frame and worker concurrency before starting. `max_worker_threads` and `frame_threads` remain operator caps.

Memory safety uses Linux `MemAvailable` plus finite cgroup-v2 allowance when present. The active reserve is the larger of the configured GiB floor and percentage of usable RAM. The default is `max(3 GiB, 8%)`. The profile-scaled scheduling model is `pixels / 1080p-pixels × max(source/target bit depth, 8)/8 × chroma factor × preset lookahead factor`; chroma factors are 1.0 for 4:2:0, 1.25 for 4:2:2, and 1.5 for 4:4:4. The lookahead factor uses documented x265 preset depths normalized to the `slow` preset. One profile unit is budgeted per concurrent frame and 0.25 unit per worker; only 75% of post-reserve headroom is planned (50% when memory PSI full/10s reaches 1%). These constants are intentionally transparent heuristics, not a claim about actual peak RSS. The UI, CLI info-only output, JSON folder scan, and MCP folder analysis expose the forecast, headroom, concurrency, risk, and model warning. Actual runs recompute the plan immediately before each file. If sustained pressure crosses the live reserve, the temporary encode is stopped and retried with a lower-memory x265 plan. CRF, preset, tune, bit depth, chroma, HDR metadata, and stream-copy policy remain unchanged.

Swap occupancy and system-wide swap-in/out rates are monitored separately from fast-memory headroom. Occupancy alone is not treated as active pressure because pages can remain in swap after a workload has ended. The active FFmpeg process's `VmSwap` is also recorded from `/proc/<pid>/status` (anonymous private swap; shared-memory swap is excluded). During an encode, sustained system-wide swap-in of at least 4 MiB/s together with memory PSI `full avg10` of at least 1% for five consecutive FFmpeg progress samples triggers a temporary-output retry with fewer x265 workers/frames. The traffic/PSI signals can reflect other processes; they are a conservative host-health response, not encoder-attributed I/O. This version does not create, resize, or configure swap: swapfile changes need explicit host authorization, disk-space checks, rollback handling, and can turn an OOM into severe paging. The forecast deliberately does not claim a precise swap size required for a media file.

### Short x265 hardware calibration

The v1.2.6 archive's calibration capability is restored in the current engine and exposed through the desktop settings, CLI, reports, and typed MCP run settings. It is a pre-encode optimization layer, never a replacement for profile-aware planning or live pressure protection.

`auto` (default) calibrates 4K-or-larger media or files meeting the configured size threshold when there is no matching result from the last 24 hours. `ask` reuses a fresh result and prompts only for an uncached eligible file when an interactive terminal is available; non-interactive GUI/MCP runs skip uncached calibration. `always` bypasses the cache and calibrates every processed file while CPU threading is `auto`, so it can add repeated overhead. `off` disables benchmarks. Check-only scans never benchmark. Calibration reads one selected primary-video section, seeks to roughly 25% of the duration, and sends output to FFmpeg's null muxer. It creates no media file and does not edit the source. Defaults compare at most two layouts and encode three source seconds per candidate.

Candidate layouts begin with the current profile-aware plan, then reduce frame and/or worker concurrency. They cannot exceed the current memory-derived/manual thread caps. Each candidate uses the effective CRF, preset, tune, target pixel format, static HDR signaling, HDR10+ metadata when enabled, Dolby Vision policy, and deinterlace path. Per-process RSS, available memory, memory PSI, and system swap-in/out are sampled during the short run. If available memory stays below the configured reserve, or sustained swap-in coincides with full-memory PSI stalls, the candidate process is stopped and rejected. A healthy candidate within the configured throughput tolerance of the fastest result is chosen by lower peak RSS first. Only worker/frame counts can change; quality, HDR, and stream-copy behavior remain fixed.

The cache key includes the FFmpeg/libx265 executable fingerprint, CPU identity and available logical-thread count, effective memory, the base thread plan, media resolution/precision/chroma/HDR/interlace profile, and encode/calibration settings. Cache entries are written atomically with user-only file permissions, pruned to recent entries, and stored under the AppImage writable sidecar when portable mode is active or under the user's XDG cache otherwise. A cached layout is bounded by the freshly recomputed plan; the live RAM reserve and swap/PSI retry stay authoritative throughout the real encode. Calibration is skipped when memory telemetry is missing or already below the configured reserve.

Measurements are short, content-dependent samples and are not peak-RAM guarantees for an entire film. The runtime guard remains necessary because later scenes, x265 builds, other applications, and system memory can differ. Suggested 8 GiB and 16 GiB swap configurations remain optional system-level advice, not per-file safety guarantees; automatic swap creation/resizing is not implemented.

Disk safety has two layers. Preflight requires enough space for a hard working floor plus the configured free-space reserve and calculates a conservative temporary-output budget. Because CRF output size is not knowable in advance, falling below the conservative budget is a warning rather than an automatic rejection when the hard floor is still safe. During encoding the actual free space is sampled every FFmpeg progress interval. Once enough media has been encoded to make projection meaningful, current output size and media progress are used to estimate remaining bytes; the temporary encode is terminated before that projection can consume the disk reserve.

The original is deliberately retained under `old/` after success, so a full rebuild increases filesystem usage by approximately the new output size until the operator deletes the backup. Storage calculations and documentation explicitly account for that behavior.

Linux Pressure Stall Information from `/proc/pressure/{cpu,memory,io}` is sampled. CPU utilization is sampled from `/proc/stat`; swap-in/out rates use deltas from `/proc/vmstat` page counters and the host page size. High CPU utilization is normal for x265. Memory PSI combined with sustained swap-in activity can trigger a lower-parallelism retry; other PSI and swap-rate readings remain diagnostic. Temperature is displayed when a plausible Linux thermal/hwmon sensor is available.

System-wide PSI semantics and the optional cgroup-v2 PSI interface are documented in the [Linux kernel PSI guide](https://docs.kernel.org/accounting/psi.html). The [proc_vmstat(5) reference](https://man7.org/linux/man-pages/man5/proc_vmstat.5.html) documents the swap counters, and [proc_pid_status(5)](https://man7.org/linux/man-pages/man5/proc_pid_status.5.html) documents per-process memory fields.

A child return code of `-9` remains SIGKILL, not proof of OOM. `journalctl -k`/`dmesg` are checked for recent kernel OOM evidence. Only a supported kernel record upgrades the diagnosis to OOM. Otherwise the log records an unexplained external SIGKILL. Every file failure also writes a `FAILURE SUMMARY` containing file, processing phase, exception, resource snapshot, and confirmation that the original was not committed by the failed operation.

## Batch and terminal UI

Files are always processed sequentially, one encode at a time. A run-level card reports queue count, source bytes, CPU topology, memory reserve, and current filesystem capacity. Each file gets a compact card with source/video/HDR characteristics, x265 settings, copied stream counts, CPU plan, RAM reserve and forecast level, and storage budget. Folder scans return the same forecast per file for GUI, JSON CLI, and MCP consumers.

FFmpeg's supported machine-readable `-progress pipe:1` feed drives a two-line in-place live HUD. The first line reports current file number, file percentage, queue percentage-by-file-count, speed and ETA. The second reports CPU utilization, available RAM, FFmpeg/x265 RSS, free disk space, temperature when available, and meaningful I/O pressure. Raw FFmpeg/x265 stderr is drained concurrently to the debug log, avoiding pipe deadlocks and terminal spam. Non-TTY output uses sparse progress milestones rather than carriage-return animation.

## Upstream references reviewed for v1.2.4

- FFmpeg progress and global options: https://ffmpeg.org/ffmpeg.html
- FFmpeg codecs / native Dolby Vision encoding: https://ffmpeg.org/ffmpeg-codecs.html
- x265 preset option mapping and lookahead: https://x265.readthedocs.io/en/master/presets.html
- x265 frame-thread memory and concurrency notes: https://x265.readthedocs.io/en/master/threading.html
- x265 threading, presets, tunes and HDR options: https://x265.readthedocs.io/
- Linux PSI pressure accounting: https://docs.kernel.org/accounting/psi.html
- Linux CPU accounting via `/proc/stat`: https://docs.kernel.org/admin-guide/cpu-load.html
- hdr10plus_tool: https://github.com/quietvoid/hdr10plus_tool
- dovi_tool behavior/reference: https://github.com/quietvoid/dovi_tool
- MKVToolNix/mkvpropedit: https://mkvtoolnix.download/doc/mkvpropedit.html
