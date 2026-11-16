// ./desktop/tests/tool-setup-events.test.mjs
// Tool setup: repair-line, log-warning and repair-finished event handlers. Run: pnpm test:tool-setup

import assert from "node:assert/strict";
import test from "node:test";
import { check, aptEnvironment, harness, notices, finishHarness } from "./support/tool-setup-harness.mjs";

test("handleRepairLine appends only the active job's lines and keeps the last 120", () => {
  const h = harness({ hostOver: { repairId: "job" } });
  h.controller.handleRepairLine({ runId: "other", line: "ignored" });
  assert.deepEqual(h.host.repairLines, []);
  for (let index = 0; index < 130; index += 1) h.controller.handleRepairLine({ runId: "job", line: `line ${index}` });
  assert.equal(h.host.repairLines.length, 120);
  assert.deepEqual([h.host.repairLines[0], h.host.repairLines.at(-1)], ["line 10", "line 129"]);
});

test("handleRepairLogWarning records the warning for the active setup log only", () => {
  const h = harness({ hostOver: { activeToolSetupLogId: "log-1" } });
  h.controller.handleRepairLogWarning({ runId: "other", line: "no" });
  assert.equal(h.host.repairLogWarning, "");
  h.controller.handleRepairLogWarning({ runId: "log-1", line: "log truncated" });
  assert.equal(h.host.repairLogWarning, "log truncated");
});

test("handleRepairFinished ignores other jobs", async () => {
  const h = finishHarness();
  await h.controller.handleRepairFinished({ jobId: "other", success: true, error: null });
  assert.deepEqual([h.host.repairId, h.invokes.length], ["job", 0]);
});

test("handleRepairFinished success for a toolchain job rechecks and reports completion", async () => {
  const h = finishHarness({ packageJobKind: "toolchain", repairToolChecksBefore: [check("ffmpeg", { available: false })], repairAffectedToolKeys: ["ffmpeg"] });
  await h.controller.handleRepairFinished({ jobId: "job", success: true, error: null, logWarning: "w" });
  assert.deepEqual([h.host.repairId, h.host.packageJobKind, h.host.repairLogWarning, h.host.repairSummaryIsError], ["", null, "w", false]);
  assert.equal(h.host.repairSummary, "Setup complete. All required media tools are ready.");
  assert.deepEqual([h.host.repairToolChecksBefore, h.host.repairAffectedToolKeys], [null, []]);
  assert.equal(h.host.repairToolOutcomes[0].key, "ffmpeg");
  assert.deepEqual(notices(h)[0], ["success", "Reviewed Ubuntu package setup completed. Rechecking installed tools."]);
  assert.deepEqual(h.invokes.map((call) => call.command), ["list_tool_setup_logs", "inspect_environment", "check_ubuntu_package_updates"]);
});

test("handleRepairFinished reports remaining attention when tools are still not ready", async () => {
  const h = finishHarness({ packageJobKind: "upgrade" }, { inspect_environment: aptEnvironment({ ready: false }) });
  await h.controller.handleRepairFinished({ jobId: "job", success: true, error: null });
  assert.equal(h.host.repairSummary, "Package work completed. One or more required tools still need attention; check their status in Settings.");
  assert.deepEqual(notices(h)[0], ["success", "Ubuntu package upgrades completed."]);
});

test("handleRepairFinished failure shows the error and clears the follow-up flags", async () => {
  const h = finishHarness({ packageJobKind: "toolchain", installHdrHelperAfterPackages: true, continueToolchainSetupAfterRefresh: true });
  await h.controller.handleRepairFinished({ jobId: "job", success: false, error: "apt exited 100" });
  assert.deepEqual([h.host.repairSummary, h.host.repairSummaryIsError, h.host.installHdrHelperAfterPackages], ["apt exited 100", true, false]);
  assert.deepEqual(notices(h)[0], ["error", "apt exited 100"]);
  const generic = finishHarness({ packageJobKind: "catalog-refresh", continueToolchainSetupAfterRefresh: true });
  await generic.controller.handleRepairFinished({ jobId: "job", success: false, error: null });
  assert.equal(generic.host.repairSummary, "The package operation did not finish successfully.");
  assert.equal(generic.host.continueToolchainSetupAfterRefresh, false);
});

test("handleRepairFinished for a catalog refresh continues the setup when asked", async () => {
  const h = finishHarness({ packageJobKind: "catalog-refresh", continueToolchainSetupAfterRefresh: true, aptUpdates: { packages: [{ name: "ffmpeg" }], catalog_stale: true } }, { check_ubuntu_package_updates: { packages: [{ name: "ffmpeg" }], catalog_stale: false } });
  await h.controller.handleRepairFinished({ jobId: "job", success: true, error: null });
  assert.equal(h.host.continueToolchainSetupAfterRefresh, false);
  assert.equal(h.host.repairConfirmOpen, true, "the reviewed list is shown after the refresh");
  assert.deepEqual(notices(h)[0], ["success", "Ubuntu package catalog refreshed."]);
});

test("handleRepairFinished installs the HDR10+ helper after a successful toolchain job", async () => {
  const h = finishHarness({ packageJobKind: "toolchain", installHdrHelperAfterPackages: true }, {
    inspect_environment: aptEnvironment({ checks: [check("pkexec"), check("hdr10plus_tool", { available: false })] }),
    install_hdr10plus: "/home/u/hdr10plus_tool",
  });
  await h.controller.handleRepairFinished({ jobId: "job", success: true, error: null });
  assert.equal(h.host.installHdrHelperAfterPackages, false);
  assert.equal(h.invokes.some((call) => call.command === "install_hdr10plus"), true);
  assert.notEqual(h.host.repairSummary, "Setup complete. All required media tools are ready.");
});

test("handleRepairFinished skips the helper when it is already available", async () => {
  const h = finishHarness({ packageJobKind: "toolchain", installHdrHelperAfterPackages: true });
  await h.controller.handleRepairFinished({ jobId: "job", success: true, error: null });
  assert.equal(h.host.installHdrHelperAfterPackages, false);
  assert.equal(h.invokes.some((call) => call.command === "install_hdr10plus"), false);
  assert.equal(h.host.repairSummary, "Setup complete. All required media tools are ready.");
});

test("handleRepairFinished uses the generic wording for unknown job kinds", async () => {
  const h = finishHarness({ packageJobKind: null });
  await h.controller.handleRepairFinished({ jobId: "job", success: true, error: null });
  assert.deepEqual(notices(h)[0], ["success", "Package repair completed. Rechecking the installed tools."]);
});
