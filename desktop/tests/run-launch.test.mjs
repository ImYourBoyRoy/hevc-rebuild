// ./desktop/tests/run-launch.test.mjs
// Characterization tests for the run launch controller extracted from the route (run slice B):
// the pre-start checks, the output-preview review, and the interrupted-file recovery review.
// Route state is a plain fake host, the native bridge a fake `invoke`. Run: pnpm test:run-launch

import assert from "node:assert/strict";
import test from "node:test";
import { createRunLaunchController } from "../src/lib/run-launch.ts";
import { initialWorkspaceState } from "../src/lib/workspace-state.ts";

const entry = (path, extra = {}) => ({ path, name: path.split("/").at(-1), size_bytes: 1, status: "Ready", ...extra });
const batch = (root, paths) => ({ source: { root }, entries: paths.map((path) => entry(path)) });
const state = (status, extra = {}) => ({ status, mode: "full", message: null, outputPath: null, reportPath: null, reportStatus: null, updatedAtMs: 1, ...extra });
const preview = (extra = {}) => ({ mode: "full", sourceCount: 1, fileCount: 1, eligible: true, targets: [], omittedTargetCount: 0, collisions: [], collisionCount: 0, omittedCollisionCount: 0, testOutputsMayReceiveUniqueSuffixes: false, ...extra });
const report = (csv, rows) => ({ csv_path: csv, log_path: null, created: 1, counts: {}, rows });

function harness({ invokeImpl, desktop = true, hostOverrides = {}, runStates = {}, persistResults = [] } = {}) {
  const calls = [];
  const workspace = initialWorkspaceState();
  const host = {
    canStart: true, mode: "full", settings: { id: "s" }, unresolvedSelectedPaths: 0, runBatches: [batch("/r", ["/r/a.mkv"])], reports: [], queuedEntries: [entry("/r/a.mkv")],
    appPreferences: { rebuildOutputDir: "/out", fileOutputDirs: { "/r/a.mkv": "/special", "/other": "/x" } },
    fullAck: true, pendingRunFullConfirmed: false, recoveryReviewedPaths: new Set(), recoveryReviewItems: [], recoveryReviewIndex: 0, recoveryProbeEntry: null,
    recoveryProbeError: "", recoveryProbeBusy: false, recoveryProbeAttempted: false, recoveryAcknowledged: false, pendingRecoveryFullConfirmed: false,
    recoveryReviewOpen: false, outputPreview: null, outputPreviewOpen: false, outputPreviewBusy: false,
    get recoveryCurrentItem() { return this.recoveryReviewItems[this.recoveryReviewIndex] ?? null; },
    setNotice: (message, kind = "info") => calls.push(["notice", kind, message]),
    loadReports: async () => { calls.push(["loadReports"]); },
    currentToolPaths: () => ({ ffmpeg: null }),
    selectFilesToReview: () => calls.push(["selectFilesToReview"]),
    ...hostOverrides,
  };
  const invokes = [];
  const invoke = async (command, args) => {
    invokes.push({ command, args });
    if (invokeImpl) return invokeImpl(command, args);
    if (command === "preview_queue_output_targets") return preview({ mode: args.request.mode });
    return null;
  };
  let persistIndex = 0;
  const saved = [];
  const persistence = {
    persistQueueRunStates: async (updates) => { saved.push(updates); calls.push(["persistStates", Object.fromEntries(Object.entries(updates).map(([path, item]) => [path, item.status]))]); return persistResults[persistIndex++] ?? true; },
  };
  const runs = {
    queueRunStateForPath: (path) => runStates[path],
    beginQueuedRun: async (confirmed) => { calls.push(["beginQueuedRun", confirmed]); },
  };
  const controller = createRunLaunchController({ invoke, isDesktop: () => desktop, workspace, host, persistence, runs });
  return { controller, host, workspace, calls, invokes, saved };
}

