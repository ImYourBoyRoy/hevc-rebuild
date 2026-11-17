// ./desktop/tests/setup-updates.test.mjs
// The header notice for tools that are missing or out of date. Run: pnpm test:setup-updates

import assert from "node:assert/strict";
import test from "node:test";
import { setupUpdateSummary, SETUP_CHECK_INTERVAL_MS } from "../src/lib/setup-updates.ts";

const plan = (over = {}) => ({ manager: "apt", blockers: [], advisories: [], refresh_catalog: false, install_packages: [], upgrade_packages: [], helper: { action: "current", installed: "1.7.2", latest: "1.7.2" }, needs_password: false, password_prompts: 0, nothing_to_do: true, steps: [], release_check_error: null, ...over });

test("a current toolchain shows no notice", () => {
  assert.equal(setupUpdateSummary(null), null);
  assert.equal(setupUpdateSummary(plan()), null);
  assert.equal(setupUpdateSummary(plan({ refresh_catalog: true, nothing_to_do: false })), null, "a stale catalog alone is not an available update");
});

test("missing packages, package updates and a helper update are counted and described", () => {
  const summary = setupUpdateSummary(plan({ install_packages: ["mediainfo"], upgrade_packages: ["ffmpeg", "mkvtoolnix"], helper: { action: "update", installed: "1.7.0", latest: "1.7.2" }, nothing_to_do: false }));
  assert.equal(summary.count, 4);
  assert.equal(summary.label, "4 UPDATES");
  assert.match(summary.detail, /Install missing: mediainfo/);
  assert.match(summary.detail, /Update: ffmpeg, mkvtoolnix/);
  assert.match(summary.detail, /HDR10\+ helper 1\.7\.0 to 1\.7\.2/);
});

test("a missing helper is an install, singular wording, and checks repeat every six hours", () => {
  const summary = setupUpdateSummary(plan({ helper: { action: "install", installed: null, latest: null }, nothing_to_do: false }));
  assert.equal(summary.label, "1 UPDATE");
  assert.match(summary.detail, /Install the HDR10\+ helper/);
  assert.equal(SETUP_CHECK_INTERVAL_MS, 6 * 60 * 60 * 1000);
});

test("a missing prerequisite shows as setup needed with the remedy, ahead of any package count", () => {
  const blocker = { key: "brew", label: "Homebrew", remedy: "Install Homebrew, then check again.", command: "/bin/bash -c install" };
  const summary = setupUpdateSummary(plan({ manager: "brew", blockers: [blocker], nothing_to_do: false, install_packages: [] }));
  assert.equal(summary.label, "SETUP NEEDED");
  assert.match(summary.detail, /Homebrew/);
  assert.match(summary.detail, /Install Homebrew, then check again\./);
  const both = setupUpdateSummary(plan({ blockers: [blocker], install_packages: ["ffmpeg"], nothing_to_do: false }));
  assert.equal(both.label, "SETUP NEEDED");
});

test("an advisory alone is not an update notice", () => {
  assert.equal(setupUpdateSummary(plan({ advisories: ["Your FFmpeg is the Essentials build."] })), null);
});
