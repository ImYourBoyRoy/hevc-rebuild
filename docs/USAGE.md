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
```

For the source engine, run `python -m hevc_rebuild` on systems where the
standalone engine is being used. `--setup-only` installs or checks the required
toolchain and exits without processing media. Add `--no-dependency-repair` for
a read-only check.

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
  GUI, JSON CLI, and MCP use the native Rust analyzer; Python is not required.
- **Integrity check** inspects container structure (Quick) or decodes every
  stream (Full) without writing output. Native; Python is not required.
- **Metadata-only MKV** remuxes tracks into a new MKV without video encoding.
- **Short test** performs a bounded test encode.
- **Full rebuild** encodes selected video tracks and copies audio, subtitles,
  attachments, chapters, and other selected streams according to the reviewed
  plan.
- **YouTube export** creates a separate upload copy and never replaces the
  source.

For installation, platform-specific tool setup, and support boundaries, see
[TOOLS.md](TOOLS.md) and [SUPPORT.md](SUPPORT.md). The complete Python-to-Rust
ownership ledger is [python-rust-convert.md](maintainer/python-rust-convert.md).
