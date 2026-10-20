# App artifact drop

Keep this folder at the repository root. It is a local cross-platform release
drop, not application source code.

The normal Tauri build writes packages under
`desktop/src-tauri/target/release/bundle/`; it does not automatically copy them
here. After a successful host-native build, copy the verified packages here if
you want a convenient local drop:

- Linux: `.deb` and AppImage
- Windows: NSIS `setup.exe`
- macOS: `.dmg` and `.app`

Everything in this folder except this README is ignored by Git. That includes
installer packages, updater signatures, manifests, and AppImage writable
sidecars. Sidecars can contain local queue, settings, logs, and other private
runtime data and must not be published.

See [`../docs/PROJECT-STRUCTURE.md`](../docs/PROJECT-STRUCTURE.md) and the root
README for the build, verification, and release workflow.
