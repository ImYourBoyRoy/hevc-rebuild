// ./desktop/tests/run-controller-retry.test.mjs
// Run controller: manual retry, skip, report and diagnostics lookups. Run: pnpm test:run-controller

import assert from "node:assert/strict";
import test from "node:test";
import { entry, source, state, report, harness, retryHarness } from "./support/run-controller-harness.mjs";

test("retry is ignored while busy or for stale entries, and explains missing or unscanned sources", async () => {
  const busy = retryHarness("/r/a.mkv", { workflowBusy: true });
  await busy.controller.prepareQueueEntryForRetry(entry("/r/a.mkv"), state("failed"));
  await busy.controller.prepareQueueEntryForRetry(entry("/r/a.mkv", { queue_stale: "changed" }), state("failed"));
  assert.deepEqual(busy.calls, []);
  const detached = retryHarness();
  await detached.controller.prepareQueueEntryForRetry(entry("/x/zzz.mkv"), state("failed"));
  assert.deepEqual(detached.calls, [["notice", "error", "This file is no longer attached to a completed queue scan. Refresh its source before retrying."]]);
  const scanning = harness({ workspaceSetup: (workspace) => { workspace.queue.sources = [source("/r", ["/r/a.mkv"], { status: "scanning" })]; } });
  await scanning.controller.prepareQueueEntryForRetry(entry("/r/a.mkv"), state("failed"));
  assert.deepEqual(scanning.calls, [["notice", "error", "Refresh this source and wait for its scan to finish before retrying the file."]]);
});

test("retrying a failed file selects it in its mode, queues it and opens the matching page", async () => {
  const h = retryHarness();
  await h.controller.prepareQueueEntryForRetry(entry("/r/a.mkv"), state("failed", { mode: "youtube", reportPath: "/rep", reportStatus: "FAILED", outputPath: "/out" }));
  assert.deepEqual(h.calls[0], ["activate", "/r"]);
  assert.equal(h.host.mode, "youtube");
  assert.deepEqual(h.workspace.queue.selectedPaths, ["/r/a.mkv"]);
  const queued = h.workspace.queue.runStates["/r/a.mkv"];
  assert.deepEqual([queued.status, queued.mode, queued.reportPath, queued.outputPath], ["queued", "youtube", "/rep", "/out"]);
  assert.deepEqual(h.calls.slice(-2), [["openPage", "youtube"], ["notice", "info", "This file is selected for a retry. Review the task and output destination before starting."]]);
});

test("retrying an interrupted file only persists the selection, and failures roll the selection back", async () => {
  const h = retryHarness();
  await h.controller.prepareQueueEntryForRetry(entry("/r/a.mkv"), state("interrupted", { mode: null }));
  assert.ok(h.calls.some((item) => item[0] === "persistSources"));
  assert.equal(h.host.mode, "check");
  assert.deepEqual(h.calls.at(-1), ["notice", "info", "This file is selected. Start its saved task to review the last report and output before retrying."]);
  const failing = retryHarness("/r/a.mkv", { mode: "full" }, (workspace) => { workspace.queue.persistenceError = "ro"; });
  failing.workspace.queue.selectedPaths = ["/keep"];
  const failingRun = harness({ persistResults: [false], hostOverrides: { mode: "full" }, workspaceSetup: (workspace) => { workspace.queue.sources = [source("/r", ["/r/a.mkv"])]; workspace.queue.selectedPaths = ["/keep"]; workspace.queue.persistenceError = "ro"; } });
  const previous = state("failed", { mode: "test" });
  failingRun.workspace.queue.runStates = { "/r/a.mkv": previous };
  await failingRun.controller.prepareQueueEntryForRetry(entry("/r/a.mkv"), previous);
  assert.deepEqual(failingRun.workspace.queue.selectedPaths, ["/keep"]);
  assert.equal(failingRun.host.mode, "full");
  assert.equal(failingRun.workspace.queue.runStates["/r/a.mkv"], previous);
  assert.deepEqual(failingRun.calls.at(-1), ["notice", "error", "The retry could not be saved to the queue: ro"]);
  const selectionFailure = harness({ persistResults: [false], hostOverrides: { mode: "full" }, workspaceSetup: (workspace) => { workspace.queue.sources = [source("/r", ["/r/a.mkv"])]; workspace.queue.selectedPaths = ["/keep"]; workspace.queue.persistenceError = "ro"; } });
  await selectionFailure.controller.prepareQueueEntryForRetry(entry("/r/a.mkv"), state("needs_review"));
  assert.deepEqual(selectionFailure.calls.at(-1), ["notice", "error", "The retry selection could not be saved to the queue: ro"]);
  assert.equal(selectionFailure.host.mode, "full");
});

