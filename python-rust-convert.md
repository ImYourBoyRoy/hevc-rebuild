# Python → Rust conversion ledger

This is the file-level retirement ledger for the Python media engine. It is
more concrete than the architectural summary in
[`RUST_MIGRATION.md`](RUST_MIGRATION.md): every production Python source file is
listed with its role, known callers, Rust destination, current state, and the
gate required before removal. Every repository-owned Python test and helper is
also listed below so a future conversion does not lose a caller, fixture, or
manual tool entry point.

## Rules

- Python remains the behavioral reference until Rust matches success, skip,
  failure, cancellation, report, recovery, and rollback behavior.
- A Rust module is not complete because it compiles or passes a happy-path
  fixture.
- Remove a Python file only after every caller has moved, parity fixtures cover
  the supported boundary, and native packaged runs pass on every claimed target.
- FFmpeg, FFprobe, MediaInfo, MKVToolNix, and HDR10+ remain explicit external
  toolchain contracts regardless of which language launches them.

## Entry points and package files

| Python file | Role and callers | Rust destination | State and removal gate |
| --- | --- | --- | --- |
| `hevc_rebuild.py` | Thin installed-script entry point for shells and Windows `py`. | `desktop/src-tauri/src/main.rs` and the packaged binary. | Partial. Retire only when the standalone engine entry point no longer serves a supported direct workflow. |
| `hevc_rebuild/__init__.py` | Package version and identity; imported by `app.py`. | Cargo/Tauri/package metadata. | Keep until the Python package is removed; versions must remain aligned. |
| `hevc_rebuild/__main__.py` | `python -m hevc_rebuild` dispatcher; calls `app.main`. | Rust CLI routing. | Partial. Retire after direct CLI media orchestration is Rust-owned. |
| `hevc_rebuild/model.py` | Dataclasses for toolchains, streams, profiles, metadata, and validation results; imported throughout the engine. | Serde request/response structs across Rust modules. | Partial. Replace one contract at a time after JSON/schema parity is stable. |

## Workflow and policy files

| Python file | Role and callers | Rust destination | State and remaining work |
| --- | --- | --- | --- |
| `hevc_rebuild/app.py` | CLI parsing, setup, discovery orchestration, per-file modes, progress, reports, retries, retained-output handling, and direct transactions. Called by `__main__.py`, `hevc_rebuild.py`, and the native `engine_command`. | `lib.rs`, `media_queue.rs`, `reports.rs`, `transactions.rs`, future run coordinator. | Python-owned coordinator. Rust owns bounded scan, planning, validation, CSV append, and commit slices; migrate run policy and report-value decisions before removal. |
| `hevc_rebuild/bridge.py` | Legacy JSON bridge for environment, scans, defaults, reports, and package/helper actions. | `environment.rs`, `media_scan.rs`, `settings.rs`, `reports.rs`, `dependencies.rs`, `mcp_server.rs`. | Native application routes exist. Retire after direct compatibility callers are migrated or explicitly dropped. |
| `hevc_rebuild/config.py` | Defaults, TOML parsing, validation, precedence, and CLI settings application. Called by `app.py` and `bridge.py`. | `settings.rs`, `settings_arguments.rs`, native app profile. | Shared/parity-checked. Direct CLI settings and folder-config behavior remain. |
| `hevc_rebuild/deps.py` | Platform tool discovery/repair, package-manager setup, HDR10+ download/verification, x265 capability checks, and toolchain resolution. Called by `app.py`, `bridge.py`, and `environment.py`. | `environment.rs`, `dependencies.rs`, future cross-platform setup module. | Split. Rust owns native inspection and Linux helper installation; Python retains direct cross-platform setup fallback. Native setup parity is the retirement gate. |
| `hevc_rebuild/discovery.py` | Bounded media discovery, recursion, filename filters, selected-file checks, and candidate handling. Called by `app.py`, `youtube.py`, `bridge.py`, and `probe.py` adapters. | `media_scan.rs`, `output_targets.rs`. | Native app path exists; direct CLI fallback remains until edge-case parity is complete. |
| `hevc_rebuild/content.py` | NFO/MediaInfo genre evidence and automatic/per-file content-tune selection. Called by `app.py`, `bridge.py`, and `encode.py`. | `content.rs`, `media_scan.rs`. | Split. Rust app path exists; direct NFO precedence and malformed-input parity remain. |
| `hevc_rebuild/calibration.py` | Workload-keyed x265 calibration, bounded candidate encodes, pressure checks, and cache persistence. Called by `app.py` and `encode.py`. | `calibration.rs`, `resource_monitor.rs`. | Split. Rust owns app-launched calibration; direct Python and report/retry policy remain. |
| `hevc_rebuild/resources.py` | CPU, memory, swap, PSI, disk, temperature, process, and resource snapshots. Called by `app.py`, `encode.py`, `calibration.py`, `environment.py`, and `bridge.py`. | `resource_monitor.rs`, `resource_plan.rs`, `runtime_guard.rs`. | Split. Linux native parity is covered by focused contracts; Windows/macOS adapters and direct-run retirement remain open. |
| `hevc_rebuild/encode.py` | FFmpeg/x265 argument construction, process execution, progress, memory/disk guards, retry plans, and cleanup. Called by `app.py`, `calibration.py`, and `youtube.py`. | `encode_plan.rs`, `encode_supervisor.rs`, `resource_plan.rs`, `runtime_guard.rs`. | Split. Rust owns app-launched planning/supervision; Python still drives direct runs, progress interpretation, retry/report flow, and fallback cleanup. |

