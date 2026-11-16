// ./desktop/tests/tool-setup-install.test.mjs
// Tool setup: reviewed toolchain confirmation and the HDR10+ helper install. Run: pnpm test:tool-setup

import assert from "node:assert/strict";
import test from "node:test";
import { check, aptEnvironment, harness, notices } from "./support/tool-setup-harness.mjs";

test("confirmAptRepair starts the reviewed toolchain job and snapshots the checks", async () => {
  const h = harness({ invokeImpl: { install_ubuntu_toolchain: "job-42" }, hostOver: { aptUpdates: { packages: [{ name: "ffmpeg" }, { name: "mkvtoolnix" }], catalog_stale: false }, repairConfirmOpen: true } });
  h.host.environment = aptEnvironment({ missing_apt_packages: ["mediainfo"], checks: [check("ffmpeg"), check("ffprobe"), check("mkvmerge"), check("mediainfo", { available: false }), check("hdr10plus_tool", { available: false })] });
  await h.controller.confirmAptRepair();
  assert.deepEqual(h.invokes[0], { command: "install_ubuntu_toolchain", args: { toolPaths: { ffmpeg: "/opt/ffmpeg", ffprobe: null }, confirmedMissingPackages: ["mediainfo"], confirmedUpdatePackages: ["ffmpeg", "mkvtoolnix"] } });
  assert.deepEqual([h.host.repairId, h.host.packageJobKind, h.host.repairConfirmOpen], ["job-42", "toolchain", false]);
  assert.equal(h.host.repairSummary, "Installing 1 missing package and updating 2 packages…");
  assert.equal(h.host.installHdrHelperAfterPackages, true);
  assert.deepEqual(h.host.repairAffectedToolKeys.sort(), ["ffmpeg", "ffprobe", "hdr10plus_tool", "mediainfo", "mkvmerge"]);
  assert.equal(h.host.repairToolChecksBefore.length, 5);
  assert.notEqual(h.host.repairToolChecksBefore[0], h.host.environment.checks[0]);
  assert.deepEqual(h.host.repairToolOutcomes, []);
  assert.deepEqual(notices(h), [["info", "System tool setup started. APT will install the reviewed missing packages, then apply only the listed updates."]]);
});

test("confirmAptRepair is guarded and does nothing with a stale catalog or an empty plan", async () => {
  const fresh = { packages: [{ name: "ffmpeg" }], catalog_stale: false };
  for (const hostOver of [{ repairId: "x", aptUpdates: fresh }, { hdrInstalling: true, aptUpdates: fresh }, { scanning: true, aptUpdates: fresh }, { aptUpdates: null }, { aptUpdates: { packages: [{ name: "ffmpeg" }], catalog_stale: true } }, { aptUpdates: { packages: [], catalog_stale: false } }]) {
    const h = harness({ hostOver });
    await h.controller.confirmAptRepair();
    assert.equal(h.invokes.length, 0);
    assert.equal(h.host.repairSummary, "");
  }
});

test("confirmAptRepair failure resets the job state and reports it", async () => {
  const h = harness({ invokeImpl: { install_ubuntu_toolchain: () => { throw new Error("polkit"); } }, hostOver: { aptUpdates: { packages: [{ name: "ffmpeg" }], catalog_stale: false } } });
  h.host.environment = aptEnvironment({ checks: [check("ffmpeg"), check("hdr10plus_tool", { available: false })] });
  await h.controller.confirmAptRepair();
  assert.deepEqual([h.host.repairId, h.host.installHdrHelperAfterPackages, h.host.repairSummaryIsError], ["", false, true]);
  assert.equal(h.host.repairSummary, "Tool setup could not start: Error: polkit");
  assert.deepEqual(notices(h), [["error", "Could not start system tool setup: Error: polkit"]]);
});

