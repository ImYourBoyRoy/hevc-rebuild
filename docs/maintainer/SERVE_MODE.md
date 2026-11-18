# Headless server mode (`hevc-rebuild-studio serve`)

Status: stages 1-4 implemented (2026-11-16/17).

## Goal

Run HEVC Rebuild Studio on a box with no desktop session, serve the existing
SvelteKit GUI over HTTP, and let an agent drive the same jobs over MCP while a
person watches them in the browser. There is still one native Rust
implementation: serve mode is a fourth interface beside the GUI, the CLI/JSON
actions and the stdio MCP server, and it must use the same settings, discovery,
work-directory, planning, validation, transaction, report, queue and
stop-control contracts.

## Decisions

### 1. The browser speaks the Tauri protocol; the frontend is not forked

The built GUI (`desktop/build`, adapter-static) imports `@tauri-apps/api` and
four plugins. In serve mode the server serves that same build and injects one
script (`/__hevc/web-shim.js`, source `web_server/web_shim.js`) into
`index.html` before the app boots. The shim installs
`window.__TAURI_INTERNALS__` / `window.isTauri`, which is the contract
`@tauri-apps/api` already calls, so `invoke`, `listen`, `getCurrentWindow`,
`Menu`, the dialog, opener, updater and process plugins keep working without
any change to `+page.svelte` or `src/lib/`. Consequences:

| GUI call | Serve-mode behaviour |
|---|---|
| `invoke("cmd", args)` | `POST /api/invoke/cmd` with the JSON args; same camelCase argument names and the same `Result<T, String>` shape (errors reject with the message string) |
| `listen(event)` (`plugin:event\|listen`) | one `EventSource` on `GET /api/events`; the shim fans events out to registered callbacks |
| `plugin:dialog\|open` / `save` | a server-side path browser modal drawn by the shim over `list_directory` (a server-only command); the browser's own filesystem is never involved |
| `plugin:opener\|open_url` | `window.open` in the browser |
| `plugin:app\|version`, `plugin:updater\|check` | the server's version; updates report "up to date" (a headless install is updated by its package manager) |
| `plugin:window\|*`, `plugin:menu\|*` | inert stubs (a browser tab has no native window or native context menu) |

Poster/artwork previews already travel as command results (`read_local_library_artwork_preview`),
not through an asset-protocol URL, so no protocol handler is needed. Queue,
preferences, credentials, reports and logs stay server-side in the normal app
directories.

### 2. One dispatch table, same functions

`web_server/commands.rs` maps a command name and JSON args onto the existing
`pub(crate)` command functions (the `#[tauri::command]` bodies are plain async
Rust functions). Tauri `State`/`AppHandle` parameters are the only thing that
ties a command to the desktop runtime, so those commands are split into a thin
Tauri wrapper plus a runtime-free `*_with(&RunState, ...)` function that both
the wrapper and the dispatcher call. Stage 2 replaces `AppHandle` in the job
runners with a small `Events` sink (Tauri emit in the desktop app, the event
bus in serve mode), so `start_run`, `scan_folder` and `provision_toolchain` run
through the same code.

### 3. Events

`web_server/events.rs` is an in-process broadcast bus with a monotonically
increasing id and the event name/payload exactly as the desktop emits them
(`run-line`, `run-progress`, `run-resources`, `run-finished`, `repair-line`,
`repair-finished`, `scan-progress`). `GET /api/events` is Server-Sent Events,
chosen over WebSocket because the stream is one-way, reconnects and resumes
(`Last-Event-ID`) by itself, passes through plain reverse proxies and needs no
extra protocol crate. Commands are always `POST`. Several browsers and the MCP
endpoint can listen at once; every subscriber sees the same job.

### 4. Safety model

- Binds `127.0.0.1:8437` by default. A non-loopback address is refused unless
  `--allow-remote` is also given; the server then still requires the token.