## Media, validation, and output files

| Python file | Role and callers | Rust destination | State and remaining work |
| --- | --- | --- | --- |
| `hevc_rebuild/probe.py` | FFprobe/MediaInfo execution, stream selection, color/HDR profile construction, and native-profile adapters. Called by `app.py`, `bridge.py`, `validate.py`, `youtube.py`, and `metadata.py`. | `media_scan.rs`, `validation.rs`, typed profile adapters. | Split. Rust app profiling exists; direct CLI and unusual probe-shape fallback remain. |
| `hevc_rebuild/hdr.py` | HDR10+ pipe verification/extraction, Dolby Vision policy, and Python fallback requests. Called by `app.py`, `validate.py`, and `encode.py`. | `hdr_metadata.rs`, `validation.rs`. | Split. Rust owns app-launched checks; Python reinsertion/orchestration and direct fallback remain. |
| `hevc_rebuild/metadata.py` | Matroska XML/tag/chapter mutation, NFO mapping, poster attachment, and expected-tag reads. Called by `app.py`, `validate.py`, and `content.py`. | `metadata_text.rs`, `library_nfo.rs`, future Matroska mutation module. | Python-owned for output mutation and final tag/poster expectations. Remove only after byte-safe remux/tag/artwork parity. |
| `hevc_rebuild/validate.py` | Copied-stream fingerprints, timing/disposition/chapter checks, encoded-track codec/color/HDR checks, metadata validation, and final acceptance. Called by `app.py`, `youtube.py`, `metadata.py`, and `hdr.py`. | `packet_verify.rs`, `validation.rs`, `hdr_metadata.rs`. | Split. Rust owns most native stream/payload/HDR checks; Python still owns final metadata/poster and direct-run gates. |
| `hevc_rebuild/transactions.py` | No-overwrite output install, source backup, rollback, and validated transaction adapter. Called by `app.py` and native JSON adapters. | `transactions.rs`. | Split with parity fixtures. Keep Python until direct CLI and native transaction/report behavior converge. |
| `hevc_rebuild/youtube.py` | Source-aware dimensions/rates, HDR loss policy, FFmpeg export, validation, separate-output install, and per-file reports. Called by `app.py` and `bridge.py`. | `youtube.rs`, `media_scan.rs`, future native batch coordinator. | Split. Rust owns policy/argument/temporary-output helpers; Python retains batch outcomes, reports, upload-copy orchestration, and direct fallback. |

## Support and utility files

| Python file | Role and callers | Rust destination | State and remaining work |
| --- | --- | --- | --- |
| `hevc_rebuild/environment.py` | Read-only OS, executable, version, encoder, and readiness report. Called by `bridge.py` and direct `app.py` setup. | `environment.rs`. | Split. Keep schema parity; remove after direct CLI uses native inspection without a Python bridge. |
| `hevc_rebuild/ui.py` | Terminal cards, progress rendering, warnings, and batch summaries. Called by `app.py`, `youtube.py`, and `encode.py`. | Rust CLI progress/report formatting and the Svelte run console. | Python direct-CLI support. Retire only when Rust preserves useful progress and diagnostics. |
| `hevc_rebuild/util.py` | Errors, process groups, bounded command execution, JSON boundaries, filesystem durability, report/log writers, and formatting. Called by nearly every Python module. | `reports.rs`, `process_output.rs`, `transactions.rs`, `stop_control.rs`, and shared Rust utilities. | Cross-cutting. Migrate contracts first; delete only after all Python callers are gone. |

