# Release and update workflow

HEVC Rebuild Studio uses the Tauri updater plugin with signed artifacts hosted in
GitHub Releases. The regular local build does not create updater signatures;
the release-only config enables them in CI.

## Published packages

`v1.3.0` is published at
https://github.com/ImYourBoyRoy/hevc-rebuild/releases/tag/v1.3.0. Its release
workflow completed the source, native compile, package, updater-signature, and
manifest checks before publication.

The release workflow produces:

- Windows x64: NSIS `setup.exe` only. MSI is deliberately not configured or published.
- macOS Apple Silicon: `.dmg`/`.app` output. Apple signing and notarization
  are optional and are not configured for the first release.
- macOS Intel: `.dmg`/`.app` output. Apple signing and notarization are
  optional and are not configured for the first release.
- Linux x64: `.AppImage` and `.deb` output built on Ubuntu 24.04.

The first release matrix covers the supported desktop architectures above. ARM
Linux and Windows ARM are intentionally not advertised until their native tool
availability and packaged runtime behavior are verified.

## Updater behavior

The desktop shell checks the signed GitHub Releases manifest five seconds after
startup and again every 24 hours. A new version appears in the application as a
banner with release notes and an explicit **Install & restart** action. The
installer is downloaded only after that action, signatures are checked by the
Tauri updater, and the application relaunches after installation when the host
allows it. A running scan, encode, repair, or queue rename blocks installation;
the user can choose **Later** and be prompted again for a newer release.

The self-update lane is for AppImage installs on Linux, NSIS installs on
Windows, and macOS installs when the app is distributed in an
updater-compatible bundle. Linux `.deb` users should update through
their package manager; the updater manifest intentionally selects the AppImage
for the Linux self-update entry rather than pretending a `.deb` can be safely
replaced in place.

`desktop/src-tauri/tauri.conf.json` contains the public updater key and the
future GitHub endpoint. `desktop/src-tauri/tauri.release.conf.json` enables
`createUpdaterArtifacts` only for release builds. Never commit the private
signing key.

## GitHub configuration

The public remote `ImYourBoyRoy/hevc-rebuild` is configured locally as `origin`.
The release workflow requires the updater signing key before a release tag is
pushed:

- `TAURI_SIGNING_PRIVATE_KEY`: the complete Tauri updater private key.
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: only if the private key is
  password-protected; the current local key is intentionally unencrypted.

Apple signing and notarization secrets are not configured. They can be added
later when a Developer ID distribution lane is desired.

Generate a new updater key outside the repository with the Tauri CLI, keep the
private file in a local credential store, and copy only the public key into the
Tauri config. The checked-in public key is unique to this application and its
matching private key is kept outside the worktree at the time this workflow was
prepared.

## Release steps

1. Run the local checks from the root and `desktop/src-tauri` documented in the
   root `README.md`.
2. Update the synchronized project version in `VERSION`, `package.json`,
   `desktop/package.json`, `desktop/src-tauri/Cargo.toml`, and
   `desktop/src-tauri/tauri.conf.json`.
3. Update release notes and commit the version change.
4. Create and push a matching tag such as `v1.3.1` only after the remote and
   GitHub secrets are ready.
5. The workflow runs CI, builds the four native release lanes, creates a draft
   GitHub Release, uploads the signed `latest.json`, and verifies that all
   updater URLs map to uploaded assets and that no MSI asset exists.
6. Inspect the draft assets and publish the draft manually. Publishing the
   release makes it visible to the in-app updater.

No release tag or upload is performed by local development builds.

## Current validation boundary

The first public release also completed GitHub Actions package lanes for
Windows x64, macOS Intel, macOS Apple Silicon, and Linux x64. The Windows NSIS
artifact and macOS DMGs were produced without MSI or Apple signing inputs, and
the signed `latest.json` manifest was verified against the uploaded assets.
This does not replace clean installer installation, native GUI walkthroughs,
real-media, accessibility, Apple signing/notarization, or live updater
installation evidence.
