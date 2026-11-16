// ./desktop/tests/run-controller-progress.test.mjs
// Run controller: progress-to-queue-state mapping, rename state and safe/immediate stop. Run: pnpm test:run-controller

import assert from "node:assert/strict";
import test from "node:test";
import { entry, state, harness, progressEvent } from "./support/run-controller-harness.mjs";

test("noteRunProgress stores the event and moves an active file's queue state with the phase", () => {
  const states = new Map([["/r/a.mkv", state("running", { mode: "full" })]]);
  const h = harness({ hostOverrides: { queueRunStatesByPath: states } });
  const before = Date.now();
  h.controller.noteRunProgress(progressEvent());
  assert.equal(h.workspace.run.progress.phase, "Validating output");
  assert.ok(h.workspace.run.progressSeenAt >= before);
  assert.deepEqual(h.calls, [["persistStates", { "/r/a.mkv": "validating" }]]);
  assert.equal(h.workspace.queue.runStates["/r/a.mkv"].message, "Validating output");
});

test("progress does not touch queue state without a file, a state, an active state or a change", () => {
  const cases = [
    [progressEvent({ filePath: null }), new Map([["/r/a.mkv", state("running")]])],
    [progressEvent(), new Map()],
    [progressEvent(), new Map([["/r/a.mkv", state("completed")]])],
    [progressEvent({ phase: "Something else" }), new Map([["/r/a.mkv", state("running")]])],
    [progressEvent(), new Map([["/r/a.mkv", state("validating")]])],
  ];
  for (const [event, states] of cases) {
    const h = harness({ hostOverrides: { queueRunStatesByPath: states } });
    h.controller.noteRunProgress(event);
    assert.deepEqual(h.calls, [], JSON.stringify(event));
  }
});

test("the recorded phase message is limited to 1024 code points", () => {
  const h = harness({ hostOverrides: { queueRunStatesByPath: new Map([["/r/a.mkv", state("running")]]) } });
  h.controller.noteRunProgress(progressEvent({ phase: `validating ${"😀".repeat(1100)}` }));
  assert.equal([...h.workspace.queue.runStates["/r/a.mkv"].message].length, 1024);
});

test("rename state updates only files that are queued, using their saved path, in organize mode", () => {
  const h = harness({ hostOverrides: { queuedEntries: [entry("C:\\Media\\A.mkv")] } });
  h.controller.handleQueueRenameState(["c:/media/a.mkv", "/not/queued.mkv"], "renaming", "working");
  assert.deepEqual(h.calls, [["persistStates", { "C:\\Media\\A.mkv": "renaming" }]]);
  assert.equal(h.workspace.queue.runStates["C:\\Media\\A.mkv"].mode, "organize");
  assert.equal(h.workspace.queue.runStates["C:\\Media\\A.mkv"].message, "working");
  const idle = harness({ workspaceSetup: (workspace) => { workspace.queue.loaded = false; }, hostOverrides: { queuedEntries: [entry("/a")] } });
  idle.controller.handleQueueRenameState(["/a"], "renamed");
  idle.controller.handleQueueRenameState([], "renamed");
  assert.deepEqual(idle.calls, []);
});

test("a safe stop needs an active run, calls request_stop and reports success or failure", async () => {
  const idle = harness();
  await idle.controller.stopAfterCurrent();
  assert.deepEqual(idle.invokes, []);
  const aborting = harness({ workspaceSetup: (workspace) => { workspace.run.id = "run-1"; workspace.run.aborting = true; } });
  await aborting.controller.stopAfterCurrent();
  assert.deepEqual(aborting.invokes, []);
  const ok = harness({ workspaceSetup: (workspace) => { workspace.run.id = "run-1"; } });
  await ok.controller.stopAfterCurrent();
  assert.deepEqual(ok.invokes, [{ command: "request_stop", args: { runId: "run-1" } }]);
  assert.equal(ok.workspace.run.stopAfterCurrent, true);
  assert.equal(ok.workspace.run.message, "Safe stop requested. The current file will finish before the queue stops.");
  assert.deepEqual(ok.calls.at(-1), ["notice", "success", "Safe stop requested; the active file will finish first."]);
  const bad = harness({ invokeImpl: () => { throw new Error("nope"); }, workspaceSetup: (workspace) => { workspace.run.id = "run-1"; } });
  await bad.controller.stopAfterCurrent();
  assert.equal(bad.workspace.run.stopAfterCurrent, false);
  assert.deepEqual(bad.calls.at(-1), ["notice", "error", "Could not request a safe stop: Error: nope"]);
});

test("stop now needs an active run that is not already stopping safely, and closes its confirmation", async () => {
  const idle = harness();
  await idle.controller.confirmStopNow();
  assert.deepEqual(idle.invokes, []);
  const safe = harness({ workspaceSetup: (workspace) => { workspace.run.id = "run-1"; workspace.run.stopAfterCurrent = true; } });
  await safe.controller.confirmStopNow();
  assert.deepEqual(safe.invokes, []);
  const ok = harness({ workspaceSetup: (workspace) => { workspace.run.id = "run-1"; } });
  await ok.controller.confirmStopNow();
  assert.deepEqual(ok.invokes, [{ command: "request_abort", args: { runId: "run-1" } }]);
  assert.deepEqual([ok.workspace.run.aborting, ok.host.abortConfirmOpen], [true, false]);
  assert.equal(ok.workspace.run.message, "Stop now requested. The source stays in place; this encode will not resume.");
  const bad = harness({ invokeImpl: () => { throw new Error("nope"); }, workspaceSetup: (workspace) => { workspace.run.id = "run-1"; } });
  await bad.controller.confirmStopNow();
  assert.equal(bad.workspace.run.aborting, false);
  assert.deepEqual(bad.calls.at(-1), ["notice", "error", "Could not stop the run: Error: nope"]);
});