- A bearer token is always required (there is no `--no-auth`). It is random
  (256 bit) and printed once at start as a login URL, or supplied with
  `--token-file PATH` / `HEVC_REBUILD_SERVE_TOKEN` for a stable token. The login
  link carries it in the URL fragment (`/#token=…`), which browsers never send;
  the web shim stores it in `sessionStorage`, strips it from the address bar and
  sends `Authorization: Bearer` (fetch-based event stream, `credentials: omit`).
  There are no cookies (a cookie is shared across ports, so it would leak the
  token to any other local service). Static GUI files are public; `/api` and
  `/mcp` need the token. A token file must be mode 0600 on Unix.
  Comparison is constant-time. The token is never written to logs or the
  app-state directory by the server.
- DNS-rebinding and cross-site defence: the `Host` header must be a loopback
  name/address or one listed with `--allowed-host`; state-changing requests
  must be `application/json` (cannot be sent by a plain cross-site form) and,
  when an `Origin` is present, it must match `Host`. Responses carry a strict
  CSP (`default-src 'self'`, `connect-src 'self'`, inline bootstrap scripts
  allowed only by hash) and `X-Content-Type-Options: nosniff`.
- TLS is not implemented in-process. Put the server behind a reverse proxy
  (Caddy, nginx, a Cloudflare tunnel) that terminates TLS and forwards to the
  loopback port; set `--allowed-host` to the public name. Documented in
  `docs/USAGE.md`.
- `--read-only` allows observation only: status, environment check, plans,
  reports, logs, queue reads, searches. Every command is classified in
  `commands.rs` (`Access::Read` / `Access::Write`); unknown or unclassified
  commands are refused, and a test fails when a registered command has no
  classification.
- Secrets never cross the API. `save_/reveal_/remove_` for TMDB/TVDB
  credentials are refused in serve mode (the keyring lives on the server;
  provide credentials with the existing environment variables or the desktop
  app). The API has no password parameter anywhere: privileged setup uses the
  server process's own authority (below).
- Desktop-only commands (`open_file_location`, agent-control reply) answer
  "not available in server mode" or no-op, never silently succeed.

### 5. Privileged setup on a headless box

`pkexec` needs a desktop agent and cannot prompt. For headless provisioning
the plan runner has a third elevation, `Elevation::SudoNonInteractive`
(`sudo -n sh -c ...`), and the server probes a route on every plan or setup
call (`provision::probe_headless_route`): already root (no wrapper),
passwordless sudo (`sudo -n true` succeeds), or none. The plan JSON gains
`elevation` (`root`, `sudo-n`, `none`) and `password_prompts: 0`. With no
route the privileged work is withheld and one `headless-privilege` blocker
names the work and the fix (start as root, or allow passwordless sudo via
`sudo visudo -f /etc/sudoers.d/hevc-rebuild`); `provision_toolchain` then
refuses to start instead of failing mid-run. The password is never requested
over HTTP. The plan stays separate from the runner (`provision.rs` /
`provision_run.rs`); only the `Privilege` (desktop or headless route) passed to
`current_plan_for` and the elevation differ. The sudoers rule must let the user
run `sh` through `sudo -n`; Windows (UAC) has no headless route and reports the
blocker.

### 6. MCP endpoint (stage 3)

`/mcp` serves the existing MCP tool set (the same `McpServer` as `--mcp`) over
streamable HTTP (rmcp's `StreamableHttpService`, JSON responses, in-memory
sessions), behind the same token, Host and Origin checks as the rest of the API.
An agent sends `Authorization: Bearer <token>`. `--read-only` does not offer
`/mcp` at all (403), because its tools start runs, renames and setup.

One job slot, one bus. The MCP server keeps its own job registry; rather than
touch its executors, `mcp_server/shared.rs` shares the server's state with it:

- **Slot:** `insert_job` claims the server's `RunState` slot (`Shared::claim`), so
  an agent's job and a browser's run refuse each other with the usual "already
  active" message; two folder analyses may overlap as in stdio. A job's stop file
  is copied to the slot so the GUI's stop buttons work on it. The slot is released
  when the job reaches a terminal status.
- **Events:** a bridge thread (200 ms) turns registry changes into the events the
  GUI already renders on the bus: `job-started` {runId, kind}, `run-line`,
  `run-progress`, `run-resources`, `run-finished` for media runs; `repair-line` /
  `repair-finished` for setup and helper install; `scan-progress` for folder
  analysis. Each line and the finish are sent exactly once (`lines_total`).
  MCP jobs carry no per-file percentage, so the bar shows queue position.
