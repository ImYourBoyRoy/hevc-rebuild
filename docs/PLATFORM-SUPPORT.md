# Platform support contract

HEVC Rebuild Studio is a cross-platform desktop application. The user-facing
workflow is one product on Linux, Windows, and macOS:

- Media Library scan, persistent queue, file/folder picker, and drag-and-drop.
- Identify & organize, local NFO/artwork inspection, provider lookup, rename
  preview, poster storage, and metadata-only MKV creation.
- Check-only, short-test, full rebuild, YouTube preparation, reports, retained
  error outputs, and safe stop/retry behavior.
- The installed CLI, JSON actions, and local stdio MCP server.
- Theme, settings, app-data persistence, tool-path overrides, and credential
  storage through the operating system's secure store.

The behavior above must not be hidden or rejected merely because the host is
not Linux. A missing host capability must produce a precise, recoverable status
and leave the workflow available for setup or explicit executable selection.

## Host adapters

Only host integration varies by operating system:

| Concern | Linux | Windows | macOS |
| --- | --- | --- | --- |
| Tool setup | APT review/repair in the desktop; Python setup fallback | `winget` setup through the standalone engine | Homebrew setup through the standalone engine |
| Required media tools | FFmpeg/FFprobe, MediaInfo, MKVToolNix, Python 3, HDR10+ helper | Same capabilities, resolved from PATH or selected `.exe` paths | Same capabilities, resolved from PATH or selected paths |
| HDR10+ helper | Verified upstream `.tar.gz` | Verified upstream `.zip` | Verified universal macOS `.zip` |
| Settings/profile paths | XDG config/state, or writable AppImage sidecar | Windows application/local-data folders | `Library/Application Support` and `Library/Logs` |
| Package targets | `.deb`, AppImage | NSIS | `.dmg`, `.app` |
| Resource telemetry | Linux metrics where available | Portable metrics; unavailable fields are reported as unknown | Portable metrics; unavailable fields are reported as unknown |

The media engine still uses a Python compatibility layer. Python 3 is a
published host requirement until the remaining orchestration, report, metadata,
validation, HDR, and transaction stages are ported and parity-tested in Rust.
This is a runtime dependency decision, not an operating-system restriction.

## Release evidence

Source-level portability is not the same as a verified release. A release must
be built and checked on each target host, including:

1. clean installer/package installation and launch;
2. settings, queue, reports, credentials, and writable profile persistence;
3. native file/folder pickers and drag-and-drop;
4. read-only scan, metadata lookup, organizer preview, and a representative
   output-producing run with the host tools installed;
5. safe failure behavior, including retained `.error.mkv` and `.error.log`
   artifacts;
6. CLI and MCP startup from the installed executable; and
7. removal/upgrade behavior without overwriting user data.

The repository contains native CI/release lanes for Linux x64, Windows x64,
macOS Intel, and macOS Apple Silicon, plus a signed Tauri updater manifest
check. The first release does not require Apple signing or notarization; the
updater signing key is the only release secret required.

As of 2026-10-19, the final source snapshot has also been built natively on a
Windows Server 2025 x64 guest and a macOS 26.7.1 Intel guest. The Windows NSIS
bundle (with no MSI) and macOS CI-safe DMG were inspected, and their packaged
executables passed read-only CLI smoke checks. Windows browser contracts also
passed the responsive/theme, modal, home/help, library, rename, run, support,
performance, workspace-state, metadata, and YouTube lanes. This is native
compile/package and bounded smoke evidence—not a clean installer installation,
full GUI walkthrough, real-media run, Apple Silicon proof, accessibility
proof, signing/notarization proof, or live updater installation.

See [TOOLS.md](TOOLS.md) for host tool setup, [USAGE.md](USAGE.md) for the
human workflows, and [SUPPORT.md](SUPPORT.md) for diagnostics and help.
