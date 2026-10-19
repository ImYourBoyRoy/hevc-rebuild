# Dependency maintenance — verified 2026-10-12

Baseline: Node 26.9.0, pnpm 12.10.1, Rust/Cargo 1.99.0. `desktop/package.json` supports Node `^22.17.0 || ^24.0.0 || ^26.0.0`; only 26.9.0 is exercised here. Current dependency Rust minimum is 1.90 (Tauri); the current toolchain exceeds it. Ubuntu 24.04/Mint 22 remain claimed baseline targets, requiring a compatible build environment rather than a newer-host glibc claim.

Registry snapshots: `/tmp/hevc-tasklist-20261011-00zltisa/t080-{frontend-outdated,npm-registry,rust-registry}.json`. Versions below were checked against the primary npm/crates.io registries, not model memory. Pending updates are freshness work, not vulnerability findings.

| Batch | Package | Before → target | Reason / boundary |
|---|---|---|---|
| Frontend | @tauri-apps/api | 2.12.1 → 2.12.3 | Stable patch, same minor as Rust |
| Frontend | @tauri-apps/cli | 2.12.1 → 2.12.2 | Stable tooling patch |
| Frontend | vite | 8.3.3 → 8.3.4 | Stable patch, unchanged Node compatibility |
| Frontend | @lucide/svelte | 1.52.0 → 1.55.0 | Compatible minor; exported icons and UI checks required |
| Retain | typescript | 6.0.3 | Preserve independent TS6 and aliased TS7 gates; no major migration |
| Retain | dialog JS/Rust plugin | 2.8.1 | Both remain exactly matched; latest 2.8.2 requires a coordinated pair |
| Rust | tauri | 2.12.1 → 2.12.3 | Same JS/Rust minor |
| Rust | tauri-build | 2.7.1 → 2.7.2 | Matched supported build tooling |
| Rust | serde_json | 1.0.151 → 1.0.154 | Stable patch, all JSON contracts tested |
| Rust | toml | 1.1.6 → 1.1.8 | Stable patch, settings merge/parsing tested |
| Rust | sha2 | 0.11.0 → 0.11.1 | Stable patch, hashes and transaction fixtures tested |

Other direct application packages are current in the snapshots. pnpm 12.11.2 is available; retain the repository-pinned 12.10.1 for this controlled dependency batch. Retain pnpm's configured publication-age behavior and project exceptions. No prerelease or unrelated Node/Rust/toolchain upgrade is authorized by this batch.

Validate frontend with `pnpm check`, `pnpm check:tsgo`, `pnpm build`, state/matching and all browser contracts, plus native smoke. Validate Rust independently with format/check/tests/Clippy/build, cross-path fixtures and packaged smoke. Compare production bundle size after the frontend batch. A source pass does not stand in for native/package proof.

Rollback is confined to the batch's manifest/lockfile versions; preserve all product source changes and unrelated dirty work. Private pre-batch exact files are under `/tmp/hevc-tasklist-20261011-00zltisa/dependency-baseline`. Restore those files only when reverting that batch, then `pnpm install --frozen-lockfile` and `cargo check --locked`, followed by relevant gates. Do not use `git checkout` to discard the whole working tree.

First-party references: [Tauri dependency synchronization](https://v2.tauri.app/develop/updating-dependencies/), [Tauri Linux prerequisites](https://v2.tauri.app/start/prerequisites/), [pnpm configuration](https://pnpm.io/settings), [Node release lines](https://nodejs.org/en/about/previous-releases).

Frontend batch: requested versions installed successfully. Both TypeScript checks, build, state/matching and browser contracts passed. Native debug app launched against the updated frontend in an isolated profile; Home accessibility and Media library navigation worked. Total production JS changed by +80 bytes (+142 gzip bytes). This launch smoke is not packaged acceptance.
