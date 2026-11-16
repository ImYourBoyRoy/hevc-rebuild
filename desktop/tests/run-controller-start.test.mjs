// ./desktop/tests/run-controller-start.test.mjs
// Run controller: queueing a run and starting source batches (start_run request, early events, failures). Run: pnpm test:run-controller

import assert from "node:assert/strict";
import test from "node:test";
import { source, batchOf, finished, harness, progressEvent, queuedHarness } from "./support/run-controller-harness.mjs";

test("beginQueuedRun records every file as queued, then starts the first batch", async () => {
  const h = queuedHarness({ recoveryReviewedPaths: new Set(["old"]) });
  await h.controller.beginQueuedRun(true);
  assert.equal(h.workspace.run.mode, "full");
  assert.equal(h.host.recoveryReviewedPaths.size, 0);
  assert.deepEqual(h.calls[0], ["persistStates", { "/r/a.mkv": "queued", "/r/b.mkv": "queued" }]);
  assert.deepEqual(h.calls[1], ["persistStates", { "/r/a.mkv": "running", "/r/b.mkv": "running" }]);
  assert.equal(h.host.pendingRunFullConfirmed, true);
  assert.equal(h.invokes.find((item) => item.command === "start_run").args.request.fullRebuildConfirmed, true);
  assert.equal(h.workspace.run.id, "run-1");
  assert.equal(h.host.activeRunBatchIndex, 0);
});

test("beginQueuedRun needs settings and stops with the persistence error when the plan cannot be saved", async () => {
  const none = queuedHarness({ settings: null });
  await none.controller.beginQueuedRun(false);
  assert.deepEqual([none.calls, none.invokes], [[], []]);
  const failing = queuedHarness({}, (workspace) => { workspace.queue.persistenceError = "disk full"; });
  failing.host.pendingRunBatches = [1];
  const failingRun = harness({
    hostOverrides: { mode: "full", runBatches: failing.host.runBatches }, persistResults: [false],
    workspaceSetup: (workspace) => { workspace.queue.persistenceError = "disk full"; },
  });
  await failingRun.controller.beginQueuedRun(false);
  assert.deepEqual(failingRun.host.pendingRunBatches, []);
  assert.deepEqual(failingRun.invokes, []);
  assert.deepEqual(failingRun.calls.at(-1), ["notice", "error", "The queue could not save its run plan, so the workflow did not start: disk full"]);
});

test("the start_run request carries the batch, mode-dependent overrides and trimmed folders", async () => {
  const h = queuedHarness({ mode: "full", youtubeOutputDir: "  /yt  ", stopOnError: true, fileStreamOverrides: { "/r/a.mkv": [1], "/other": [2] },
    fileSettingsOverrides: { "/r/b.mkv": { crf: 20 }, "/other": {} }, youtubeHdr10plusLossPaths: new Set(["/r/a.mkv", "/other"]),
    appPreferences: { rebuildOutputDir: "/out", fileOutputDirs: { "/r/a.mkv": "/special", "/other": "/x" }, filenameIncludePatterns: ["inc"], filenameExcludePatterns: ["exc"] } });
  await h.controller.beginQueuedRun(false);
  const request = h.invokes.find((item) => item.command === "start_run").args.request;
  assert.deepEqual(request, {
    root: "/r", recursive: true, minimumSizeMib: 5, toolPaths: { ffmpeg: null }, mode: "full", youtubeResolution: "1440p", youtubeQualityMode: "balanced",
    youtubeSourceMultiplier: 2, youtubeMode: "upscale", youtubeSdrCodec: "h264", youtubePreset: "slow", youtubeShrink: "auto", youtubeQualityCheck: "standard",
    youtubeOutputDir: "/yt", rebuildOutputDir: "/out", fileOutputDirs: { "/r/a.mkv": "/special" }, filenameIncludePatterns: ["inc"], filenameExcludePatterns: ["exc"],
    selectedFiles: ["/r/a.mkv", "/r/b.mkv"], settings: { id: "active-settings", metadata: { comment: "" } }, stopOnError: true, fullRebuildConfirmed: false, integrityLevel: null,
    fileTuneOverrides: { "/r/a.mkv": "none", "/r/b.mkv": "none" }, fileStreamOverrides: { "/r/a.mkv": [1] }, fileSettingsOverrides: { "/r/b.mkv": { crf: 20 } },
    filePosterOverrides: { "/r/a.mkv": "poster", "/r/b.mkv": "poster" }, fileMetadataOverrides: { "/r/a.mkv": { title: "T" }, "/r/b.mkv": { title: "T" } },
    allowYoutubeHdr10plusLossPaths: [],
  });
});