test("retrying a rename reopens the metadata review instead of a run", async () => {
  const h = retryHarness();
  await h.controller.prepareQueueEntryForRetry(entry("/r/a.mkv"), state("failed", { mode: "organize" }));
  assert.deepEqual(h.calls, [
    ["activate", "/r"], ["openPage", "library"], ["openMetadata", "/r/a.mkv"],
    ["notice", "info", "Rename review reopened for this file. Check the title and preview the proposed filename before confirming another change."],
  ]);
});

test("skipping unselects the file and records it, keeping report details; failures roll back", async () => {
  const h = harness({ workspaceSetup: (workspace) => { workspace.queue.selectedPaths = ["/r/a.mkv", "/r/b.mkv"]; } });
  await h.controller.skipQueueEntry(entry("/r/a.mkv"), state("failed", { mode: "test", reportPath: "/rep", outputPath: "/out", reportStatus: "FAILED" }));
  const skipped = h.workspace.queue.runStates["/r/a.mkv"];
  assert.deepEqual([skipped.status, skipped.mode, skipped.reportPath, skipped.outputPath], ["skipped", "test", "/rep", "/out"]);
  assert.deepEqual(h.workspace.queue.selectedPaths, ["/r/b.mkv"]);
  assert.deepEqual(h.calls.at(-1), ["notice", "success", "a.mkv is skipped for TEST. Select it again to review or run it."]);
  const fallback = harness({ hostOverrides: { mode: "metadata" } });
  await fallback.controller.skipQueueEntry(entry("/r/a.mkv"), state("failed", { mode: null }));
  assert.equal(fallback.workspace.queue.runStates["/r/a.mkv"].mode, "metadata");
  const previous = state("failed");
  const bad = harness({ persistResults: [false], workspaceSetup: (workspace) => { workspace.queue.selectedPaths = ["/r/a.mkv"]; workspace.queue.runStates = { "/r/a.mkv": previous }; workspace.queue.persistenceError = "ro"; } });
  await bad.controller.skipQueueEntry(entry("/r/a.mkv"), previous);
  assert.deepEqual(bad.workspace.queue.selectedPaths, ["/r/a.mkv"]);
  assert.equal(bad.workspace.queue.runStates["/r/a.mkv"], previous);
  assert.deepEqual(bad.calls.at(-1), ["notice", "error", "The skipped state could not be saved: ro"]);
  const busy = harness({ hostOverrides: { workflowBusy: true } });
  await busy.controller.skipQueueEntry(entry("/r/a.mkv"), state("failed"));
  assert.deepEqual(busy.calls, []);
});

test("report and diagnostics lookups use the recent history and explain missing reports", async () => {
  const csv = "/rep/run.csv";
  const missing = harness();
  await missing.controller.openQueueRunReport(csv);
  assert.deepEqual(missing.calls.at(-1), ["notice", "error", "The saved report is no longer available in recent history."]);
  const found = harness();
  found.host.loadReports = async () => { found.calls.push(["loadReports"]); found.host.reports = [report(csv, [])]; };
  await found.controller.openQueueRunReport(csv);
  assert.deepEqual(found.calls.slice(-2), [["openPage", "reports"], ["chooseReport", csv]]);

  const noPath = harness();
  await noPath.controller.loadQueueRunDiagnostics(entry("/r/a.mkv"), state("failed"));
  assert.deepEqual([noPath.calls, noPath.host.queueRunLogs], [[], {}]);
  const busy = harness({ hostOverrides: { queueRunLogs: { "/r/a.mkv": { loading: true, text: "", error: "" } } } });
  await busy.controller.loadQueueRunDiagnostics(entry("/r/a.mkv"), state("failed", { reportPath: csv }));
  assert.deepEqual(busy.calls, []);
  const ok = harness({ invokeImpl: () => "log text" });
  ok.host.loadReports = async () => { ok.host.reports = [report(csv, [])]; };
  await ok.controller.loadQueueRunDiagnostics(entry("/r/a.mkv"), state("failed", { reportPath: csv }));
  assert.deepEqual(ok.host.queueRunLogs["/r/a.mkv"], { loading: false, text: "log text", error: "" });
  assert.deepEqual(ok.invokes[0], { command: "read_log", args: { path: `${csv}.log` } });
  const gone = harness();
  await gone.controller.loadQueueRunDiagnostics(entry("/r/a.mkv"), state("failed", { reportPath: csv }));
  assert.deepEqual(gone.host.queueRunLogs["/r/a.mkv"], { loading: false, text: "", error: "Error: The report is no longer available in recent history." });
});
