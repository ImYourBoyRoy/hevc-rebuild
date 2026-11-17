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