test("other modes drop the overrides and output folders they do not use", async () => {
  const youtube = queuedHarness({ mode: "youtube", youtubeOutputDir: "", youtubeHdr10plusLossPaths: new Set(["/r/a.mkv", "/other"]), fileStreamOverrides: { "/r/a.mkv": [1] } });
  await youtube.controller.beginQueuedRun(false);
  const yt = youtube.invokes.find((item) => item.command === "start_run").args.request;
  assert.deepEqual([yt.youtubeOutputDir, yt.rebuildOutputDir, yt.fileOutputDirs, yt.fileTuneOverrides, yt.fileStreamOverrides, yt.fileSettingsOverrides, yt.filePosterOverrides, yt.fileMetadataOverrides, yt.allowYoutubeHdr10plusLossPaths],
    [null, null, {}, {}, {}, {}, {}, {}, ["/r/a.mkv"]]);
  const integrity = queuedHarness({ mode: "integrity", integrityLevel: "full", fileStreamOverrides: { "/r/a.mkv": [1] } });
  await integrity.controller.beginQueuedRun(false);
  const check = integrity.invokes.find((item) => item.command === "start_run").args.request;
  assert.deepEqual([check.integrityLevel, check.rebuildOutputDir, check.filePosterOverrides, check.fileStreamOverrides, check.allowYoutubeHdr10plusLossPaths], ["full", null, {}, { "/r/a.mkv": [1] }, []]);
  const metadata = queuedHarness({ mode: "metadata" });
  await metadata.controller.beginQueuedRun(false);
  const md = metadata.invokes.find((item) => item.command === "start_run").args.request;
  assert.deepEqual([md.rebuildOutputDir, md.fileTuneOverrides, md.fileSettingsOverrides, md.filePosterOverrides["/r/a.mkv"]], ["/out", {}, {}, "poster"]);
});

test("a started batch records its run, announces it per mode and navigates to the right page", async () => {
  const cases = [["full", "run", /^Full rebuild 1\/1 · r · 2 files\./], ["test", "run", /^Short tests 1\/1/], ["youtube", "youtube", null], ["check", "run", null]];
  for (const [mode, page, pattern] of cases) {
    const h = queuedHarness({ mode });
    await h.controller.beginQueuedRun(false);
    assert.equal(h.workspace.run.id, "run-1", mode);
    assert.equal(h.workspace.run.starting, false, mode);
    assert.ok(h.calls.some((item) => item[0] === "openPage" && item[1] === page), mode);
    assert.deepEqual(h.calls.at(-1), ["notice", "success", "Run started. Live progress and diagnostics are shown below."], mode);
    if (pattern) assert.match(h.workspace.run.message, pattern, mode);
    assert.equal(h.workspace.run.lines.length, 0);
  }
});

