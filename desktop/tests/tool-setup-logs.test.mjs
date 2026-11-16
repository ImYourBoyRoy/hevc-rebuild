// ./desktop/tests/tool-setup-logs.test.mjs
// Tool setup: saved setup logs (list, open, export). Run: pnpm test:tool-setup

import assert from "node:assert/strict";
import test from "node:test";
import { harness, notices } from "./support/tool-setup-harness.mjs";

test("refreshToolSetupLogs loads the list and selects a preferred or the first log", async () => {
  const logs = [{ jobId: "b", modifiedUnix: 2, sizeBytes: 1, truncated: false }, { jobId: "a", modifiedUnix: 1, sizeBytes: 1, truncated: false }];
  const h = harness({ invokeImpl: { list_tool_setup_logs: logs, read_tool_setup_log: (args) => `log of ${args.jobId}` } });
  await h.controller.refreshToolSetupLogs();
  assert.deepEqual([h.host.toolSetupLogs, h.host.selectedToolSetupLog, h.host.viewedToolSetupLog, h.host.toolSetupLogError], [logs, "b", "", ""]);
  await h.controller.refreshToolSetupLogs("a");
  assert.deepEqual([h.host.selectedToolSetupLog, h.host.viewedToolSetupLog], ["a", "log of a"]);
  h.host.selectedToolSetupLog = "a";
  await h.controller.refreshToolSetupLogs("missing");
  assert.equal(h.host.selectedToolSetupLog, "a");
  h.host.selectedToolSetupLog = "gone";
  await h.controller.refreshToolSetupLogs();
  assert.equal(h.host.selectedToolSetupLog, "b");
});

test("refreshToolSetupLogs reports a list failure and does nothing off desktop", async () => {
  const bad = harness({ invokeImpl: { list_tool_setup_logs: () => { throw new Error("io"); } } });
  await bad.controller.refreshToolSetupLogs();
  assert.equal(bad.host.toolSetupLogError, "Could not load saved setup logs: Error: io");
  const off = harness({ desktop: false });
  await off.controller.refreshToolSetupLogs();
  assert.equal(off.invokes.length, 0);
});

test("openToolSetupLog reads a log, guards against reentry and reports a read failure", async () => {
  const h = harness({ invokeImpl: { read_tool_setup_log: (args) => `text:${args.jobId}` } });
  await h.controller.openToolSetupLog("j1");
  assert.deepEqual([h.host.selectedToolSetupLog, h.host.viewedToolSetupLog, h.host.toolSetupLogLoading, h.host.toolSetupLogError], ["j1", "text:j1", false, ""]);
  await h.controller.openToolSetupLog("");
  assert.equal(h.invokes.length, 1);
  h.host.toolSetupLogLoading = true;
  await h.controller.openToolSetupLog("j2");
  assert.equal(h.invokes.length, 1);
  const bad = harness({ invokeImpl: { read_tool_setup_log: () => { throw new Error("gone"); } } });
  bad.host.viewedToolSetupLog = "stale";
  await bad.controller.openToolSetupLog("j3");
  assert.deepEqual([bad.host.viewedToolSetupLog, bad.host.toolSetupLogError, bad.host.toolSetupLogLoading], ["", "Could not read this setup log: Error: gone", false]);
});

test("exportToolSetupLog saves through the dialog and confirms the path", async () => {
  const h = harness({ invokeImpl: { export_tool_setup_log: (args) => `${args.destination}` }, saveResult: "/tmp/out.log" });
  h.host.selectedToolSetupLog = "job-9";
  await h.controller.exportToolSetupLog();
  assert.deepEqual(h.log[0], ["save", { title: "Export tool setup log", defaultPath: "hevc-tool-setup-job-9.log", filters: [{ name: "Setup log", extensions: ["log", "txt"] }] }]);
  assert.deepEqual(h.invokes[0], { command: "export_tool_setup_log", args: { jobId: "job-9", destination: "/tmp/out.log" } });
  assert.deepEqual(notices(h), [["success", "Saved the complete setup log to /tmp/out.log."]]);
  assert.equal(h.host.toolSetupLogExporting, false);
});

test("exportToolSetupLog does nothing when cancelled, unselected or already exporting, and reports failures", async () => {
  const cancelled = harness({ saveResult: null });
  cancelled.host.selectedToolSetupLog = "j";
  await cancelled.controller.exportToolSetupLog();
  assert.deepEqual([cancelled.invokes.length, notices(cancelled).length, cancelled.host.toolSetupLogExporting], [0, 0, false]);
  const none = harness();
  await none.controller.exportToolSetupLog();
  assert.equal(none.log.length, 0);
  const busy = harness({ hostOver: { toolSetupLogExporting: true, selectedToolSetupLog: "j" } });
  await busy.controller.exportToolSetupLog();
  assert.equal(busy.log.length, 0);
  const bad = harness({ invokeImpl: { export_tool_setup_log: () => { throw new Error("full disk"); } } });
  bad.host.selectedToolSetupLog = "j";
  await bad.controller.exportToolSetupLog();
  assert.deepEqual(notices(bad), [["error", "Could not export the setup log: Error: full disk"]]);
  assert.equal(bad.host.toolSetupLogExporting, false);
});
