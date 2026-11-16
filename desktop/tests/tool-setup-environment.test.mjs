// ./desktop/tests/tool-setup-environment.test.mjs
// Tool setup: environment inspection, tool-to-package mapping and repair outcome recording. Run: pnpm test:tool-setup

import assert from "node:assert/strict";
import test from "node:test";
import { check, aptEnvironment, harness, notices } from "./support/tool-setup-harness.mjs";

test("loadEnvironment stores the inspection, passes the tool paths and toggles loading", async () => {
  const environment = aptEnvironment({ ready: true });
  const h = harness({ invokeImpl: { inspect_environment: environment } });
  h.host.environmentError = "old";
  const pending = h.controller.loadEnvironment();
  assert.equal(h.host.environmentLoading, true);
  assert.equal(await pending, true);
  assert.deepEqual([h.host.environment, h.host.environmentError, h.host.environmentLoading], [environment, "", false]);
  assert.deepEqual(h.invokes[0], { command: "inspect_environment", args: { toolPaths: { ffmpeg: "/opt/ffmpeg", ffprobe: null } } });
});

test("loadEnvironment failure clears the environment, records the error and warns", async () => {
  const h = harness({ invokeImpl: { inspect_environment: () => { throw new Error("boom"); } } });
  assert.equal(await h.controller.loadEnvironment(), false);
  assert.equal(h.host.environment, null);
  assert.equal(h.host.environmentError, "Error: boom");
  assert.equal(h.host.environmentLoading, false);
  assert.deepEqual(notices(h), [["error", "Could not check the selected executables: Error: boom"]]);
});

test("aptPackageForTool maps tools to their packages and unknown tools to null", () => {
  const { controller } = harness();
  assert.equal(controller.aptPackageForTool("ffmpeg"), "ffmpeg");
  assert.equal(controller.aptPackageForTool("ffprobe"), "ffmpeg");
  for (const key of ["mkvpropedit", "mkvmerge", "mkvextract"]) assert.equal(controller.aptPackageForTool(key), "mkvtoolnix");
  for (const key of ["mediainfo", "fontconfig", "ca-certificates"]) assert.equal(controller.aptPackageForTool(key), key);
  assert.equal(controller.aptPackageForTool("hdr10plus_tool"), null);
});

test("packageUpdateForTool finds the pending update for the tool's package", () => {
  const h = harness();
  assert.equal(h.controller.packageUpdateForTool("ffprobe"), undefined);
  h.host.aptUpdates = { packages: [{ name: "ffmpeg", candidate: "7" }, { name: "mkvtoolnix", candidate: "9" }], catalog_stale: false };
  assert.equal(h.controller.packageUpdateForTool("ffprobe").candidate, "7");
  assert.equal(h.controller.packageUpdateForTool("mkvmerge").candidate, "9");
  assert.equal(h.controller.packageUpdateForTool("hdr10plus_tool"), undefined);
});

test("toolCheckReadout covers missing, versioned and unchecked tools", () => {
  const { controller } = harness();
  assert.equal(controller.toolCheckReadout(undefined), "Not checked");
  assert.equal(controller.toolCheckReadout(check("a", { available: false, path: "/x" })), "Missing · /x");
  assert.equal(controller.toolCheckReadout(check("a", { available: false, path: null, detail: "not found" })), "Missing · not found");
  assert.equal(controller.toolCheckReadout(check("a")), "1.0 · /usr/bin/a");
  assert.equal(controller.toolCheckReadout(check("a", { version: null, path: null })), "Available");
});

test("toolKeysForPackages returns the unique checked tools that belong to the packages", () => {
  const h = harness();
  h.host.environment = aptEnvironment({ checks: [check("ffmpeg"), check("ffprobe"), check("mkvmerge"), check("mediainfo"), check("pkexec")] });
  assert.deepEqual(h.controller.toolKeysForPackages(["ffmpeg"]), ["ffmpeg", "ffprobe"]);
  assert.deepEqual(h.controller.toolKeysForPackages(["mkvtoolnix", "mediainfo"]), ["mkvmerge", "mediainfo"]);
  assert.deepEqual(h.controller.toolKeysForPackages([]), []);
  h.host.environment = null;
  assert.deepEqual(h.controller.toolKeysForPackages(["ffmpeg"]), []);
});

test("recordRepairToolOutcomes classifies each affected tool and keeps earlier outcomes", () => {
  const h = harness();
  const before = [check("ffmpeg", { available: false, version: null, path: null, detail: "gone" }), check("ffprobe"), check("mkvmerge", { version: "1" }), check("mediainfo")];
  const after = [check("ffmpeg"), check("ffprobe", { available: false, path: null, detail: "x" }), check("mkvmerge", { version: "2" }), check("mediainfo")];
  h.host.repairAffectedToolKeys = ["ffmpeg", "ffprobe", "mkvmerge", "mediainfo", "ghost"];
  h.host.repairToolOutcomes = [{ key: "other", label: "OTHER", before: "b", after: "a", status: "Installed" }];
  h.controller.recordRepairToolOutcomes(before, after);
  const byKey = Object.fromEntries(h.host.repairToolOutcomes.map((outcome) => [outcome.key, outcome]));
  assert.equal(byKey.other.status, "Installed");
  assert.equal(byKey.ffmpeg.status, "Installed");
  assert.equal(byKey.ffmpeg.before, "Missing · gone");
  assert.equal(byKey.ffprobe.status, "Still missing");
  assert.equal(byKey.mkvmerge.status, "Updated or re-resolved");
  assert.equal(byKey.mediainfo.status, "Ready · unchanged");
  assert.equal("ghost" in byKey, false);
  assert.equal(byKey.mkvmerge.label, "MKVMERGE");
});

test("recordRepairToolOutcomes does nothing without a baseline or affected tools", () => {
  const h = harness();
  h.host.repairAffectedToolKeys = ["ffmpeg"];
  h.controller.recordRepairToolOutcomes(null, [check("ffmpeg")]);
  assert.deepEqual(h.host.repairToolOutcomes, []);
  h.host.repairAffectedToolKeys = [];
  h.controller.recordRepairToolOutcomes([check("ffmpeg")], [check("ffmpeg")]);
  assert.deepEqual(h.host.repairToolOutcomes, []);
});
