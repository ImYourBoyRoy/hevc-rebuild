// ./desktop/tests/tool-setup-repair.test.mjs
// Tool setup: update checks, catalog refresh and the repair-apt flow. Run: pnpm test:tool-setup

import assert from "node:assert/strict";
import test from "node:test";
import { check, aptEnvironment, harness, notices } from "./support/tool-setup-harness.mjs";

test("checkUbuntuPackageUpdates returns the current value when it may not run", async () => {
  for (const hostOver of [{ aptChecking: true }, { runId: "r1" }, { aptUpdates: "kept", environment: aptEnvironment({ os: { supported: true, package_manager: "dnf" } }) }]) {
    const h = harness({ hostOver });
    const before = h.host.aptUpdates;
    assert.equal(await h.controller.checkUbuntuPackageUpdates(), before);
    assert.equal(h.invokes.length, 0);
  }
  const off = harness({ desktop: false });
  assert.equal(await off.controller.checkUbuntuPackageUpdates(), null);
  assert.equal(off.invokes.length, 0);
});

test("checkUbuntuPackageUpdates reports counts, staleness and honours silent", async () => {
  const cases = [
    [{ packages: [], catalog_stale: false }, "success", "No updates are available for the checked Ubuntu packages."],
    [{ packages: [{ name: "ffmpeg" }], catalog_stale: false }, "success", "1 allowlisted Ubuntu package update found."],
    [{ packages: [{ name: "a" }, { name: "b" }], catalog_stale: false }, "success", "2 allowlisted Ubuntu package updates found."],
    [{ packages: [{ name: "ffmpeg" }], catalog_stale: true }, "info", "Found 1 update in a stale local APT catalog. Refresh the catalog before installing."],
    [{ packages: [], catalog_stale: true }, "info", "Found 0 updates in a stale local APT catalog. Refresh the catalog before installing."],
  ];
  for (const [status, kind, message] of cases) {
    const h = harness({ invokeImpl: { check_ubuntu_package_updates: status } });
    assert.equal(await h.controller.checkUbuntuPackageUpdates(), status);
    assert.equal(h.host.aptUpdates, status);
    assert.equal(h.host.aptChecking, false);
    assert.deepEqual(notices(h), [[kind, message]]);
    const quiet = harness({ invokeImpl: { check_ubuntu_package_updates: status } });
    await quiet.controller.checkUbuntuPackageUpdates(true);
    assert.deepEqual(notices(quiet), []);
  }
});

test("checkUbuntuPackageUpdates failure returns null and only warns when not silent", async () => {
  const fail = { check_ubuntu_package_updates: () => { throw new Error("apt down"); } };
  const loud = harness({ invokeImpl: fail });
  assert.equal(await loud.controller.checkUbuntuPackageUpdates(), null);
  assert.equal(loud.host.aptChecking, false);
  assert.deepEqual(notices(loud), [["error", "Could not check Ubuntu package updates: Error: apt down"]]);
  const quiet = harness({ invokeImpl: fail });
  assert.equal(await quiet.controller.checkUbuntuPackageUpdates(true), null);
  assert.deepEqual(notices(quiet), []);
});

test("refreshUbuntuPackageCatalog starts the job and records its kind", async () => {
  const h = harness({ invokeImpl: { refresh_ubuntu_package_catalog: "job-7" } });
  h.host.repairLines = ["old"];
  h.host.repairLogWarning = "old warning";
  await h.controller.refreshUbuntuPackageCatalog(true);
  assert.deepEqual([h.host.repairId, h.host.packageJobKind, h.host.continueToolchainSetupAfterRefresh], ["job-7", "catalog-refresh", true]);
  assert.deepEqual([h.host.repairLines, h.host.repairLogWarning, h.host.repairSummary, h.host.repairSummaryIsError], [[], "", "Refreshing Ubuntu package information…", false]);
  assert.deepEqual(notices(h), [["info", "Refreshing the configured Ubuntu package catalog. No software is being installed."]]);
});

test("refreshUbuntuPackageCatalog failure resets the continue flag and reports the error", async () => {
  const h = harness({ invokeImpl: { refresh_ubuntu_package_catalog: () => { throw new Error("denied"); } } });
  h.host.continueToolchainSetupAfterRefresh = true;
  await h.controller.refreshUbuntuPackageCatalog(true);
  assert.equal(h.host.continueToolchainSetupAfterRefresh, false);
  assert.equal(h.host.repairId, "");
  assert.deepEqual([h.host.repairSummary, h.host.repairSummaryIsError], ["Package information could not be refreshed: Error: denied", true]);
  assert.deepEqual(notices(h), [["error", "Could not refresh the Ubuntu package catalog: Error: denied"]]);
});

