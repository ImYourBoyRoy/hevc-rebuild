// ./desktop/tests/run-controller-adopt.test.mjs
// Run controller: following a run another client started (an agent over MCP, or a second browser tab
// against the headless server). The GUI only renders events for the run id it knows, so it must adopt
// the id first. Run: pnpm test:run-controller

import assert from "node:assert/strict";
import test from "node:test";
import { finished, harness, progressEvent, source, batchOf } from "./support/run-controller-harness.mjs";

test("adoptActiveRun follows a media run started elsewhere: its id, a note, and the Run page", () => {
  const h = harness();
  const adopted = h.controller.adoptActiveRun({ id: "job-7", kind: "media_run" });
  assert.equal(adopted, true);
  assert.equal(h.workspace.run.id, "job-7");
  assert.deepEqual(h.workspace.run.lines, []);
  assert.equal(h.workspace.run.progress, null);
  assert.equal(h.workspace.run.starting, false);
  assert.ok(typeof h.workspace.run.startedAt === "number");
  assert.match(h.workspace.run.message, /another client/i);
  assert.ok(h.calls.some((call) => call[0] === "openPage" && call[1] === "run"));
  assert.deepEqual(h.calls.find((call) => call[0] === "notice"), ["notice", "info", "A run started by another client is shown here live."]);
});

test("an integrity check started elsewhere is shown as an integrity run", () => {
  const h = harness();
  h.controller.adoptActiveRun({ id: "job-8", kind: "integrity_check" });
  assert.equal(h.workspace.run.mode, "integrity");
});

test("adoptActiveRun never takes over a run this window is already following or starting", () => {
  const following = harness({ workspaceSetup: (workspace) => { workspace.run.id = "mine"; } });
  assert.equal(following.controller.adoptActiveRun({ id: "job-7", kind: "media_run" }), false);
  assert.equal(following.workspace.run.id, "mine");
  const starting = harness({ workspaceSetup: (workspace) => { workspace.run.starting = true; } });
  assert.equal(starting.controller.adoptActiveRun({ id: "job-7", kind: "media_run" }), false);
  assert.equal(starting.workspace.run.id, "");
  assert.deepEqual(starting.calls, []);
});

test("jobs that are not media runs, and an idle server, are not adopted", () => {
  const h = harness();
  for (const kind of ["system_provision", "helper_install", "folder_analysis", "rename_apply", "", undefined]) {
    assert.equal(h.controller.adoptActiveRun({ id: "job-9", kind }), false, String(kind));
  }
  assert.equal(h.controller.adoptActiveRun(null), false);
  assert.equal(h.controller.adoptActiveRun({ id: "", kind: "media_run" }), false);
  assert.equal(h.workspace.run.id, "");
  assert.deepEqual(h.calls, []);
});

test("an adopted run ends like any other: reports reload and the window is free again", async () => {
  const h = harness();
  h.controller.adoptActiveRun({ id: "job-7", kind: "media_run" });
  await h.controller.finishRun(finished({ runId: "job-7", exitCode: 0, reportPath: "/logs/x.csv", logPath: "/logs/x.log" }));
  assert.equal(h.workspace.run.id, "");
  assert.equal(h.workspace.run.message, "Workflow finished. Review the report and any skipped files.");
  assert.ok(h.calls.some((call) => call[0] === "loadReports"));
  assert.deepEqual(h.calls.filter((call) => call[0] === "persistStates"), [], "no queue entries belong to a run this window did not plan");
});

test("a failed foreign run is worded as a failure", async () => {
  const h = harness();
  h.controller.adoptActiveRun({ id: "job-7", kind: "media_run" });
  await h.controller.finishRun(finished({ runId: "job-7", exitCode: 1, error: "ffmpeg missing" }));
  assert.equal(h.workspace.run.message, "ffmpeg missing");
});

test("adopting does not disturb a window that has batches queued locally", () => {
  const src = source("/r", ["/r/a.mkv"]);
  const h = harness({ hostOverrides: { pendingRunBatches: [batchOf(src)], activeRunBatchIndex: -1 } });
  h.controller.adoptActiveRun({ id: "job-7", kind: "media_run" });
  assert.equal(h.host.pendingRunBatches.length, 1);
});

test("when a followed run ends the saved queue is reloaded, because the other client updated it on disk", async () => {
  const h = harness();
  h.controller.adoptActiveRun({ id: "job-7", kind: "media_run" });
  await h.controller.finishRun(finished({ runId: "job-7" }));
  const names = h.calls.map((call) => call[0]);
  assert.equal(names.filter((name) => name === "loadMediaQueue").length, 1);
  assert.ok(names.indexOf("loadMediaQueue") > names.indexOf("loadReports"), "reports first, then the queue");
});

test("a run this window planned and started does not reload the queue from disk when it ends", async () => {
  const h = harness();
  await h.controller.finishRun(finished({ runId: "run-1" }));
  assert.equal(h.calls.some((call) => call[0] === "loadMediaQueue"), false);
});

test("a followed run refreshes the queue as each new file starts, not on every progress tick", () => {
  const h = harness();
  h.controller.adoptActiveRun({ id: "job-7", kind: "media_run" });
  h.controller.noteRunProgress(progressEvent({ runId: "job-7", batchNumber: 1, filePercent: 10 }));
  h.controller.noteRunProgress(progressEvent({ runId: "job-7", batchNumber: 1, filePercent: 40 }));
  assert.equal(h.calls.filter((call) => call[0] === "loadMediaQueue").length, 1, "the first file shows as running");
  h.controller.noteRunProgress(progressEvent({ runId: "job-7", batchNumber: 2, filePercent: 5 }));
  assert.equal(h.calls.filter((call) => call[0] === "loadMediaQueue").length, 2, "the next file refreshes again");
});

test("the next run this window follows starts fresh after the first one ended", async () => {
  const h = harness();
  h.controller.adoptActiveRun({ id: "job-7", kind: "media_run" });
  await h.controller.finishRun(finished({ runId: "job-7" }));
  assert.equal(h.controller.adoptActiveRun({ id: "job-8", kind: "media_run" }), true);
  const before = h.calls.filter((call) => call[0] === "loadMediaQueue").length;
  h.controller.noteRunProgress(progressEvent({ runId: "job-8", batchNumber: 1 }));
  assert.equal(h.calls.filter((call) => call[0] === "loadMediaQueue").length, before + 1, "the new run tracks its own file position");
});