## Test and repository helper files

These files are invoked by `python3 -m unittest discover -s tests` or directly
as noted. They are not runtime application modules, but they are part of the
conversion surface because they define parity fixtures, safety gates, and
manual helper workflows.

| Python file | Role and callers | Rust destination | State and removal gate |
| --- | --- | --- | --- |
| `tests/test_calibration.py` | Calibration eligibility, candidate plans, cache, and resource-pressure tests; called by unittest discovery. | `calibration.rs`, `resource_plan.rs`, `resource_monitor.rs` tests. | Active parity coverage; keep until direct and native calibration agree. |
| `tests/test_config_content.py` | Settings precedence, content-tune selection, artwork compatibility, HDR/Dolby Vision policy; unittest discovery. | `settings.rs`, `content.rs`, `hdr_metadata.rs`. | Active parity coverage. |
| `tests/test_content_native.py` | Python/Rust content-tune decisions for NFO and per-file overrides; unittest discovery. | `content.rs` tests. | Active parity gate. |
| `tests/test_copy_hash.py` | Copied-stream hash and Rust fingerprint parity; unittest discovery. | `packet_verify.rs`, `validation.rs` tests. | Active parity gate. |
| `tests/test_cross_path_parity.py` | Synthetic direct-Python versus Rust-helper scan, encode, and transaction fixtures; optional native helper caller. | Rust native integration fixtures. | Keep until the helper is rebuilt and both paths pass. |
| `tests/test_deps_updates.py` | APT allowlist, exact confirmation, simulation, JSON bridge, and read-only resolver tests; unittest discovery. | `dependencies.rs`, `environment.rs`, native JSON tests. | Active safety coverage; extend for winget/Homebrew before native setup migration. |
| `tests/test_discovery.py` | Filename filters, recursion, exclusions, and candidate selection; unittest discovery. | `media_scan.rs`, `output_targets.rs` tests. | Active parity coverage. |
| `tests/test_durability.py` | Directory sync and durability error propagation; unittest discovery. | `transactions.rs`, `reports.rs` tests. | Keep until native and Python durability contracts converge. |
| `tests/test_encode_integration.py` | Generated-media FFmpeg argument and stream-copy integration checks; unittest discovery. | `encode_plan.rs`, `packet_verify.rs` integration tests. | Requires local media tools; retain for real command coverage. |
| `tests/test_encode_plan_parity.py` | Multi-video Python/Rust FFmpeg argument vectors; unittest discovery. | `encode_plan.rs` tests. | Active parity gate. |
| `tests/test_encode_supervisor.py` | Native encoder cancellation and resistant-descendant process handling; unittest discovery. | `encode_supervisor.rs`, `process_output.rs`. | Active native safety gate. |
| `tests/test_extract_clip.py` | Safe bounded fixture extraction command and duration parsing; unittest discovery. | Repository helper contract, not an application Rust module. | Keep while `tools/extract_clip.py` is supported. |
| `tests/test_hdr_native.py` | HDR and Dolby Vision Python/native policy parity; unittest discovery. | `hdr_metadata.rs`, `validation.rs`. | Active parity gate. |
| `tests/test_logic.py` | Primary-video selection, disposition, and pixel-format policy; unittest discovery. | `media_scan.rs`, `encode_plan.rs`. | Active policy coverage. |
| `tests/test_metadata.py` | Matroska tags, NFO mapping, release-group cleanup, artwork, and metadata validation; unittest discovery. | `metadata_text.rs`, `library_nfo.rs`, future Matroska mutation module. | Keep until mutation and tag parity are complete. |
| `tests/test_metadata_native.py` | Python/Rust metadata text-cleaning parity; unittest discovery. | `metadata_text.rs` tests. | Active parity gate. |
| `tests/test_metadata_only_integration.py` | Generated MKV metadata-only remux and validation path; unittest discovery. | Native metadata-only integration fixtures. | Requires local MKVToolNix; retain through migration. |
| `tests/test_output_destinations.py` | Output mirroring, overrides, collision planning, and no-create planning rules; unittest discovery. | `output_targets.rs` tests. | Active safety coverage. |
| `tests/test_platform_metadata.py` | Platform resolver branches plus metadata and poster behavior; unittest discovery. | `environment.rs`, `dependencies.rs`, metadata modules. | Extend for macOS/Windows native setup before removing Python fallback. |
| `tests/test_process_safety.py` | Python process-group cleanup after parent/descendant exit; unittest discovery. | `process_output.rs`, `encode_supervisor.rs`. | Active safety coverage. |
| `tests/test_progress_view.py` | CLI progress rendering and GUI progress JSON emission; unittest discovery. | Rust CLI/report formatting and Svelte run console. | Keep until progress contracts are native-owned. |
| `tests/test_report_safety.py` | Private report/log identity, schema, locking, and append safety; unittest discovery. | `reports.rs`. | Active parity and privacy gate. |
| `tests/test_resource_compatibility.py` | Historical encode resource-helper return compatibility; unittest discovery. | `resource_plan.rs`, `resource_monitor.rs`. | Retire only after downstream callers no longer use compatibility aliases. |
| `tests/test_resources.py` | Memory, swap, disk, retry, guard, and resource-plan behavior; unittest discovery. | `resource_plan.rs`, `runtime_guard.rs`, `resource_monitor.rs`. | Active parity coverage. |
| `tests/test_retained_output.py` | Late-failure `.error.mkv`/`.error.log` retention, source protection, collision handling, and subtitle-timing errors; unittest discovery. | `transactions.rs`, native run coordinator. | Required safety gate for any output-transaction migration. |
| `tests/test_stop_control.py` | Stale, symlink, inode, ownership, and finish-current-file stop markers; unittest discovery. | `stop_control.rs`. | Active safety coverage. |
| `tests/test_stream_selections.py` | Per-file stream-map validation and selection policy; unittest discovery. | `media_scan.rs`, `run_arguments.rs`, `encode_plan.rs`. | Active parity coverage. |
| `tests/test_transaction_safety.py` | No-overwrite install, rollback, backup, and operation-time failure fixtures; unittest discovery. | `transactions.rs`. | Required before Python transaction removal. |
| `tests/test_utility_boundaries.py` | JSON, finite-number, CSV, filesystem, and bounded utility contracts; unittest discovery. | Shared Rust utility modules. | Cross-cutting active coverage. |
| `tests/test_validation_native.py` | Output validation, replaceable-cover policy, stream/HDR checks, and native parity; unittest discovery. | `validation.rs`, `packet_verify.rs`, `hdr_metadata.rs`. | Active parity gate. |
| `tests/test_youtube.py` | YouTube dimensions, bitrate, HDR-loss consent, output collision, and export policy; unittest discovery. | `youtube.rs`, `media_scan.rs`. | Keep until batch/report parity is complete. |
| `tools/benchmark_scan.py` | Generates disposable media and benchmarks native scan performance; called manually by the README benchmark command. | Native benchmark harness. | Python helper remains supported; never run against user media during validation. |
| `tools/extract_clip.py` | Creates a bounded stream-copy or transcode fixture without overwriting the source; called by users/agents and `tests/test_extract_clip.py`. | No direct runtime replacement required. | Keep as a portable Python utility while the project uses Python fixtures. |

## Main caller graph

```text
hevc-rebuild-studio / hevc_rebuild.py
  -> __main__.py -> app.py
      -> config.py, deps.py, discovery.py, probe.py
      -> encode.py -> resources.py, calibration.py, hdr.py
      -> metadata.py -> validate.py -> transactions.py
      -> ui.py, util.py, reports/log writers

Tauri GUI / native JSON / MCP
  -> lib.rs / mcp_server.rs
      -> native Rust scan/settings/toolchain/queue/report/transaction modules
      -> engine_command -> app.py for stages not yet ported
```

## Progress states

- **Native**: the claimed application caller uses Rust for the behavior.
- **Split**: Rust owns a bounded app-launched slice while Python remains a live
  direct or compatibility caller.
- **Python-owned**: no equivalent Rust implementation is ready for removal.
- **Retirement-ready**: parity fixtures, supported-platform packaging, and
  normal/error/cancellation/report/recovery evidence are complete.

Update this file whenever a caller, owner, parity fixture, or removal gate
changes. `RUST_MIGRATION.md` remains the high-level architecture summary; this
ledger is the file-level source of truth.