test("beginRun explains what is missing per mode, and otherwise resets the full-rebuild acknowledgement before launching", async () => {
  const cases = [["youtube", "YouTube export"], ["integrity", "this integrity check"], ["full", "this rebuild task"], ["check", "this rebuild task"]];
  for (const [mode, label] of cases) {
    const h = harness({ hostOverrides: { canStart: false, mode } });
    await h.controller.beginRun();
    assert.deepEqual(h.calls, [["notice", "error", `Analyze the selected media and resolve the tools required for ${label} before starting.`]], mode);
  }
  const ok = harness({ hostOverrides: { mode: "check", fullAck: true } });
  await ok.controller.beginRun();
  assert.equal(ok.host.fullAck, false);
  assert.deepEqual(ok.calls, [["beginQueuedRun", false]]);
});

test("launchRun stops early without settings, with unresolved selections, or with nothing to run", async () => {
  const none = harness({ hostOverrides: { settings: null } });
  await none.controller.launchRun(false);
  assert.deepEqual(none.calls, []);
  const unresolved = harness({ hostOverrides: { unresolvedSelectedPaths: 2 } });
  await unresolved.controller.launchRun(false);
  assert.deepEqual(unresolved.calls, [["notice", "error", "Some selected files no longer belong to a completed scan. Refresh or remove those source folders before starting."]]);
  const empty = harness({ hostOverrides: { runBatches: [] } });
  await empty.controller.launchRun(false);
  assert.deepEqual(empty.calls, [["notice", "error", "Add and scan at least one source before starting a workflow."]]);
});

test("modes that write nothing start immediately with the given confirmation", async () => {
  for (const mode of ["check", "integrity", "youtube"]) {
    const h = harness({ hostOverrides: { mode } });
    await h.controller.launchRun(true);
    assert.deepEqual(h.calls, [["beginQueuedRun", true]], mode);
    assert.deepEqual(h.invokes, [], mode);
  }
});

test("writing modes request an output preview with the selected files and their folders, then wait for confirmation", async () => {
  const h = harness({ hostOverrides: { mode: "test" } });
  await h.controller.launchRun(true);
  assert.deepEqual(h.invokes, [{ command: "preview_queue_output_targets", args: { request: { mode: "test", sources: [{ root: "/r", files: ["/r/a.mkv"] }], outputDir: "/out", fileOutputDirs: { "/r/a.mkv": "/special" } } } }]);
  assert.equal(h.host.outputPreview.mode, "test");
  assert.deepEqual([h.host.outputPreviewOpen, h.host.outputPreviewBusy, h.host.pendingRunFullConfirmed], [true, false, true]);
  assert.deepEqual(h.calls, []);
});

test("a preview with collisions opens with an error notice naming how many", async () => {
  for (const [count, text] of [[1, "1 output collision must"], [3, "3 output collisions must"]]) {
    const h = harness({ invokeImpl: () => preview({ eligible: false, collisionCount: count }) });
    await h.controller.launchRun(false);
    assert.equal(h.host.outputPreviewOpen, true);
    assert.deepEqual(h.calls.at(-1), ["notice", "error", `${text} be resolved before this queue can start.`]);
  }
});

test("a failed preview reports the error, leaves no preview and does not mark the run pending", async () => {
  const h = harness({ invokeImpl: () => { throw new Error("no access"); }, hostOverrides: { pendingRunFullConfirmed: false } });
  await h.controller.launchRun(true);
  assert.deepEqual(h.calls, [["notice", "error", "Could not check the queue's output paths: Error: no access"]]);
  assert.deepEqual([h.host.outputPreview, h.host.outputPreviewOpen, h.host.outputPreviewBusy, h.host.pendingRunFullConfirmed], [null, false, false, false]);
});

