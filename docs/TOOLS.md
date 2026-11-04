# Installation and external tools

The media engine uses external tools for media inspection and Matroska work.
The setup command installs or resolves them explicitly; it never silently
changes a user's selected executable override.

The GUI, CLI, MCP server, queue, and metadata workflows are intended to be
available on Linux, Windows, and macOS. Host setup, app-data locations, and
installer formats are platform adapters; see the [platform support contract](PLATFORM-SUPPORT.md).

Required for a full rebuild:

- FFmpeg and FFprobe with the required x264/x265 encoders.
- MediaInfo command-line tools.
- MKVToolNix: `mkvpropedit`, `mkvmerge`, and `mkvextract`.
- The HDR10+ helper (`hdr10plus_tool`) when the selected media requires
  dynamic HDR10+ preservation.

## HDR10+ preservation

When **Preserve HDR10+** is on, the engine chooses a path automatically:

1. **Encode-time** — if the host libx265 was built with `HDR10_PLUS`, FFmpeg
   passes extracted JSON through `dhdr10-info` during the encode.
2. **Post-inject** — otherwise the encode keeps static HDR10/PQ signaling, then
   `hdr10plus_tool inject` rewrites the HEVC bitstream and remuxes (mkvmerge for
   Matroska rebuilds; FFmpeg for YouTube MP4). Output verification still runs.

Ubuntu package libx265 often lacks `HDR10_PLUS`; the inject path is the portable
default on those hosts. YouTube Upscale/Optimize re-encodes use the same inject
stack when the helper is present; per-file loss consent is only required when
the helper is missing.

Scratch/work folders default to a `.hevc-rebuild-work/` folder beside each
source file, keeping temporary outputs on the source media volume. This is the
`paths.work_root = "auto"` setting shown in the GUI under **Temporary work
storage**. Set it to an absolute folder to use a shared root. The same policy
is available to all clients: direct CLI runs accept `--work-root`, MCP
`start_media_run` accepts `work_root`, and `json work-directory --root PATH`
or MCP `inspect_work_directory` reports the dynamic resolution without creating
anything.

`HEVC_REBUILD_WORK_ROOT` remains a process-level override for an operator or
test and takes precedence over the saved setting. When the automatic source
folder is not writable, the app-state `work/` folder is used as a fallback and
reported to the caller. The engine never uses `/tmp`. For media tests, the
override can point at a folder on the media volume (for example
`Media_Examples/.hevc-rebuild-work`).

## Linux

Supported Ubuntu and Ubuntu-based Linux Mint systems use APT for FFmpeg,
MediaInfo, MKVToolNix, and Fontconfig. The HDR10+ helper is
downloaded from its official GitHub release and installed only after a published
SHA-256 digest or checksum sidecar verifies the archive.

```bash
sudo apt update
sudo apt install ffmpeg mediainfo mkvtoolnix fontconfig
```

The desktop setup screen performs the same package review with explicit
administrator confirmation. The GUI, CLI, and MCP workflows themselves remain
native Rust; the commands above are only the host-tool installation lane.

## Windows

Windows setup uses the Windows Package Manager (`winget`) with fixed package
IDs for FFmpeg, MediaInfo, and MKVToolNix. The HDR10+ helper uses the official
Windows release asset and the same digest verification.

```powershell
winget install --exact --id Gyan.FFmpeg --accept-source-agreements --accept-package-agreements
winget install --exact --id MediaArea.MediaInfo --accept-source-agreements --accept-package-agreements
winget install --exact --id MoritzBunkus.MKVToolNix --accept-source-agreements --accept-package-agreements
```

`winget` must be installed and available on `PATH` for automatic package setup.
The Tauri Windows installer targets are configured, but a release still needs a
native Windows build and runtime acceptance pass.

## macOS

macOS setup uses Homebrew for FFmpeg, MediaInfo, MKVToolNix, and Fontconfig.
The app does not install Homebrew itself. The HDR10+ helper uses the
official universal macOS release asset and verifies its SHA-256 digest before
installation.

```bash
brew install ffmpeg mediainfo mkvtoolnix fontconfig
```

Install Homebrew first from [brew.sh](https://brew.sh/) if `brew` is not
available. Desktop macOS installer targets are configured, but a release still
needs a native macOS build and runtime acceptance pass.

## Safety and verification

Use `--no-dependency-repair` to inspect existing tools without installing or
downloading anything. Tool downloads use bounded HTTPS requests, fixed upstream
hosts, archive-layout checks, atomic staging, and version smoke tests. Third-party
tool licenses remain separate from this project's PolyForm license.
