# TODOs

## Cross-platform architecture

- [ ] **Long-term: replace the Python media engine with native Rust.** Remove Python as an application/runtime dependency while keeping the Tauri + Svelte interface. Make Rust the single authority for configuration defaults and precedence, media discovery and analysis, resource planning, FFmpeg orchestration, validation, safe file transactions, logs/reports, CLI, JSON bridge, and MCP tools. Preserve current behavior and interfaces through staged parity checks and representative performance/resource benchmarks. Add and verify Ubuntu, Windows, and macOS platform adapters and release builds before claiming support. Keep FFmpeg and other media-tool dependencies explicit, with per-platform discovery/repair or bundling decisions.

## GitHub readiness

- [ ] Decide repository visibility and license before any public release. The supplied source has no license file and the current package metadata is private; do not imply redistribution rights or invent a license.
- [ ] Create or select the GitHub repository and decide when to add a remote. The local repository has no remote configured.