test("events that arrive while start_run is pending are replayed, and an early finish is processed", async () => {
  const batches = [batchOf(source("/r", ["/r/a.mkv", "/r/b.mkv"]))];
  const invokeWithEvents = harness({
    hostOverrides: { mode: "full", runBatches: batches },
    invokeImpl: (command) => {
      if (command !== "start_run") return null;
      buffer2.line({ runId: "run-1", line: "early line" });
      buffer2.update(progressEvent({ phase: "Remuxing" }));
      buffer2.sample({ runId: "run-1", snapshot: { cpu: 1 } });
      return "run-1";
    },
  });
  const buffer2 = invokeWithEvents.runEventBuffer;
  await invokeWithEvents.controller.beginQueuedRun(false);
  assert.deepEqual(invokeWithEvents.workspace.run.lines, ["early line"]);
  assert.equal(invokeWithEvents.workspace.run.progress.phase, "Remuxing");
  assert.deepEqual(invokeWithEvents.workspace.run.resources, { cpu: 1 });
  assert.equal(invokeWithEvents.workspace.run.id, "run-1");
  const early = harness({
    hostOverrides: { mode: "full", runBatches: batches },
    invokeImpl: (command) => {
      if (command !== "start_run") return null;
      buffer3.finish(finished({ exitCode: 0 }));
      buffer3.finish(finished({ runId: "other-run", exitCode: 1 }));
      return "run-1";
    },
  });
  const buffer3 = early.runEventBuffer;
  await early.controller.beginQueuedRun(false);
  assert.equal(early.workspace.run.id, "");
  assert.equal(early.workspace.run.message, "Workflow finished. Review the report and any skipped files.");
});

test("starting a batch fails cleanly: settings missing, active batch not recorded, or start_run rejected", async () => {
  const other = source("/other", ["/other/a.mkv"]);
  other.scanResult.settings = undefined;
  const missing = harness({
    hostOverrides: { mode: "full", settings: null, pendingRunBatches: [batchOf(other)] },
    workspaceSetup: (workspace) => { workspace.queue.sources = [other]; workspace.queue.activeRoot = "/elsewhere"; },
  });
  await missing.controller.startNextRunBatch();
  assert.deepEqual(missing.calls.at(-1), ["notice", "error", "Settings are unavailable for /other. Rescan that source before running it."]);
  assert.deepEqual([missing.host.pendingRunBatches, missing.host.activeRunBatchIndex], [[], -1]);
  assert.deepEqual(missing.invokes, []);
  const none = harness({ hostOverrides: { pendingRunBatches: [] } });
  await none.controller.startNextRunBatch();
  assert.deepEqual([none.calls, none.invokes], [[], []]);
  const unsaved = harness({
    hostOverrides: { mode: "full", runBatches: [batchOf(source("/r", ["/r/a.mkv"]))] }, persistResults: [true, false],
    workspaceSetup: (workspace) => { workspace.queue.persistenceError = "ro"; },
  });
  await unsaved.controller.beginQueuedRun(false);
  assert.deepEqual(unsaved.invokes, []);
  assert.deepEqual(unsaved.calls.at(-1), ["notice", "error", "The queue could not record the active batch, so it was not started: ro"]);
  assert.deepEqual([unsaved.host.pendingRunBatches, unsaved.host.activeRunBatchIndex], [[], -1]);
  const rejected = harness({
    invokeImpl: () => { throw new Error("engine missing"); },
    hostOverrides: { mode: "full", runBatches: [batchOf(source("/r", ["/r/a.mkv", "/r/b.mkv"]))] },
  });
  await rejected.controller.beginQueuedRun(false);
  assert.equal(rejected.workspace.run.id, "");
  assert.equal(rejected.workspace.run.starting, false);
  assert.equal(rejected.workspace.run.message, "Could not start the selected workflow.");
  assert.deepEqual(rejected.calls.at(-2), ["persistStates", { "/r/a.mkv": "failed", "/r/b.mkv": "failed" }]);
  assert.equal(rejected.workspace.queue.runStates["/r/a.mkv"].message, "Error: engine missing");
  assert.deepEqual(rejected.calls.at(-1), ["notice", "error", "Error: engine missing"]);
  assert.deepEqual([rejected.host.pendingRunBatches, rejected.host.activeRunBatchIndex], [[], -1]);
});
