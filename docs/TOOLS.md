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
- The HDR10+ helper when the selected media requires it.

## Linux

Supported Ubuntu and Ubuntu-based Linux Mint systems use APT for FFmpeg,
MediaInfo, MKVToolNix, Fontconfig, and the Python runtime. The HDR10+ helper is
downloaded from its official GitHub release and installed only after a published
SHA-256 digest or checksum sidecar verifies the archive.

```bash
python3 -m hevc_rebuild --setup-only
```

The desktop setup screen performs the same package review with explicit
administrator confirmation. This is the native Linux package-manager lane;
the rest of the desktop workflow is not Linux-only.

## Windows

Windows setup uses the Windows Package Manager (`winget`) with fixed package
IDs for FFmpeg, MediaInfo, and MKVToolNix. The HDR10+ helper uses the official
Windows release asset and the same digest verification.

```powershell
py -3 -m hevc_rebuild --setup-only --assume-yes
```

`winget` must be installed and available on `PATH` for automatic package setup.
The direct engine's Windows setup lane is supported. Python 3 is still a host
prerequisite for the current compatibility engine; the command above must be
launched with `py -3` or an equivalent Python 3 executable. The Tauri Windows
installer targets are configured, but a release still needs a native Windows
build and runtime acceptance pass.

## macOS

macOS setup uses Homebrew for Python, FFmpeg, MediaInfo, MKVToolNix, and
Fontconfig. The app does not install Homebrew itself. The HDR10+ helper uses the
official universal macOS release asset and verifies its SHA-256 digest before
installation.

```bash
python3 -m hevc_rebuild --setup-only
```

Install Homebrew first from [brew.sh](https://brew.sh/) if `brew` is not
available. The direct engine's macOS setup path is supported; desktop macOS
installer targets are configured, but a release still needs a native macOS build
and runtime acceptance pass.

## Safety and verification

Use `--no-dependency-repair` to inspect existing tools without installing or
downloading anything. Tool downloads use bounded HTTPS requests, fixed upstream
hosts, archive-layout checks, atomic staging, and version smoke tests. Third-party
tool licenses remain separate from this project's PolyForm license.