test("unfinished files open the recovery review before any preview, with the report row found by path", async () => {
  const csv = "/rep/run.csv";
  const row = { input: "/r/a.mkv", status: "FAILED", output: "/out/a.mkv" };
  let h;
  h = harness({
    runStates: { "/r/a.mkv": state("interrupted", { reportPath: csv }), },
    hostOverrides: { runBatches: [batch("/r", ["/r/a.mkv", "/r/b.mkv"])], recoveryReviewIndex: 5, recoveryProbeAttempted: true, recoveryAcknowledged: true, recoveryProbeError: "old" },
  });
  h.host.loadReports = async () => { h.calls.push(["loadReports"]); h.host.reports = [report(csv, [row])]; };
  await h.controller.launchRun(true);
  assert.deepEqual(h.invokes, []);
  assert.equal(h.host.recoveryReviewOpen, true);
  assert.equal(h.host.recoveryReviewItems.length, 1);
  const item = h.host.recoveryReviewItems[0];
  assert.deepEqual([item.entry.path, item.report, item.targetPath, item.hasRecordedOutput], ["/r/a.mkv", row, "/out/a.mkv", true]);
  assert.deepEqual([h.host.recoveryReviewIndex, h.host.recoveryProbeAttempted, h.host.recoveryAcknowledged, h.host.recoveryProbeError, h.host.pendingRecoveryFullConfirmed], [0, false, false, "", true]);
  assert.equal(h.calls[0][0], "loadReports");
});

test("files already reviewed in this session, or finished ones, do not reopen the recovery review", async () => {
  const reviewed = harness({ runStates: { "/r/a.mkv": state("needs_review") }, hostOverrides: { recoveryReviewedPaths: new Set(["/r/a.mkv"]) } });
  await reviewed.controller.launchRun(false);
  assert.equal(reviewed.host.recoveryReviewOpen, false);
  assert.equal(reviewed.invokes[0].command, "preview_queue_output_targets");
  const done = harness({ runStates: { "/r/a.mkv": state("completed") } });
  await done.controller.launchRun(false);
  assert.equal(done.host.recoveryReviewOpen, false);
});

test("confirming the preview rechecks it and starts with the right confirmation for the mode", async () => {
  const full = harness({ hostOverrides: { outputPreview: preview(), fullAck: true, pendingRunFullConfirmed: false, outputPreviewOpen: true } });
  await full.controller.confirmOutputPreview();
  assert.equal(full.invokes[0].command, "preview_queue_output_targets");
  assert.deepEqual([full.host.outputPreviewOpen, full.host.outputPreviewBusy], [false, false]);
  assert.deepEqual(full.calls, [["beginQueuedRun", true]]);
  const meta = harness({ hostOverrides: { mode: "metadata", outputPreview: preview({ mode: "metadata" }), fullAck: true, pendingRunFullConfirmed: false } });
  await meta.controller.confirmOutputPreview();
  assert.deepEqual(meta.calls, [["beginQueuedRun", false]]);
});

test("confirming does nothing when there is no preview, it is busy or blocked, or the run cannot start", async () => {
  for (const overrides of [{ outputPreview: null }, { outputPreview: preview(), outputPreviewBusy: true }, { outputPreview: preview({ eligible: false }) }, { outputPreview: preview(), canStart: false }]) {
    const h = harness({ hostOverrides: overrides });
    await h.controller.confirmOutputPreview();
    assert.deepEqual([h.invokes, h.calls], [[], []], JSON.stringify(Object.keys(overrides)));
  }
});

test("confirming refuses a workflow that changed or a destination set that gained collisions", async () => {
  const changed = harness({ hostOverrides: { mode: "metadata", outputPreview: preview({ mode: "full" }) }, invokeImpl: () => preview({ mode: "full" }) });
  await changed.controller.confirmOutputPreview();
  assert.deepEqual(changed.calls, [["notice", "error", "The selected workflow changed during output review. Review its destinations again before starting."]]);
  const collided = harness({ hostOverrides: { outputPreview: preview() }, invokeImpl: () => preview({ eligible: false, collisionCount: 2 }) });
  await collided.controller.confirmOutputPreview();
  assert.deepEqual(collided.calls, [["notice", "error", "2 output collisions appeared during review. Resolve them before starting."]]);
  assert.equal(collided.host.outputPreviewBusy, false);
  const failed = harness({ hostOverrides: { outputPreview: preview() }, invokeImpl: () => { throw new Error("gone"); } });
  await failed.controller.confirmOutputPreview();
  assert.deepEqual(failed.calls, [["notice", "error", "Could not recheck the queue's output paths: Error: gone"]]);
});

