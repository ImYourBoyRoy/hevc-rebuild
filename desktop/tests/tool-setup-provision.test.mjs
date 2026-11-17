// ./desktop/tests/tool-setup-provision.test.mjs
// Tool setup: the one-pass "install and update everything" review and apply flow (one password).
// Run: pnpm test:tool-setup

import assert from "node:assert/strict";
import test from "node:test";
import { check, aptEnvironment, finishHarness, harness, notices } from "./support/tool-setup-harness.mjs";

const plan = (over = {}) => ({
  refresh_catalog: true, install_packages: ["mediainfo"], upgrade_packages: ["ffmpeg"],
  helper: { action: "install", installed: null, latest: null },
  needs_password: true, password_prompts: 1, nothing_to_do: false, steps: ["Refresh the Ubuntu package catalog"], release_check_error: null,
  ...over,
});

test("reviewSetup fetches the plan once and opens the single confirmation", async () => {
  const h = harness({ invokeImpl: { provision_plan: plan() } });
  await h.controller.reviewSetup();
  assert.deepEqual(h.invokes, [{ command: "provision_plan", args: { toolPaths: { ffmpeg: "/opt/ffmpeg", ffprobe: null } } }]);
  assert.equal(h.host.setupPlan.install_packages[0], "mediainfo");
  assert.equal(h.host.setupConfirmOpen, true);
});

test("reviewSetup reports an up-to-date toolchain without opening a dialog or asking for a password", async () => {
  const h = harness({ invokeImpl: { provision_plan: plan({ nothing_to_do: true, needs_password: false, install_packages: [], upgrade_packages: [], refresh_catalog: false, helper: { action: "current", installed: "1.7.2", latest: null } }) } });
  await h.controller.reviewSetup();
  assert.equal(h.host.setupConfirmOpen, false);
  assert.equal(h.host.repairSummaryIsError, false);
  assert.deepEqual(notices(h), [["success", "Everything is installed and up to date."]]);
});

test("reviewSetup is blocked while busy or off the desktop and reports a failed plan", async () => {
  for (const [options, hostOver] of [[{}, { runId: "r" }], [{}, { repairId: "j" }], [{ desktop: false }, {}]]) {
    const h = harness({ ...options, hostOver, invokeImpl: { provision_plan: plan() } });
    await h.controller.reviewSetup();
    assert.equal(h.invokes.length, 0);
  }
  const failing = harness({ invokeImpl: { provision_plan: () => { throw new Error("offline"); } } });
  await failing.controller.reviewSetup();
  assert.equal(failing.host.setupConfirmOpen, false);
  assert.deepEqual(notices(failing), [["error", "Could not prepare the setup plan: Error: offline"]]);
});

test("confirmSetup starts exactly one provisioning job and snapshots the checks", async () => {
  const h = harness({ invokeImpl: { provision_toolchain: "job-9" }, hostOver: { setupConfirmOpen: true, setupPlan: plan() } });
  h.host.environment = aptEnvironment({ checks: [check("ffmpeg"), check("ffprobe"), check("mediainfo", { available: false }), check("hdr10plus_tool", { available: false })] });
  await h.controller.confirmSetup();
  assert.deepEqual(h.invokes, [{ command: "provision_toolchain", args: { toolPaths: { ffmpeg: "/opt/ffmpeg", ffprobe: null } } }]);
  assert.deepEqual([h.host.repairId, h.host.packageJobKind, h.host.setupConfirmOpen], ["job-9", "provision", false]);
  assert.ok(h.host.repairAffectedToolKeys.includes("hdr10plus_tool") && h.host.repairAffectedToolKeys.includes("mediainfo"));
  assert.equal(h.host.repairToolChecksBefore.length, 4);
  assert.equal(h.host.installHdrHelperAfterPackages, false, "the helper is part of the same job, not a follow-up");
});

test("confirmSetup without a reviewed plan does nothing and failures reset the job state", async () => {
  const none = harness({ invokeImpl: { provision_toolchain: "x" } });
  await none.controller.confirmSetup();
  assert.equal(none.invokes.length, 0);
  const failing = harness({ invokeImpl: { provision_toolchain: () => { throw new Error("polkit"); } }, hostOver: { setupPlan: plan(), setupConfirmOpen: true } });
  await failing.controller.confirmSetup();
  assert.deepEqual([failing.host.repairId, failing.host.repairSummaryIsError], ["", true]);
  assert.equal(failing.host.repairSummary, "Setup could not start: Error: polkit");
  assert.deepEqual(notices(failing), [["error", "Could not start setup: Error: polkit"]]);
});

test("a finished provisioning job rechecks tools and reports completion", async () => {
  const h = finishHarness({ packageJobKind: "provision", repairAffectedToolKeys: [], aptUpdates: null });
  await h.controller.handleRepairFinished({ jobId: "job", success: true, error: null });
  assert.equal(h.host.repairId, "");
  assert.equal(h.host.repairSummary, "Setup complete. All required media tools are ready.");
  assert.deepEqual(notices(h).at(-1), ["success", "Setup and updates completed. Rechecking installed tools."]);
  const failed = finishHarness({ packageJobKind: "provision" });
  await failed.controller.handleRepairFinished({ jobId: "job", success: false, error: "Authentication was cancelled" });
  assert.deepEqual([failed.host.repairSummary, failed.host.repairSummaryIsError], ["Authentication was cancelled", true]);
});

test("checkSetupUpdates quietly remembers the plan and never interrupts or errors", async () => {
  const h = harness({ invokeImpl: { provision_plan: plan() } });
  await h.controller.checkSetupUpdates();
  assert.equal(h.host.setupUpdates.install_packages[0], "mediainfo");
  assert.equal(h.host.setupConfirmOpen, false);
  assert.deepEqual(notices(h), []);
  const busy = harness({ hostOver: { runId: "r" }, invokeImpl: { provision_plan: plan() } });
  await busy.controller.checkSetupUpdates();
  assert.equal(busy.invokes.length, 0);
  const offline = harness({ hostOver: { setupUpdates: plan() }, invokeImpl: { provision_plan: () => { throw new Error("offline"); } } });
  await offline.controller.checkSetupUpdates();
  assert.equal(offline.host.setupUpdates.install_packages[0], "mediainfo", "the last known result is kept");
  assert.deepEqual(notices(offline), []);
});

test("a finished provisioning job refreshes the update notice", async () => {
  const h = finishHarness({ packageJobKind: "provision" }, { provision_plan: plan({ nothing_to_do: true, install_packages: [], upgrade_packages: [], helper: { action: "current", installed: "1.7.2", latest: "1.7.2" } }) });
  await h.controller.handleRepairFinished({ jobId: "job", success: true, error: null });
  assert.deepEqual(h.host.setupUpdates.install_packages, []);
});
