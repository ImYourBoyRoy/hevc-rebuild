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

### What validation checks

Before a rebuild replaces the original, the output must pass these checks:

- stream layout, codecs, resolution, pixel format, dispositions, and chapters;
- HDR signaling per video track (and Dolby Vision / HDR10+ rules where chosen);
- container duration, plus start/duration drift of audio and subtitle tracks;
- compressed payload hashes of every stream that was copied rather than encoded;
- **frame count of every re-encoded video track** against its source. The encode
  keeps one output frame per input frame, so a track that comes out short (or
  padded) by more than a few frames (0.01% of the track, minimum 3) is refused.
  Container duration alone cannot catch this, because copied audio keeps the
  file's overall duration intact. Source packets flagged as decoder preroll
  (discard) never become frames and are not counted, so clips cut with a stream
  copy are not refused for it.

Validation proves the output is structurally sound, complete, and carries the
same tracks. A separate **picture quality check** then compares how it *looks*
(next section). The original is kept beside the file in a hidden
`.hevc-rebuild-originals` folder after a successful in-place rebuild (see "Where
originals are kept" below).

### Picture quality check

Before a rebuilt file replaces its original, the app compares sampled scenes of
the new file with the source using SSIM (the whole clip when it is under a
minute, otherwise six 6-second windows through the middle of the file). Both
sides are converted to the same 10-bit format, and when the rebuild deinterlaced
the picture the source is deinterlaced the same way first, so interlacing is not
scored as damage. The report records the average and lowest score.

Settings (Run page, **HDR & stream safety**; or `quality.ssim_gate` and
`quality.min_ssim` in the config):

- **Enforce** (default): a file whose average SSIM is under the minimum (0.95 by
  default), or whose worst window is more than 0.03 under it, is *not* installed.
  The original stays untouched, the file is reported as failed, and a batch
  continues with the next file. If the check cannot run, the file is also kept
  as it was.
- **Report only**: measure and record, but always install.
- **Off**: no measurement.

What the score means. On the sample library at CRF 22 (the default is now the
slightly higher-quality CRF 21; the scores below were measured at 22), slow,
10-bit, good encodes scored 0.977 to 0.993 (clean CGI 0.993, live action
0.988 to 0.990, hand-drawn film 0.979). A deliberately poor encode of clean CGI
(CRF 40) still scored 0.965, so SSIM is a safety net against broken output (wrong
filter, frames out of sync, bitrate collapse), not proof that nothing is
visible: CRF choice remains the main quality control. Very grainy sources can
score lower without looking worse; for those, lower the minimum or use report
only. Raise the minimum (for example 0.975) for a stricter check.

### Choosing quality, preset and tune

The Run page explains each control in place (select the **i** beside it). In
short:

- **Quality (CRF)** is one dial for the whole video. Lower keeps more detail and
  makes bigger files; higher discards fine detail. The default is **21**
  (recommended). 18-20 is very high quality for archiving, 23-24 gives noticeably
  smaller files that still look excellent on most films, 25-28 is a space saver
  that softens dark or grainy scenes, and anything above 28 is for previews.
- **Preset** trades time for size. **slow** is the recommended balance; slower
  presets add a small saving for a lot of time.
- **Content tune**: **Automatic** applies animation tuning only when the file is
  reliably known to be animated and never turns on film grain. **Normal** has
  no special tuning. **Film grain** keeps the grain texture and makes files much
  larger: in a test on a dark 4K film clip (8 seconds, CRF 21, preset slow,
  10-bit), Normal produced 10.5 MB, Film grain produced 33.4 MB (about 3.2x as
  much) and Normal's SSIM was the higher of the two. On the same clip CRF 24
  produced 5.8 MB (SSIM 0.968 against 0.969 at CRF 21), so raising CRF is the
  far better lever for a bigger saving than changing the tune. If a dark or grainy film
  barely shrinks, check that Film grain is not selected for it (the per-file
  override in the file settings also offers it).
- The app warns beside the settings when Film grain is chosen or the CRF is at
  an extreme, so a surprising choice is explained before a long run.

### What is carried over

Audio, subtitles, chapters, and attachments are copied without re-encoding.
An embedded cover (a Matroska `cover.jpg` attachment or an MP4 cover) is
re-attached to the new file as a `cover.jpg` attachment, converted to JPEG when
needed; a poster you choose in the Library replaces it. Data streams such as an
MP4 timecode track cannot be stored in Matroska, so they are not copied; the run
says so in its log and in the report message. Selecting a data stream by hand is
rejected before encoding starts.

If a validated output is installed but the operating system cannot confirm that
the final directory change reached disk, the run still counts as successful and
the report message carries a warning; the output and the backup are both
intact.

## CLI

Use the installed executable when available:

```bash
hevc-rebuild-studio cli "/media/Movies" --recursive --info-only
hevc-rebuild-studio cli "/media/Movies" --recursive --output-dir "/media/Rebuilt"
hevc-rebuild-studio json environment
hevc-rebuild-studio json provision-plan
hevc-rebuild-studio json provision --confirm
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

### Rebuilding a whole library unattended

For "here is my media folder, rebuild everything and tell me how it went", an
agent uses two tools instead of assembling a run by hand:

1. `plan_library_batch` (read-only) lists every file a full rebuild would
   process, records each file's size and modification time, and forecasts disk
   use per volume. Originals are kept in `.hevc-rebuild-originals`, so disk use goes up as the
   batch runs; the forecast states the break-even ratio (the largest
   output-to-source size the batch can tolerate before reaching the protected
   floor). The free-space floor defaults to the larger of 10 GiB and 5% of the
   volume (never more than a quarter of it); `reserve_gib` overrides it. The
   result includes `show_the_user`, a plain-language plan the agent should show
   before asking for a go-ahead.
2. `start_planned_batch` takes only the `plan_id`. It is refused if the plan is
   unknown, older than six hours, already used, or if any planned file changed
   since planning, and it is refused (without losing the plan) if free space is
   already below the floor.

While it runs, `get_job` returns a `batch` object with live per-file results,
bytes written and originals retained, free space now, an ETA, and a
closing `report`. The engine enforces the floor itself (preflight plus live
projection), and the run pauses by itself when free space reaches the floor
(status `paused`) or when three files fail in a row. Isolated failures do not
stop the library: the status is `completed_with_failures` and each reason is in
`batch.failures`. To continue after a pause, free some space and plan the same
folder again; files that already have a backup are skipped. Nothing is
ever deleted automatically, including backups.

`list_jobs` lets an agent find a job again after losing its context. Backup folders
(`.hevc-rebuild-originals`, and `old` from earlier versions) hold this app's
originals, so the batch planner never includes them, and discovery always skips
the hidden folder. Other MCP and command-line runs also skip a legacy `old`
folder unless `filename_exclude_patterns` is passed explicitly. A full rebuild started through `start_media_run` with a zero disk
reserve is given the same default floor.

## Headless server

`hevc-rebuild-studio serve` runs the app without a desktop session and serves
the same GUI to a browser. It is a fourth interface over the same Rust
commands; the design and safety model are in
[`maintainer/SERVE_MODE.md`](maintainer/SERVE_MODE.md).

```bash
hevc-rebuild-studio serve                       # 127.0.0.1:8437, new token each start
hevc-rebuild-studio serve --read-only           # observe only
hevc-rebuild-studio serve --token-file ~/.config/hevc-serve-token --web-root desktop/build
```

The server prints a login link (`http://127.0.0.1:8437/?token=...`). Opening it
sets an `HttpOnly`, `SameSite=Strict` session cookie and removes the token from
the address bar; scripts and agents send `Authorization: Bearer <token>`. There
is no way to turn authentication off.

- It listens on loopback only. `--host 0.0.0.0` is refused unless
  `--allow-remote` and at least one `--allowed-host NAME` are also given. Put a
  reverse proxy (Caddy, nginx, a tunnel) in front for TLS; the server marks the
  cookie `Secure` when the proxy sends `X-Forwarded-Proto: https`.
- `--read-only` refuses every command that changes state (preferences, queue,
  renames, artwork, reports) and still serves environment checks, plans,
  reports, logs and searches.
- Metadata credentials are never sent over the API; configure them with the
  desktop app or the environment variables, not through the browser.
- File and folder pickers become a browser dialog that browses the *server's*
  disk, so paths always mean paths on the machine that runs the encodes.
- Runs, scans and one-pass setup started from the browser use the same engines
  as the desktop and stream their events live. Setup cannot show a password
  prompt: run the server as root, or allow the user passwordless sudo
  (`sudo -n`). Without either, the setup plan reports a `headless-privilege`
  blocker with the fix and nothing is installed.
- An agent connects to `http://HOST:PORT/mcp` (streamable HTTP MCP) with the
  same token. A job it starts shares the single job slot and appears live in
  every open browser tab, which switches to the Run page by itself; a run
  started in the browser makes the agent's start fail with "already active".
  `--read-only` servers do not offer `/mcp`.

  ```json
  {
    "mcpServers": {
      "hevc-rebuild": {
        "type": "http",
        "url": "http://127.0.0.1:8437/mcp",
        "headers": { "Authorization": "Bearer <token>" }
      }
    }
  }
  ```
- A tab that opens while a run is active follows it, including its recent
  output, progress and ETA; the saved queue refreshes as the run advances.
- `--root DIR` (repeatable) keeps the browser API inside those folders: the path
  browser starts there and cannot leave, and media/output paths in commands are
  checked. It is a guard rail, not a sandbox (tool paths and the MCP tools are not
  confined, so `/mcp` is off with `--root` unless `--allow-mcp-with-root`). Use a
  dedicated user or a container for real isolation.
- Right-click in the browser opens the same menu as the desktop window.

### Headless install (no webkit/GTK)

`cargo build --release --no-default-features --bin hevc-rebuild-studio` builds the
server without any GUI library; `tools/package-headless.sh` also builds the
frontend and writes `dist/hevc-rebuild-studio-headless.tar.gz` (`bin/` and
`share/hevc-rebuild-studio/web/`). Unpack anywhere and run
`bin/hevc-rebuild-studio serve`; it finds `web/` by itself, or pass `--web-root`.
This build has no window, so run it with `serve`, `--mcp`, `cli` or `json`.

## Modes

- **Check only** reads media and reports compatibility without writing output.
  GUI, JSON CLI, and MCP use the native Rust analyzer.
- **Integrity check** inspects container structure (Quick) or decodes every
  stream (Full) without writing output. It is native.
- **Metadata-only MKV** remuxes tracks into a new MKV without video encoding.
- **Short test** performs a bounded test encode at the same CRF as the full run (21 by default, or your
  setting), so it previews the real result; `--test-crf` overrides it for the CLI.
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

## Where originals are kept

After a validated in-place rebuild the original moves into a hidden
`.hevc-rebuild-originals` folder beside it, so media servers and file browsers
do not list backups as library content (Windows also gets the hidden attribute).
Earlier versions used a visible `old/` folder. Those libraries keep working: a
file with a backup in either folder is treated as already rebuilt and skipped,
and an `old/` folder is still excluded from scans by default. New backups only go
to the hidden folder. The app never moves, merges or deletes existing backups;
when you are sure you no longer need a backup, delete it yourself.