test("installHdrHelper installs, rechecks, records outcomes and clears its bookkeeping", async () => {
  const afterEnv = aptEnvironment({ checks: [check("pkexec"), check("hdr10plus_tool", { version: "2.0" })] });
  let jobIdAtInvoke;
  const h = harness({
    invokeImpl: {
      install_hdr10plus: (args) => { jobIdAtInvoke = args.jobId; assert.equal(h.host.activeToolSetupLogId, args.jobId); assert.equal(h.host.hdrInstalling, true); return "/home/u/hdr10plus_tool"; },
      inspect_environment: afterEnv, list_tool_setup_logs: [{ jobId: "x", modifiedUnix: 1, sizeBytes: 1, truncated: false }], read_tool_setup_log: "helper log",
    },
  });
  h.host.environment = aptEnvironment({ checks: [check("pkexec"), check("hdr10plus_tool", { available: false })] });
  await h.controller.installHdrHelper();
  assert.match(jobIdAtInvoke, /^\d+-\d+$/);
  assert.deepEqual([h.host.hdrInstalling, h.host.activeToolSetupLogId, h.host.repairToolChecksBefore, h.host.repairAffectedToolKeys], [false, "", null, []]);
  assert.equal(h.host.repairSummary, "HDR10+ helper installed and detected.");
  assert.equal(h.host.repairSummaryIsError, false);
  assert.equal(h.host.repairToolOutcomes.find((outcome) => outcome.key === "hdr10plus_tool").status, "Installed");
  assert.deepEqual(notices(h).map((entry) => entry[0]), ["info", "success"]);
  assert.equal(h.host.selectedToolSetupLog, "x");
});

test("installHdrHelper reports a helper that is still undetected after install", async () => {
  const h = harness({ invokeImpl: { install_hdr10plus: "/p", inspect_environment: aptEnvironment({ checks: [check("hdr10plus_tool", { available: false })] }), list_tool_setup_logs: [] } });
  await h.controller.installHdrHelper();
  assert.equal(h.host.repairSummary, "The helper installer finished, but the environment check still cannot detect HDR10+ helper.");
  assert.equal(h.host.repairSummaryIsError, true);
});

test("installHdrHelper failure rechecks the environment, keeps the error and always resets", async () => {
  const h = harness({ invokeImpl: { install_hdr10plus: () => { throw new Error("sha mismatch"); }, inspect_environment: aptEnvironment(), list_tool_setup_logs: [] } });
  h.host.repairToolOutcomes = [{ key: "ffmpeg", label: "FFMPEG", before: "a", after: "b", status: "Installed" }];
  await h.controller.installHdrHelper();
  assert.deepEqual([h.host.repairSummary, h.host.repairSummaryIsError], ["HDR10+ helper setup failed: Error: sha mismatch", true]);
  assert.deepEqual([h.host.hdrInstalling, h.host.activeToolSetupLogId, h.host.repairAffectedToolKeys], [false, "", []]);
  assert.deepEqual(h.host.repairToolOutcomes.map((outcome) => outcome.key), ["hdr10plus_tool"], "a fresh install clears earlier outcomes, then records the helper from the recheck");
  assert.deepEqual(notices(h).at(-1), ["error", "HDR10+ helper setup failed: Error: sha mismatch"]);
});

test("installHdrHelper(true) keeps the previous outcomes and is blocked while busy", async () => {
  const h = harness({ invokeImpl: { install_hdr10plus: "/p", inspect_environment: aptEnvironment(), list_tool_setup_logs: [] } });
  h.host.repairToolOutcomes = [{ key: "ffmpeg", label: "FFMPEG", before: "a", after: "b", status: "Installed" }];
  await h.controller.installHdrHelper(true);
  assert.equal(h.host.repairToolOutcomes.some((outcome) => outcome.key === "ffmpeg"), true);
  for (const hostOver of [{ runId: "r" }, { runStarting: true }, { repairId: "j" }, { hdrInstalling: true }, { scanning: true }]) {
    const blocked = harness({ hostOver });
    await blocked.controller.installHdrHelper();
    assert.equal(blocked.invokes.length, 0);
  }
});
