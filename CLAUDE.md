# CLAUDE.md

Follow every rule in [`AGENTS.md`](AGENTS.md). It is the single source of project instructions; this file adds nothing and must not diverge from it.

Before doing anything else in a session:

1. Read `AGENTS.md`, starting with "Assistant operating rules (read first)".
2. Run `node tools/tasklist-check.mjs --open` and read the newest `tasklist/*` file (newest wins; it lists what it supersedes).
3. Read `ARCHITECTURE.md` to locate files before searching the tree.

Key reminders (details live in `AGENTS.md`): the product is Rust-only; do not
add references to retired runtimes or missing paths; write tests before moving
code; keep docs, `ARCHITECTURE.md`, `SHA256SUMS` and the tasklist current in
the same change; ask the owner when something is ambiguous; commit locally
only when asked and never push.
