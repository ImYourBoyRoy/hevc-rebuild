# Project structure

HEVC Rebuild Studio contains a compatibility engine, a native Rust boundary,
and a Svelte desktop interface. The repository root is reserved for product
entry points, build metadata, and the canonical project instructions.

## Start here

| Path | Purpose |
| --- | --- |
| [`README.md`](../README.md) | Human installation, usage, build, and release guide. |
| [`AGENTS.md`](../AGENTS.md) | Repository operating contract and safety rules. |
| [`docs/USAGE.md`](USAGE.md) | GUI, CLI, MCP, output, and recovery workflows. |
| [`docs/TOOLS.md`](TOOLS.md) | External media tools and platform setup. |
| [`docs/PLATFORM-SUPPORT.md`](PLATFORM-SUPPORT.md) | Cross-platform feature contract and native acceptance matrix. |
| [`docs/SUPPORT.md`](SUPPORT.md) | Troubleshooting, support boundaries, and licensing contact. |
| [`docs/maintainer/`](maintainer/) | Maintainer architecture, validation, dependency, and migration notes. |

## Runtime code

| Path | Responsibility |
| --- | --- |
| `hevc_rebuild/` | Python compatibility engine, direct CLI, reports, metadata, and fallback paths. |
| `desktop/src/` | Svelte UI, queue workflows, browser-safe formatting, and UI contracts. |
| `desktop/src-tauri/src/` | Rust native services, typed GUI/CLI/MCP boundary, tool inspection, queue persistence, scan, run planning, and native process work. |
| `desktop/src-tauri/` | Tauri manifest, Rust lockfile, icons, and native packaging configuration. |
| `desktop/tests/` | Isolated browser/component contracts. These fixtures do not access native dialogs, credentials, or user media. |
| `tests/` | Python behavior, safety, parity, and synthetic-fixture tests. |
| `tools/` | Explicitly invoked fixture, benchmark, and inspection utilities. |

## Human-facing and release files

| Path | Responsibility |
| --- | --- |
| `docs/` | Short user, platform, release, and maintainer documentation. |
| `desktop/README.md` | Desktop-specific development and native boundary notes. |
| `App/` | Local cross-platform artifact drop for verified host-built packages. It is not source code; keep it at the repository root. |
| `VERSION`, `LICENSE`, `SHA256SUMS` | Version, license, and integrity metadata. Regenerate `SHA256SUMS` after source or documentation changes. |
| `docs/maintainer/` | Current validation, architecture, measurement, dependency, and Python/Rust migration records. |

Local `MEMORY.md`, dated audit evidence, and personal task lists are excluded
from Git and kept in the sibling local notes archive. They are session
continuity material, not application documentation. New user instructions
belong in `README.md` or `docs/`; new architectural ownership belongs in
`AGENTS.md` or `docs/maintainer/`.

## Generated and local-only paths

Do not edit or publish `desktop/node_modules/`, `desktop/.svelte-kit/`,
`desktop/build/`, `desktop/output/`, `desktop/src-tauri/target/`, browser
evidence directories, Python caches, or application sidecar data. They are
recreated by the documented install/build/test commands and are ignored where
appropriate.

## Data flow

1. The GUI, JSON CLI, or MCP caller enters through the Rust boundary.
2. Rust validates paths, settings, tool capabilities, queue state, and output
   destinations, then returns typed data to the Svelte UI or starts an owned
   job.
3. Python remains the compatibility/run-orchestration layer for stages that
   have not reached parity; it is tracked file by file in the migration ledger.
4. Reports and retained `.error.mkv`/`.error.log` pairs are user-visible
   outputs, not source files. A failed output remains marked unvalidated.
