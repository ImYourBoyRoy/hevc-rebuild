// ./desktop/tests/run-controller-finish.test.mjs
// Run controller: finishing a batch, report-row mapping and moving to the next source. Run: pnpm test:run-controller

import assert from "node:assert/strict";
import test from "node:test";
import { source, batchOf, finished, report, harness, runningHarness } from "./support/run-controller-harness.mjs";

test("finishRun words the result by outcome and reloads the reports", async () => {
  const cases = [
    [finished({ error: "engine crashed", exitCode: 1 }), "engine crashed", "error"],
    // An engine error is announced as an error even when the exit code was 0.
    [finished({ error: "engine crashed" }), "engine crashed", "error"],
    [finished({ stopRequested: true }), "Queue stopped safely after the active file.", "success"],
    [finished({ exitCode: 0 }), "Workflow finished. Review the report and any skipped files.", "success"],
    [finished({ exitCode: 3 }), "Workflow finished with exit code 3. Review the diagnostics.", "error"],
    [finished({ exitCode: null }), "Workflow finished with exit code unknown. Review the diagnostics.", "error"],
  ];
  for (const [payload, message, kind] of cases) {
    const h = runningHarness();
    await h.controller.finishRun(payload);
    assert.equal(h.workspace.run.id, "");
    assert.equal(h.workspace.run.message, message);
    assert.ok(h.calls.some((item) => item[0] === "loadReports"));
    assert.deepEqual(h.calls.at(-1), ["notice", kind, message]);
    assert.deepEqual([h.host.pendingRunBatches, h.host.activeRunBatchIndex, h.workspace.run.stopAfterCurrent, h.workspace.run.aborting], [[], -1, false, false]);
  }
});

test("finishRun maps each file to its report row; a file without one is interrupted after a bad exit", async () => {
  const csv = "/rep/run.csv";
  const rows = [
    { input: "/r/a.mkv", status: "ok", message: "fine", output: "/out/a.mkv" },
    { input: "/r/b.mkv", status: "failed", message: "x".repeat(2000), output: "" },
  ];
  const h = runningHarness({ reports: [] }, null);
  h.host.loadReports = async () => { h.calls.push(["loadReports"]); h.host.reports = [report(csv, rows)]; };
  await h.controller.finishRun(finished({ exitCode: 1, reportPath: csv }));
  const states = h.workspace.queue.runStates;
  assert.deepEqual([states["/r/a.mkv"].status, states["/r/a.mkv"].message, states["/r/a.mkv"].outputPath, states["/r/a.mkv"].reportStatus, states["/r/a.mkv"].reportPath], ["completed", "fine", "/out/a.mkv", "OK", csv]);
  assert.equal(states["/r/b.mkv"].status, "failed");
  assert.equal(states["/r/b.mkv"].message.length, 1024);
  assert.equal(states["/r/b.mkv"].outputPath, null);
  assert.equal(states["/r/c.mkv"].status, "interrupted");
  assert.equal(states["/r/c.mkv"].message, null);
  assert.equal(states["/r/c.mkv"].mode, "full");
});

test("after a clean exit a file without report evidence needs review", async () => {
  const h = runningHarness();
  await h.controller.finishRun(finished({ exitCode: 0 }));
  const state = h.workspace.queue.runStates["/r/a.mkv"];
  assert.equal(state.status, "needs_review");
  assert.match(state.message, /No matching per-file result was found in the run report/);
});

test("a safe stop leaves files without a result queued with an explanation", async () => {
  const h = runningHarness();
  await h.controller.finishRun(finished({ stopRequested: true }));
  assert.equal(h.workspace.queue.runStates["/r/a.mkv"].status, "queued");
  assert.equal(h.workspace.queue.runStates["/r/a.mkv"].message, "Not started before the safe stop.");
});

test("finishRun starts the next source batch unless it stopped, errored with stop-on-error, or was the last", async () => {
  const a = source("/a", ["/a/1.mkv"]), b = source("/b", ["/b/1.mkv"]);
  const setup = (workspace) => { workspace.queue.sources = [a, b]; };
  const proceed = harness({ hostOverrides: { mode: "full", pendingRunBatches: [batchOf(a), batchOf(b)], activeRunBatchIndex: 0 }, workspaceSetup: (workspace) => { setup(workspace); workspace.run.id = "run-1"; workspace.run.mode = "full"; } });
  await proceed.controller.finishRun(finished({ exitCode: 0 }));
  assert.equal(proceed.host.activeRunBatchIndex, 1);
  assert.deepEqual(proceed.invokes.filter((item) => item.command === "start_run").map((item) => item.args.request.root), ["/b"]);
  assert.match(proceed.workspace.run.message, /^Full rebuild 2\/2 · b/);
  const blocked = [
    [{ stopOnError: true }, finished({ exitCode: 1 }), /^Stopped after an error\. 1 source batch remains queued\.$/],
    [{}, finished({ exitCode: 0, stopRequested: true }), /^Stopped safely\. 1 source batch remains queued\.$/],
  ];
  for (const [overrides, payload, message] of blocked) {
    const h = harness({ hostOverrides: { mode: "full", pendingRunBatches: [batchOf(a), batchOf(b)], activeRunBatchIndex: 0, ...overrides }, workspaceSetup: (workspace) => { setup(workspace); workspace.run.id = "run-1"; workspace.run.mode = "full"; } });
    await h.controller.finishRun(payload);
    assert.equal(h.invokes.filter((item) => item.command === "start_run").length, 0);
    assert.match(h.calls.at(-1)[2], message);
  }
  const stopNow = harness({ hostOverrides: { mode: "full", pendingRunBatches: [batchOf(a), batchOf(b), batchOf(b)], activeRunBatchIndex: 0 }, workspaceSetup: (workspace) => { setup(workspace); workspace.run.id = "run-1"; workspace.run.aborting = true; } });
  await stopNow.controller.finishRun(finished({ exitCode: 1 }));
  assert.match(stopNow.calls.at(-1)[2], /^Stopped now\. 2 source batches remain queued\. Source files were not replaced\.$/);
});

test("finishing the last batch opens its log, and a queue that cannot be saved stops the remaining batches", async () => {
  const withLog = runningHarness();
  await withLog.controller.finishRun(finished({ logPath: "/rep/run.log" }));
  assert.ok(withLog.calls.some((item) => item[0] === "openLog" && item[1] === "/rep/run.log"));
  const a = source("/a", ["/a/1.mkv"]), b = source("/b", ["/b/1.mkv"]);
  const failing = harness({
    persistResults: [false], hostOverrides: { mode: "full", pendingRunBatches: [batchOf(a), batchOf(b)], activeRunBatchIndex: 0 },
    workspaceSetup: (workspace) => { workspace.queue.sources = [a, b]; workspace.run.id = "run-1"; workspace.run.mode = "full"; workspace.queue.persistenceError = "ro"; workspace.run.stopAfterCurrent = true; },
  });
  await failing.controller.finishRun(finished({ exitCode: 0 }));
  assert.equal(failing.workspace.run.message, "The report was written, but queue state could not be saved. Later source batches were not started.");
  assert.deepEqual(failing.calls.at(-1), ["notice", "error", "The report was written, but queue state could not be saved. Later source batches were not started. ro"]);
  assert.deepEqual([failing.host.pendingRunBatches, failing.workspace.run.stopAfterCurrent], [[], false]);
});
