// ./desktop/tests/app-version.test.mjs
// The version shown in the sidebar must come from package.json, never a typed-in number that
// drifts from the release (it once read v1.3.0 in a 1.4.1 build). Run: pnpm test:app-version

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (path) => readFileSync(new URL(path, import.meta.url), "utf8");

test("package.json, tauri.conf.json and Cargo.toml agree on the version", () => {
  const version = JSON.parse(read("../package.json")).version;
  assert.equal(JSON.parse(read("../src-tauri/tauri.conf.json")).version, version);
  assert.match(read("../src-tauri/Cargo.toml"), new RegExp(`^version = "${version.replaceAll(".", "\\.")}"`, "m"));
});

test("the sidebar reads the version instead of typing one", () => {
  const sidebar = read("../src/lib/views/ShellSidebar.svelte");
  assert.doesNotMatch(sidebar, /version-tag">v\d/, "no literal version number in the markup");
  assert.match(sidebar, /app-version/, "the version comes from the shared module");
});
