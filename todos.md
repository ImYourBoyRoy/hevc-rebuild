# todos.md — feature ideas not yet scheduled

This file is tracked in Git (owner decision 2026-10-29). Scheduled work lives in the newest tasklist, not here.

## Interactive tutorial guide (requested 2026-10-29)

Teach a new user the app with a guided tour driven by key commands, using the same keyboard hooks used to drive and monitor the real Tauri window (tasklist T10.10, T10.11).

- Tour steps are data (JSON): target element, caption, expected key, next step. No logic in markup.
- Must be embedded in the app: nothing for the user to install, identical on Windows, macOS and Linux.
- Keyboard-only: arrows/Enter advance, Esc exits, a help key restarts; the app currently has no global shortcut map, so one must exist first.
- Highlights the real control with the app's own focus ring and tokens, never a mouse-only overlay; respects reduced motion and WCAG AA contrast.
- Covers: add a folder to the Library, review a match, choose a Run mode, start and safely stop, read a Report, YouTube Optimize vs Upscale.
- Progress persisted in settings; skippable and replayable from Help.
- The same control channel doubles as the automated native-window smoke test.
