# Human usage guide

HEVC Rebuild Studio has three interfaces. Use the GUI for an interactive
workflow, the CLI for repeatable local jobs, and MCP for an AI host or other
stdio client.

## GUI

1. Open the application.
2. Open **Settings → Environment & capabilities** and choose the setup action
   for missing tools.
3. Add a media file or folder and wait for the read-only scan. **Stop scan**
   finishes the file currently being read, then returns. Use the **Queue**
   manager to add or remove sources and files, and drag (or use the arrow
   buttons) to change run order. Removed files stay hidden across refresh until
   you restore them.
4. Review streams, subtitles, HDR information, metadata, and the output plan.
5. Choose **Check only**, **Integrity check**, **Metadata-only MKV**, **Short
   test**, or **Full rebuild**. Check and integrity run in the native Rust
   pipeline and write no media. Full rebuilds require an explicit confirmation.
6. Review the result and its report before treating an output as complete.

The source is preserved until the replacement passes validation. A completed
encode that fails a later metadata, validation, or install gate remains a
failed review artifact:

```text
The Proposal (2009 PG-13).mkv
The Proposal (2009 PG-13).error.mkv
The Proposal (2009 PG-13).error.log
```

The `.error.mkv` file is not reported as a successful rebuild.

## CLI

Use the installed executable when available:

```bash
hevc-rebuild-studio cli "/media/Movies" --recursive --info-only
hevc-rebuild-studio cli "/media/Movies" --recursive --output-dir "/media/Rebuilt"
hevc-rebuild-studio json environment
hevc-rebuild-studio json scan --root "/media/Movies" --recursive
hevc-rebuild-studio json check --file "/media/Movies/Film.mkv"
hevc-rebuild-studio json integrity --file "/media/Movies/Film.mkv" --level quick
hevc-rebuild-studio json work-directory --root "/media/Movies/Film.mkv"
```

Use `--work-root auto` or an absolute path on direct `cli` rebuild, metadata,
and YouTube runs. The JSON work-directory action reports the source-specific
folder, whether a saved setting or environment override selected it, and the
app-state fallback that would be used if the automatic adjacent folder is not
writable.

## MCP

The desktop executable exposes a local stdio MCP server:

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

Inspect the environment first. Analysis is read-only. Folder analysis can be
stopped with `request_safe_stop` after it returns a job ID; the scanner
finishes the current file, then skips the rest. JSON `scan` accepts the same
private `--stop-request-file` contract as media runs. Media runs, full rebuilds,
metadata writes,
renames, poster saves, package repair, and stop requests have separate
confirmation fields and should be shown to the user before execution.

## Modes

- **Check only** reads media and reports compatibility without writing output.
  GUI, JSON CLI, and MCP use the native Rust analyzer.
- **Integrity check** inspects container structure (Quick) or decodes every
  stream (Full) without writing output. It is native.
- **Metadata-only MKV** remuxes tracks into a new MKV without video encoding.
- **Short test** performs a bounded test encode.
- **Full rebuild** encodes selected video tracks and copies audio, subtitles,
  attachments, chapters, and other selected streams according to the reviewed
  plan.
- **YouTube export** creates a separate upload copy and never replaces the
  source.

The GUI, CLI, and MCP server call the same Rust-native run coordinators. MCP
offers `inspect_work_directory` for discovery and `start_media_run` accepts
the same `work_root` policy used by the GUI setting and CLI flag.

## Native walkthrough control

For maintainer acceptance, launch the real Tauri binary with
`HEVC_REBUILD_AGENT_CONTROL=1`. The executable then exposes local commands for
bounded state snapshots and keyboard-only navigation:

```bash
cd desktop
export HEVC_REBUILD_AGENT_CONTROL=1
export HEVC_REBUILD_PORTABLE_DATA_DIR="$PWD/../.native-smoke/manual"
pnpm tauri dev

# From a second terminal at the repository root:
./desktop/src-tauri/target/debug/hevc-rebuild-studio agent-control state
./desktop/src-tauri/target/debug/hevc-rebuild-studio agent-control press Alt+1
./desktop/src-tauri/target/debug/hevc-rebuild-studio agent-control press F8
```

`pnpm tauri dev` must be used for a development window because it starts Vite
and Tauri together. The configured dev URL is `http://127.0.0.1:1420`, which
matches Vite; a raw `cargo run` debug binary does not start that server. A
bundled release build uses its embedded frontend instead. The portable-data
override keeps recyclable walkthrough state beside the checkout; omit it for
the normal OS app-state location.

Endpoint cleanup is ownership-aware, so an older development process cannot
remove a newer process's project-local control endpoint. If the supervisor is
forcibly killed, relaunch `pnpm tauri dev` to recreate the local endpoint.

The shortcut map is F8 Help, Alt+1 Library, Alt+2 Run, Alt+3 YouTube, Alt+4
Settings, Alt+5 Reports, Alt+6 Help, Alt+M metadata, Alt+S stop-after-current,
Alt+Shift+S stop-now confirmation, and Ctrl+Enter start. The channel is
disabled by default and never executes shell commands. Native pickers,
drag/drop, screen-reader announcements, close/reopen, and live stop-now still
need a real-window acceptance pass.

For installation, platform-specific tool setup, and support boundaries, see
[TOOLS.md](TOOLS.md) and [SUPPORT.md](SUPPORT.md). The native architecture and
retirement record is [python-rust-convert.md](maintainer/python-rust-convert.md).