const withItem = (extra = {}, hostExtra = {}) => {
  const item = { entry: entry("/r/a.mkv"), state: state("interrupted", { outputPath: "/out/a.mkv", reportStatus: "ok" }), report: null, targetPath: "/out/a.mkv", hasRecordedOutput: true, ...extra };
  return { recoveryReviewItems: [item], recoveryReviewOpen: true, ...hostExtra };
};

test("inspecting the recovery target scans it read-only and keeps the matching entry or the first one", async () => {
  const match = entry("/out/a.mkv", { marker: "match" });
  const h = harness({ hostOverrides: withItem(), invokeImpl: () => ({ entries: [entry("/other"), match] }) });
  await h.controller.inspectRecoveryTarget();
  assert.deepEqual(h.invokes[0], { command: "scan_folder", args: { options: { root: "/out/a.mkv", recursive: false, minimumSizeMib: 0, filenameIncludePatterns: [], filenameExcludePatterns: [], toolPaths: { ffmpeg: null } } } });
  assert.equal(h.host.recoveryProbeEntry, match);
  assert.deepEqual([h.host.recoveryProbeAttempted, h.host.recoveryProbeBusy, h.host.recoveryProbeError], [true, false, ""]);
  const first = harness({ hostOverrides: withItem(), invokeImpl: () => ({ entries: [entry("/x/first.mkv")] }) });
  await first.controller.inspectRecoveryTarget();
  assert.equal(first.host.recoveryProbeEntry.path, "/x/first.mkv");
  const none = harness({ hostOverrides: withItem(), invokeImpl: () => ({ entries: [] }) });
  await none.controller.inspectRecoveryTarget();
  assert.deepEqual([none.host.recoveryProbeEntry, none.host.recoveryProbeError], [null, "No readable media file was found at this path."]);
  const bad = harness({ hostOverrides: withItem(), invokeImpl: () => { throw new Error("ffprobe failed"); } });
  await bad.controller.inspectRecoveryTarget();
  assert.deepEqual([bad.host.recoveryProbeError, bad.host.recoveryProbeBusy], ["Error: ffprobe failed", false]);
});

test("inspecting is ignored in preview, while busy, or without a current item", async () => {
  for (const options of [{ desktop: false, hostOverrides: withItem() }, { hostOverrides: withItem({}, { recoveryProbeBusy: true }) }, { hostOverrides: {} }]) {
    const h = harness(options);
    await h.controller.inspectRecoveryTarget();
    assert.deepEqual(h.invokes, []);
  }
});

const ready = (extra = {}) => withItem(extra.item, { recoveryProbeAttempted: true, recoveryAcknowledged: true, recoveryProbeEntry: entry("/out/a.mkv"), ...extra.host });

test("finishing needs a probe and an acknowledgement; skipping records the file as reviewed and advances", async () => {
  const blocked = harness({ hostOverrides: withItem({}, { recoveryProbeAttempted: true }) });
  await blocked.controller.finishRecoveryReview(false);
  assert.deepEqual(blocked.calls, []);
  const two = ready({ host: { recoveryReviewItems: [withItem().recoveryReviewItems[0], withItem({ entry: entry("/r/b.mkv") }).recoveryReviewItems[0]] } });
  const h = harness({ hostOverrides: two });
  await h.controller.finishRecoveryReview(false);
  assert.deepEqual([...h.host.recoveryReviewedPaths], ["/r/a.mkv"]);
  assert.deepEqual([h.host.recoveryReviewIndex, h.host.recoveryProbeEntry, h.host.recoveryProbeAttempted, h.host.recoveryAcknowledged, h.host.recoveryReviewOpen], [1, null, false, false, true]);
  assert.deepEqual(h.calls, []);
});

