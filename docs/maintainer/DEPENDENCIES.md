# Dependency and toolchain record

Last reviewed: 2026-11-04.

The repository now records the current compatible stable direct dependencies in
desktop/package.json and desktop/src-tauri/Cargo.toml. The lockfiles were
regenerated with the pinned package manager after the manifest update.

## Toolchain

| Tool | Repository value | Reason |
| --- | --- | --- |
| Node | 24.22.2 via .nvmrc; CI major 24 | Active supported LTS line for the frontend |
| pnpm | 12.16.1 in packageManager and CI | Current pnpm 12 line; frozen-lockfile installs |
| Rust | stable in CI; local toolchain checked during validation | Tauri's current supported MSRV is enforced by the resolved dependency graph |
| Linux build baseline | Ubuntu 24.04 / glibc baseline | Release AppImage must be rebuilt and tested on the supported baseline |

## Direct package pins

Frontend direct ranges are synchronized to the current compatible releases:
Tauri JS API/CLI 2.12.4, Tauri dialog 2.8.4, updater 2.14.1, Svelte 5.57.3,
SvelteKit 3.1.1, Vite 8.3.5, adapter-static 4.0.1, lucide 1.68.0,
svelte-check 4.7.8, TypeScript 6.0.3, and the TypeScript-native 7.0.2 lane.

Rust direct ranges are synchronized to Tauri 2.12.5/build 2.7.4, dialog 2.8.4,
updater 2.14.1, rmcp 3.8.0, tokio 1.54.1, libc 0.2.191,
unicode-normalization 0.1.26, toml 1.2.0, zip 9.3.0, and interprocess 2.4.7
for the opt-in local walkthrough channel. Other direct ranges remain at the
latest compatible stable values recorded in the manifest.

These are compatibility updates, not a security conclusion. The lockfile and
CI checks remain authoritative for reproducibility. Do not introduce
prereleases or broad transitive upgrades without a concrete compatibility
reason.

## Verification

Run:

~~~bash
pnpm --dir desktop install --frozen-lockfile
pnpm --dir desktop check
pnpm --dir desktop check:tsgo
pnpm --dir desktop build
cargo update --manifest-path desktop/src-tauri/Cargo.toml
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked --lib
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets --locked -- -D warnings
~~~

The install script builds the native Tauri executable and never installs a
secondary language runtime or silently changes system package state. Linux
media-tool setup remains an explicit host-tool operation documented in
TOOLS.md.

## References

- [Tauri dependency updating](https://v2.tauri.app/develop/updating-dependencies/)
- [Tauri Linux prerequisites](https://v2.tauri.app/start/prerequisites/)
- [pnpm installation](https://pnpm.io/installation)
- [Node.js release lines](https://nodejs.org/en/about/previous-releases)