- **Setup:** `plan_system_setup` / `provision_system` use the headless route
  (`Privilege::Headless`) when hosted by the server, as in section 5. Both tools
  now plan on a blocking thread; before, the stdio server panicked on them ("Cannot
  drop a runtime in a context where blocking is not allowed") because the
  GitHub lookup uses a blocking client.

Following a job you did not start: `active_job` (Tauri command and server
command) reports the slot holder `{id, kind}`. The GUI adopts a media run from
`job-started` or, at boot, from `active_job` (`run-controller.ts`
`adoptActiveRun`, `app-boot.ts`): it takes the run id, opens the Run page and
renders the live events. It never replaces a run it started or is starting, and
ignores setup/analysis kinds (those have their own views). The desktop app uses
the same code path, where `active_job` is simply always empty.

Catching up (stage 3 follow-up). A tab that opens mid-run gets the run's recent
output: the event bus keeps, per run id, the newest 240 lines, the latest
`run-progress` and `run-resources` (dropped when the run finishes), and a new
`GET /api/events` connection without `Last-Event-ID` starts with a synthetic
`job-started` for the active job followed by that replay, then the live stream
(taken under one lock, so nothing is lost or repeated). A reconnect with
`Last-Event-ID` resumes from the id as before. Every job in the slot now carries a
kind (`media_run`, `integrity_check`, `folder_analysis`, `system_provision`,
`artwork_cache_purge`), including GUI-started ones, so a reloaded tab follows its
own run. Agent jobs report per-file detail: the executors store a `ProgressDetail`
(phase, file/queue percent, speed, ETA, elapsed, input/output/estimated bytes) on
the job, which `get_job` returns as `progress_detail` and the bridge sends as
`run-progress`. The MCP job already saves queue results to the queue file, so a
window that followed a run reloads the queue as each new file starts and when
the run ends (`run-controller.ts`).

Verifying real privileged setup. `tools/serve-setup-sandbox.sh` runs the server as
a namespace root (bubblewrap) on private copies of `/var/lib/apt`, `/var/lib/dpkg`
and `/etc/apt`, plans (route `root`, 0 prompts), runs `provision_toolchain` for real
(`apt-get update`, streamed as `repair-line`, `repair-finished` success), re-plans
(`nothing_to_do`) and fails if the host's apt state changed. The `sudo -n` route
cannot be exercised for real on a host without passwordless sudo; its command
line is covered by the command-builder tests.

## Stages

1. **Done (stage 1):** `serve` subcommand, config + safety checks, token auth, host/origin
   checks, static GUI (embedded or `--web-root`), shim, command dispatch with
   Read/Write classification and `--read-only`, SSE bus, server-side path
   browser, tests for auth/routing/events/shim.
2. **Done (stage 2):** `Events` sink in the job runners; `start_run`,
   `scan_folder` and `provision_toolchain` served (Write class, refused by
   `--read-only`) with live events on the bus; headless `sudo -n` route and
   plan blocker. Known gap: a browser tab only follows jobs it started itself.
   The GUI ignores events for run ids it did not start, and a freshly opened tab
   does not learn the active job, so a job started by another client (an agent,
   a second tab) is not shown until the GUI can adopt an active run. That needs
   an `active_job` command (kind and run id) plus a small run-controller change;
   done in stage 3.
3. **Done (stage 3):** `/mcp` over streamable HTTP sharing the job slot and bus
   (section 6); `active_job` plus GUI adoption of a job another client started;
   fix for the stdio `plan_system_setup`/`provision_system` panic.
4. **Done (stage 4):** a `desktop` cargo feature (default on) gates Tauri, its
   plugins, the embedded GUI and the agent-control channel; `--no-default-features`
   builds the same executable as a headless server with no webkit/GTK; `--root`
   confinement; a DOM right-click menu in the shim (section 7).

### 7. Stage 4 details

**Headless build.** `cargo build --no-default-features --bin hevc-rebuild-studio`
links only libc, libm and libgcc (release-profile binary: 15.7 MB) (5 shared libraries, against 124 for the window
build, none of them GUI libraries; the dependency graph has no tauri, wry, tao,
webkit or gtk). Everything non-GUI is shared: `#[tauri::command]` is applied only with
the feature (`cfg_attr`), the wrappers that take `State`/`AppHandle` exist only with
it, and blocking work goes through `runtime::spawn_blocking` (Tauri's runtime in the
window build, Tokio's in the headless one). Without `desktop` there is no embedded GUI: the server
serves a folder, found in this order: `--web-root`, `web/` or
`../share/hevc-rebuild-studio/web` next to the executable, then (window build only)
the embedded copy. `tools/package-headless.sh` builds the frontend and the release
binary and writes `hevc-rebuild-studio-headless.tar.gz` with that layout. Running the
binary with no arguments prints that it has no window; `agent-control` is
window-build only. CI builds, lints, tests the headless configuration and fails if
the binary links a GUI library.

**`--root DIR` (repeatable).** A guard rail for the browser API, not a sandbox. The path
browser starts in the first root and cannot go above it (symlinks that lead out are
hidden), `default_media_root` is the first root, and before a command runs every media or
output path in its arguments (table in `web_server/scope.rs`: roots, selected files,
output folders, per-file override keys, queue selection, rename roots, log export
destination, ...) must resolve inside a root, following symlinks and rejecting `..` and
relative paths; a file to be created is judged by where it would land. Not confined:
tool paths (a token holder can point at any executable), data inside the queue
document, and the MCP tools, which take paths anywhere, so `/mcp` is not offered with
`--root` unless `--allow-mcp-with-root` is also given. For real isolation run the server
as a dedicated user or in a container.

**Right-click menu.** The GUI's native context menu is built with the Tauri `Menu`
API; the shim records the items and their channel handlers, draws them as a menu at the
pointer, runs the chosen action through its channel (once, in order), runs Cut / Copy /
Paste / Select all through the browser, and closes on a choice, click outside or Escape.

### 8. Cross-platform verification (2026-11-17)

Native runs of both build configurations (window build and `--no-default-features`):

| Machine | Window build | Headless build | Notes |
|---|---|---|---|
| Linux laptop (Ubuntu 26.04) | 479 | 477 | media tools present (`HEVC_REQUIRE_MEDIA_TOOLS=1`) |
| Ubuntu server (builder) | 479 | 477 | no ffmpeg; media tests skip; headless links 5 libs, 0 GUI |
| macOS Tahoe SIP-off guest | 480 | 478 | all media tools installed there through the server (Homebrew), `HEVC_REQUIRE_MEDIA_TOOLS=1`; headless links only system frameworks |
| Windows Server 2025 (native, GNU) | 468 | 466 | ffmpeg and mkvtoolnix present, media tests ran |

Live `serve` smoke tests on macOS and Windows: token, shim, environment check, plan, path
browser. They found real defects, fixed with tests first: `default_media_root` read only
`HOME` (unset on Windows, so the GUI's boot call failed there); Windows canonicalisation
returned `\\?\C:\...` paths in the browser; tests assumed Unix separators, `/etc` and
installed media tools. A server running elevated on Windows (High integrity, or SYSTEM)
now gets the `root` route; on Windows without elevation the plan still reports a blocker.
Real privileged runs: Linux apt catalog refresh as namespace root
(`tools/serve-setup-sandbox.sh`), Windows helper install through the server
(`hdr10plus_tool` 1.7.2), Homebrew provisioning through the server on the disposable
macOS guest (ffmpeg, mediainfo, mkvtoolnix and the HDR10+ helper; re-plan `nothing_to_do`). Not covered: apt package installs (the sandbox cannot write `/usr`), winget
package installs (winget is per-user and absent for SYSTEM), the `sudo -n` route on a host
with passwordless sudo.

## Open questions for the owner (defaults chosen, change on request)

- Port 8437, loopback only, remote binding needs `--allow-remote`.
- TLS through a reverse proxy only (no in-process TLS).
- Resolved in stage 4: the headless build has no webkit/GTK dependency at all.
- Credentials: refuse over the API and use env vars/desktop app instead.