test("finishing the last file closes the review, says so, and launches the run again", async () => {
  const h = harness({ hostOverrides: { ...ready(), pendingRecoveryFullConfirmed: true, mode: "check" } });
  await h.controller.finishRecoveryReview(false);
  assert.deepEqual([h.host.recoveryReviewOpen, h.host.recoveryReviewItems, h.host.recoveryAcknowledged], [false, [], false]);
  assert.deepEqual(h.calls, [
    ["notice", "info", "Recovery review is recorded for this session. Output paths will be checked again before the run starts."],
    ["beginQueuedRun", true],
  ]);
});

test("recording a file as complete needs a recorded output, a successful report and a readable output", async () => {
  const csvOk = { status: "OK" };
  const good = harness({ hostOverrides: { ...ready({ item: { report: csvOk } }), mode: "check" } });
  await good.controller.finishRecoveryReview(true);
  assert.deepEqual(good.calls[0], ["persistStates", { "/r/a.mkv": "completed" }]);
  assert.equal(good.workspace.queue.runStates["/r/a.mkv"], undefined);
  assert.deepEqual(good.calls.at(-2), ["notice", "info", "The reviewed successful output was recorded as complete."]);
  for (const bad of [
    ready({ item: { hasRecordedOutput: false, report: csvOk } }),
    ready({ item: { report: { status: "FAILED" } } }),
    ready({ item: { report: csvOk }, host: { recoveryProbeEntry: entry("/out/a.mkv", { status: "Analysis failed" }) } }),
  ]) {
    const h = harness({ hostOverrides: bad });
    await h.controller.finishRecoveryReview(true);
    assert.deepEqual(h.calls, []);
  }
});

test("a completed record keeps the report's output and mode, and a failed save keeps the review open", async () => {
  const item = { report: { status: " ok ", output: "/rep/out.mkv" }, state: state("interrupted", { mode: "test", reportPath: "/rep.csv", outputPath: "" }) };
  const h = harness({ hostOverrides: { ...ready({ item }), mode: "check" } });
  await h.controller.finishRecoveryReview(true);
  const completed = h.saved[0]["/r/a.mkv"];
  assert.deepEqual([completed.status, completed.mode, completed.outputPath, completed.reportPath, completed.reportStatus], ["completed", "test", "/rep/out.mkv", "/rep.csv", "OK"]);
  assert.equal(completed.message, "Recorded as complete after reviewing the successful report and readable output.");
  const failing = harness({ persistResults: [false], hostOverrides: { ...ready({ item: { report: { status: "OK" } } }) } });
  failing.workspace.queue.persistenceError = "ro";
  await failing.controller.finishRecoveryReview(true);
  assert.deepEqual(failing.calls.at(-1), ["notice", "error", "Could not save the reviewed run result. ro"]);
  assert.equal(failing.host.recoveryReviewOpen, true);
});

test("opening the interrupted review lists unfinished queued files, or selects them for review when none remain", async () => {
  const h = harness({
    runStates: { "/r/a.mkv": state("needs_review") },
    hostOverrides: { queuedEntries: [entry("/r/a.mkv"), entry("/r/b.mkv")] },
  });
  await h.controller.openInterruptedRecovery();
  assert.equal(h.calls[0][0], "loadReports");
  assert.deepEqual(h.host.recoveryReviewItems.map((item) => item.entry.path), ["/r/a.mkv"]);
  assert.deepEqual([h.host.recoveryReviewOpen, h.host.recoveryReviewIndex, h.host.pendingRecoveryFullConfirmed], [true, 0, false]);
  const none = harness({ hostOverrides: { queuedEntries: [entry("/r/b.mkv")] } });
  await none.controller.openInterruptedRecovery();
  assert.deepEqual(none.calls, [["loadReports"], ["selectFilesToReview"]]);
  assert.equal(none.host.recoveryReviewOpen, false);
});