test("refreshUbuntuPackageCatalog is blocked while busy, off desktop or on a non-APT host", async () => {
  for (const [options, hostOver] of [[{}, { aptChecking: true }], [{}, { runId: "r" }], [{ desktop: false }, {}], [{}, { environment: aptEnvironment({ os: { supported: true, package_manager: "dnf" } }) }]]) {
    const h = harness({ ...options, hostOver });
    await h.controller.refreshUbuntuPackageCatalog();
    assert.equal(h.invokes.length, 0);
  }
});

test("repairApt does nothing while a job runs and explains a non-APT host", async () => {
  const busy = harness({ hostOver: { repairId: "job" } });
  await busy.controller.repairApt();
  assert.deepEqual([busy.invokes.length, busy.log.length], [0, 0]);
  const other = harness({ hostOver: { environment: aptEnvironment({ os: { supported: true, package_manager: "dnf" } }) } });
  await other.controller.repairApt();
  assert.deepEqual(notices(other), [["error", "Automatic APT repair is available only on Linux hosts using APT. Use the platform setup documented in Help & commands."]]);
  const unsupported = harness({ hostOver: { environment: aptEnvironment({ os: { supported: false, package_manager: "apt", package_manager_available: true } }) } });
  await unsupported.controller.repairApt();
  assert.equal(notices(unsupported).length, 1);
});

test("repairApt checks updates first and stops when that check fails", async () => {
  const h = harness({ invokeImpl: { check_ubuntu_package_updates: () => { throw new Error("no apt"); } } });
  await h.controller.repairApt();
  assert.equal(h.host.repairConfirmOpen, false);
  assert.deepEqual(h.invokes.map((call) => call.command), ["check_ubuntu_package_updates"]);
});

test("repairApt refreshes a stale catalog first and remembers to continue", async () => {
  const h = harness({ invokeImpl: { refresh_ubuntu_package_catalog: "job-1" }, hostOver: { aptUpdates: { packages: [], catalog_stale: true } } });
  await h.controller.repairApt();
  assert.deepEqual(h.invokes.map((call) => call.command), ["refresh_ubuntu_package_catalog"]);
  assert.equal(h.host.continueToolchainSetupAfterRefresh, true);
  assert.equal(h.host.packageJobKind, "catalog-refresh");
  assert.deepEqual(notices(h)[0], ["info", "The Ubuntu package catalog is stale. Refreshing it first; the package list will be shown for review before anything is installed."]);
  assert.equal(h.host.repairConfirmOpen, false);
});

test("repairApt with nothing to install installs a missing HDR10+ helper, else reports current", async () => {
  const current = { packages: [], catalog_stale: false };
  const helper = harness({ hostOver: { aptUpdates: current }, invokeImpl: { install_hdr10plus: "/home/u/.local/bin/hdr10plus_tool", inspect_environment: aptEnvironment(), list_tool_setup_logs: [] } });
  helper.host.environment = aptEnvironment({ checks: [check("pkexec"), check("hdr10plus_tool", { available: false })] });
  await helper.controller.repairApt();
  assert.equal(helper.invokes[0].command, "install_hdr10plus");
  const done = harness({ hostOver: { aptUpdates: current } });
  await done.controller.repairApt();
  assert.equal(done.host.repairSummary, "The checked Ubuntu packages are installed and current in the enabled repositories.");
  assert.equal(done.host.repairSummaryIsError, false);
  assert.deepEqual(notices(done), [["success", "The required system tools are installed and current in the enabled Ubuntu package sources."]]);
  assert.equal(done.host.repairConfirmOpen, false);
});

test("repairApt without pkexec refuses to change packages", async () => {
  const h = harness({ hostOver: { aptUpdates: { packages: [{ name: "ffmpeg" }], catalog_stale: false } } });
  h.host.environment = aptEnvironment({ checks: [check("pkexec", { available: false })] });
  await h.controller.repairApt();
  assert.deepEqual([h.host.repairSummary, h.host.repairSummaryIsError, h.host.repairConfirmOpen], ["Desktop administrator authentication (pkexec) is missing. No system packages were changed.", true, false]);
  assert.deepEqual(notices(h), [["error", "Desktop administrator authentication (pkexec) is missing. Install the listed system packages through Ubuntu Software or a terminal."]]);
});

test("repairApt opens the confirmation when there are missing packages or updates", async () => {
  const missing = harness({ hostOver: { aptUpdates: { packages: [], catalog_stale: false } } });
  missing.host.environment = aptEnvironment({ missing_apt_packages: ["mediainfo"] });
  await missing.controller.repairApt();
  assert.equal(missing.host.repairConfirmOpen, true);
  const updates = harness({ hostOver: { aptUpdates: { packages: [{ name: "ffmpeg" }], catalog_stale: false } } });
  await updates.controller.repairApt();
  assert.equal(updates.host.repairConfirmOpen, true);
});
