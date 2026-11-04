# Retired runtime cleanup record

## Conclusion

The former scripting implementation has been removed from the current
worktree. The supported product no longer depends on it for the GUI, CLI,
JSON actions, MCP, metadata-only output, short tests, full rebuilds, HDR
validation, YouTube preparation, reports, or queue recovery.

This file keeps its historical filename so existing documentation links do not
break. It is not an instruction to restore a compatibility path.

## Evidence

The retirement boundary is satisfied when all of the following remain true:

- the Rust binary is the only application executable built by the product;
- the Tauri bundle contains no secondary-language source or engine resource;
- Rust source contains no process launch for the retired interpreter;
- frontend, Rust, CLI, and MCP tests exercise native coordinators;
- install and uninstall scripts operate on the native executable; and
- CI checks Rust/frontend builds without an interpreter setup step.

The current source tree satisfies those source/package conditions. A scoped
search of desktop/src, desktop/tests, desktop/src-tauri, installers, and CI
finds no retired interpreter command, module import, removed engine path, or
legacy test invocation. Remaining references should be limited to this
historical record and older audit material, never an active command or
supported workflow.

## Replacement map

| Former responsibility | Current native owner |
| --- | --- |
| Host/tool inspection and setup | environment.rs and dependencies.rs |
| Settings and precedence | settings.rs and settings_arguments.rs |
| Discovery, profiles, and forecasts | media_scan.rs and content.rs |
| Encode planning and process lifecycle | encode_plan.rs, encode_supervisor.rs, process_output.rs |
| Rebuild orchestration | rebuild_run.rs and run_coordinator.rs |
| Metadata-only output | metadata_run.rs |
| HDR10+/Dolby Vision policy | hdr_metadata.rs and validation.rs |
| YouTube export and Optimize | youtube.rs, youtube_optimize.rs, youtube_run.rs |
| Output validation | validation.rs, packet_verify.rs, matroska_verify.rs |
| No-overwrite transactions | transactions.rs |
| Reports and retained failures | reports.rs |
| Queue and recovery | media_queue.rs and output_targets.rs |
| GUI/CLI/MCP presentation | Svelte frontend, lib.rs, main.rs, and mcp_server.rs |

## No remaining transfer percentage

There is no remaining runtime transfer to perform. What remains is not another
module port; it is evidence and hardening:

- finish route/component extraction and characterization/controller tests;
- close native Tauri picker, drag/drop, queue, report, close/reopen,
  screen-reader, and live stop-now walkthroughs;
- run clean installer and supported-glibc AppImage acceptance;
- exercise representative real media across SDR, static HDR10, HDR10+, and
  explicit Dolby Vision fallback; and
- remove any stale historical wording discovered outside this record.

## Required checks

~~~bash
rg -n -i 'python|python3|app\.py|hevc_rebuild|pip|pytest|venv'   desktop/src desktop/tests desktop/src-tauri install.sh uninstall.sh .github
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked --lib
pnpm --dir desktop check
pnpm --dir desktop build
~~~

The first search is expected to return only the Cargo crate identifier and
ordinary shell syntax, not a runtime dependency. Do not convert a source check
failure into an ignored test; repair the source or record a concrete external
blocker.
