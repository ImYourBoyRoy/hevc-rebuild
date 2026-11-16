// ./desktop/tests/queue-controller.test.mjs
// Characterization tests for the queue controller extracted from the route (queue slice B):
// scanning, activation, removal, reordering and restoring the saved queue. The route state is
// replaced by a plain fake host and the native bridge by a fake `invoke`, so every branch runs
// without Tauri. Run: pnpm test:queue-controller

import assert from "node:assert/strict";
import test from "node:test";
import { createQueueController } from "../src/lib/queue-controller.ts";
import { initialWorkspaceState } from "../src/lib/workspace-state.ts";

const entry = (path, extra = {}) => ({ path, name: path.split("/").at(-1), relative_path: path.split("/").at(-1), size_bytes: 100, modified_unix_ms: 1, ...extra });
const scan = (root, paths, extra = {}) => ({
  root, entries: paths.map((path) => entry(path)), candidate_count: paths.length, recursive: true, minimum_size_mib: 5, interrupted: false,
  settings: { id: "scan-settings" }, settings_sources: ["/cfg"], work_directory: { directory: "/w" }, environment: { ready: true }, ...extra,
});
const source = (root, paths = [], extra = {}) => ({
  root, status: "ready", error: null, updatedAtMs: 1, scanResult: paths.length ? scan(root, paths) : null,
  changedFiles: [], missingFiles: [], excludedPaths: [], entryOrder: [], ...extra,
});

function harness({ desktop = true, scanResponse, hostOverrides = {}, workspaceSetup } = {}) {
  const calls = [];
  const workspace = initialWorkspaceState();
  workspace.queue.loaded = true;
  workspaceSetup?.(workspace);
  const host = {
    root: "", scanResult: null, settings: null, settingsRoot: "", settingsBaseline: "", librarySettingsBaseline: "", settingsSources: [], workDirectory: null,
    environment: null, recursive: true, minimumSizeMib: "5", stopOnError: false, mode: "check", page: "library", scanning: false, scanProgress: null,
    metadataLookupClaimed: true, metadataLookupPending: false, metadataLookupStatus: { stale: true }, search: "x", mediaPage: 3, libraryBrowse: { seriesRoot: "/s", season: 1 },
    fileTuneOverrides: {}, fileStreamOverrides: {}, queueMatchSuggestions: {}, queueMovieReviews: {}, queueSeriesReviews: {},
    appPreferences: { filenameIncludePatterns: ["inc"], filenameExcludePatterns: ["old"], fileOutputDirs: {} },
    settingsChanged: false, workflowBusy: false, repairId: "", hdrInstalling: false,
    setNotice: (message, kind = "info") => calls.push(["notice", kind, message]),
    persistPreferences: async (patch) => { calls.push(["prefs", patch]); return true; },
    openPage: (page) => calls.push(["openPage", page]),
    currentToolPaths: () => ({ ffmpeg: null }),
    loadSettings: async (root) => { calls.push(["loadSettings", root]); },
    sourceNeedsMetadataLookup: () => false,
    recoverInterruptedSession: async () => { calls.push(["recover"]); },
    ...hostOverrides,
  };
  const invokes = [];
  const invoke = async (command, args) => {
    invokes.push({ command, args });
    if (command === "scan_folder") {
      if (scanResponse instanceof Error) throw scanResponse;
      return typeof scanResponse === "function" ? scanResponse(args) : scanResponse;
    }
    if (command === "load_library_queue") return scanResponse;
    return null;
  };
  const persistence = {
    persistQueueSources: async () => { calls.push(["persist", workspace.queue.sources.map((item) => `${item.root}:${item.status}`)]); return true; },
    persistQueueSelection: async () => { calls.push(["persistSelection"]); },
  };
  const controller = createQueueController({ invoke, isDesktop: () => desktop, workspace, host, persistence });
  return { controller, host, workspace, calls, invokes };
}

test("scanning needs the desktop app and a path, and is skipped while busy", async () => {
  const preview = harness({ desktop: false });
  await preview.controller.scanQueueSource("/r", true, false);
  assert.deepEqual(preview.calls, [["notice", "error", "Open HEVC Rebuild Studio from the desktop app to inspect this computer and connect to the media tools."]]);
  const blank = harness();
  await blank.controller.scanQueueSource("   ", true, false);
  assert.deepEqual(blank.calls, [["notice", "error", "Choose a media file or folder first."]]);
  for (const busy of [{ scanning: true }, { repairId: "job" }, { hdrInstalling: true }]) {
    const h = harness({ hostOverrides: busy });
    await h.controller.scanQueueSource("/r", true, false);
    assert.deepEqual([h.calls, h.invokes], [[], []], JSON.stringify(busy));
  }
  const running = harness({ workspaceSetup: (workspace) => { workspace.run.id = "run"; } });
  await running.controller.scanQueueSource("/r", true, false);
  assert.deepEqual(running.invokes, []);
});

test("an already scanned source is only activated, with a hint, unless refresh is requested", async () => {
  const h = harness({ workspaceSetup: (workspace) => { workspace.queue.sources = [source("/r", ["/r/a"])]; } });
  await h.controller.scanQueueSource("/r", true, false);
  assert.deepEqual(h.invokes, []);
  assert.equal(h.host.root, "/r");
  assert.deepEqual(h.calls.at(-1), ["notice", "info", "This source is already in the queue. Use Refresh on its card to scan it again."]);
  const inactive = harness({ workspaceSetup: (workspace) => { workspace.queue.sources = [source("/r", ["/r/a"])]; } });
  await inactive.controller.scanQueueSource("/r", false, false);
  assert.equal(inactive.host.root, "");
});

test("a new scan queues the source, calls scan_folder with clamped options and activates the result", async () => {
  const response = scan("/r", ["/r/a", "/r/b"]);
  const h = harness({ scanResponse: response, hostOverrides: { minimumSizeMib: "99999", recursive: false, stopOnError: true } });
  await h.controller.scanQueueSource(" /r ", true, false);
  const call = h.invokes.find((item) => item.command === "scan_folder");
  assert.deepEqual(call.args, { options: { root: "/r", recursive: false, minimumSizeMib: 4096, filenameIncludePatterns: ["inc"], filenameExcludePatterns: ["old"], toolPaths: { ffmpeg: null } } });
  assert.deepEqual(h.workspace.queue.sources.map((item) => [item.root, item.status, item.scanResult.candidate_count, item.entryOrder]), [["/r", "ready", 2, ["/r/a", "/r/b"]]]);
  assert.deepEqual(h.calls[0], ["notice", "info", "Scanning this source. Other queued folders stay available."]);
  assert.deepEqual(h.calls[1], ["persist", ["/r:scanning"]]);
  assert.deepEqual(h.calls[2], ["prefs", { root: "/r", recursive: false, minimumSizeMib: "99999", stopOnError: true }]);
  assert.equal(h.workspace.queue.activeRoot, "/r");
  assert.equal(h.host.root, "/r");
  assert.equal(h.host.scanResult.entries.length, 2);
  assert.equal(h.host.recursive, true);
  assert.equal(h.host.minimumSizeMib, "5");
  assert.deepEqual([h.host.settingsRoot, h.host.settingsBaseline, h.host.librarySettingsBaseline], ["/r", JSON.stringify({ id: "scan-settings" }), JSON.stringify({ id: "scan-settings" })]);
  assert.deepEqual([h.host.settings, h.host.settingsSources, h.host.workDirectory, h.host.environment], [{ id: "scan-settings" }, ["/cfg"], { directory: "/w" }, { ready: true }]);
  assert.ok(h.calls.some((item) => item[0] === "openPage" && item[1] === "library"));
  assert.deepEqual(h.calls.at(-1), ["notice", "success", "2 media files scanned and saved in the queue."]);
  assert.equal(h.host.scanning, false);
  assert.equal(h.host.scanProgress, null);
  assert.equal(h.host.metadataLookupStatus, null);
});

test("a scan without activation keeps the current view, and the YouTube page is not navigated away from", async () => {
  const quiet = harness({ scanResponse: scan("/r", ["/r/a"]) });
  await quiet.controller.scanQueueSource("/r", false, false);
  assert.equal(quiet.host.scanResult, null);
  assert.equal(quiet.workspace.queue.activeRoot, null);
  assert.deepEqual(quiet.calls.at(-1), ["notice", "success", "1 media file scanned and saved in the queue."]);
  const youtube = harness({ scanResponse: scan("/r", ["/r/a"]), hostOverrides: { page: "youtube", mode: "youtube" } });
  await youtube.controller.scanQueueSource("/r", true, false);
  assert.equal(youtube.calls.some((item) => item[0] === "openPage"), false);
  assert.equal(youtube.host.mode, "youtube");
  const leaving = harness({ scanResponse: scan("/r", ["/r/a"]), hostOverrides: { page: "library", mode: "youtube" } });
  await leaving.controller.scanQueueSource("/r", true, false);
  assert.equal(leaving.host.mode, "check");
});

test("unsaved run settings for the same source survive its own refresh", async () => {
  const mine = { id: "edited" };
  const h = harness({
    scanResponse: scan("/r", ["/r/a"]),
    hostOverrides: { settings: mine, settingsChanged: true, settingsRoot: "/r" },
    workspaceSetup: (workspace) => { workspace.queue.sources = [source("/r", ["/r/a"])]; },
  });
  await h.controller.scanQueueSource("/r", true, true);
  assert.equal(h.host.settings, mine);
  const other = harness({ scanResponse: scan("/r", ["/r/a"]), hostOverrides: { settings: mine, settingsChanged: true, settingsRoot: "/elsewhere" } });
  await other.controller.scanQueueSource("/r", true, false);
  assert.deepEqual(other.host.settings, { id: "scan-settings" });
});

test("a scan failure marks a new source failed and an existing one ready, and always clears the scanning flag", async () => {
  const fresh = harness({ scanResponse: new Error("no access") });
  await fresh.controller.scanQueueSource("/r", true, false);
  assert.deepEqual(fresh.workspace.queue.sources.map((item) => [item.status, item.error]), [["failed", "Error: no access"]]);
  assert.deepEqual(fresh.calls.at(-1), ["notice", "error", "Media scan failed: Error: no access"]);
  assert.equal(fresh.host.scanning, false);
  const existing = harness({ scanResponse: new Error("gone"), workspaceSetup: (workspace) => { workspace.queue.sources = [source("/r", ["/r/a"])]; } });
  await existing.controller.scanQueueSource("/r", true, true);
  assert.deepEqual(existing.workspace.queue.sources.map((item) => item.status), ["ready"]);
});

test("an interrupted refresh with nothing left restores the previous analysis", async () => {
  const h = harness({
    scanResponse: scan("/r", [], { interrupted: true }),
    workspaceSetup: (workspace) => { workspace.queue.sources = [source("/r", ["/r/a"], { status: "ready" })]; },
  });
  await h.controller.scanQueueSource("/r", true, true);
  assert.equal(h.workspace.queue.sources[0].status, "ready");
  assert.equal(h.workspace.queue.sources[0].scanResult.entries.length, 1);
  assert.deepEqual(h.calls.at(-1), ["notice", "info", "Scan stopped. The previous analysis is unchanged."]);
  const first = harness({ scanResponse: scan("/r", [], { interrupted: true }) });
  await first.controller.scanQueueSource("/r", true, false);
  assert.equal(first.workspace.queue.sources[0].status, "interrupted");
  assert.deepEqual(first.calls.at(-1), ["notice", "info", "Scan stopped. 0 analyzed files remain in the queue."]);
});

test("refreshing keeps hidden files hidden and the saved order, and drops state for vanished files", async () => {
  const h = harness({
    scanResponse: scan("/r", ["/r/c", "/r/b", "/r/a", "/r/hidden"]),
    workspaceSetup: (workspace) => {
      workspace.queue.sources = [source("/r", ["/r/a", "/r/b"], { excludedPaths: ["/r/hidden"], entryOrder: ["/r/b", "/r/a"] })];
      workspace.queue.selectedPaths = ["/r/a", "/r/gone"];
    },
    hostOverrides: { fileTuneOverrides: { "/r/a": "none", "/r/gone": "none" }, fileStreamOverrides: { "/r/a": [1] }, queueMatchSuggestions: { "/r/a": {}, "/r/gone": {}, "/elsewhere/z": {} } },
  });
  await h.controller.scanQueueSource("/r", true, true);
  assert.deepEqual(h.workspace.queue.sources[0].scanResult.entries.map((item) => item.path), ["/r/b", "/r/a", "/r/c"]);
  assert.deepEqual(h.workspace.queue.sources[0].excludedPaths, ["/r/hidden"]);
  assert.deepEqual(h.workspace.queue.selectedPaths, ["/r/a"]);
  assert.deepEqual(Object.keys(h.host.fileTuneOverrides), ["/r/a"]);
  assert.deepEqual(h.host.fileStreamOverrides, {});
  // A refresh discards every suggestion under the refreshed root so lookups run again, and suggestions for
  // paths that are not queued anywhere are dropped as well.
  assert.deepEqual(Object.keys(h.host.queueMatchSuggestions), []);
});

test("a changed source flips its recorded run state to needs_review, an unchanged one keeps it", async () => {
  const h = harness({
    scanResponse: scan("/r", [], {}),
    workspaceSetup: (workspace) => { workspace.queue.sources = [source("/r", ["/r/same", "/r/changed"])]; },
  });
  h.workspace.queue.runStates = {
    "/r/same": { status: "completed", mode: "full", message: null, outputPath: null, reportPath: "/rep", reportStatus: "OK", updatedAtMs: 1 },
    "/r/changed": { status: "failed", mode: null, message: "x", outputPath: null, reportPath: "/rep2", reportStatus: "FAILED", updatedAtMs: 1 },
  };
  const response = scan("/r", ["/r/same", "/r/changed"]);
  response.entries[1].modified_unix_ms = 2;
  const again = harness({
    scanResponse: response,
    hostOverrides: { mode: "test" },
    workspaceSetup: (workspace) => { workspace.queue.sources = [source("/r", ["/r/same", "/r/changed"])]; workspace.queue.runStates = h.workspace.queue.runStates; },
  });
  await again.controller.scanQueueSource("/r", true, true);
  assert.equal(again.workspace.queue.runStates["/r/same"].status, "completed");
  const changed = again.workspace.queue.runStates["/r/changed"];
  assert.deepEqual([changed.status, changed.mode, changed.reportPath, changed.reportStatus], ["needs_review", "test", "/rep2", "FAILED"]);
  assert.match(changed.message, /changed since the recorded run/);
});

test("per-file output folders for files that left the queue are dropped through the preferences", async () => {
  const h = harness({
    scanResponse: scan("/r", ["/r/a"]),
    hostOverrides: { appPreferences: { filenameIncludePatterns: [], filenameExcludePatterns: [], fileOutputDirs: { "/r/a": "/out", "/r/gone": "/out2" } } },
  });
  await h.controller.scanQueueSource("/r", false, false);
  assert.ok(h.calls.some((item) => item[0] === "prefs" && JSON.stringify(item[1]) === JSON.stringify({ fileOutputDirs: { "/r/a": "/out" } })));
});

test("activating a scanned source mirrors its scan into the view; an unscanned one loads settings instead", () => {
  const h = harness({ workspaceSetup: (workspace) => { workspace.queue.sources = [source("/a", ["/a/1"], { changedFiles: ["/a/1"] }), source("/b")]; } });
  h.controller.activateQueueSource("/a");
  assert.equal(h.workspace.queue.activeRoot, "/a");
  assert.deepEqual([h.host.root, h.host.search, h.host.mediaPage, h.host.libraryBrowse, h.host.metadataLookupClaimed], ["/a", "", 1, null, false]);
  assert.equal(h.host.scanResult.entries[0].queue_stale, "changed");
  assert.deepEqual([h.host.metadataLookupPending, h.host.metadataLookupStatus], [false, null]);
  assert.deepEqual(h.calls.at(-1), ["prefs", { root: "/a" }]);
  h.controller.activateQueueSource("/b");
  assert.equal(h.host.scanResult, null);
  assert.deepEqual(h.calls.filter((item) => item[0] === "loadSettings"), [["loadSettings", "/b"]]);
  h.controller.activateQueueSource("/missing");
  assert.equal(h.workspace.queue.activeRoot, "/b");
});

test("activating the already active source does not reset the metadata lookup claim", () => {
  const h = harness({ workspaceSetup: (workspace) => { workspace.queue.sources = [source("/a", ["/a/1"])]; workspace.queue.activeRoot = "/a"; } });
  h.controller.activateQueueSource("/a");
  assert.equal(h.host.metadataLookupClaimed, true);
});

test("removing a source prunes its records, activates the next one and persists; the last one clears the view", () => {
  const h = harness({
    workspaceSetup: (workspace) => {
      workspace.queue.sources = [source("/a", ["/a/1"]), source("/b", ["/b/1"])];
      workspace.queue.activeRoot = "/a";
      workspace.queue.selectedPaths = ["/a/1", "/b/1"];
    },
  });
  h.controller.removeQueueSource("/a");
  assert.deepEqual(h.workspace.queue.sources.map((item) => item.root), ["/b"]);
  assert.deepEqual(h.workspace.queue.selectedPaths, ["/b/1"]);
  assert.equal(h.workspace.queue.activeRoot, "/b");
  assert.equal(h.host.root, "/b");
  assert.deepEqual(h.calls.at(-1), ["persist", ["/b:ready"]]);
  h.controller.removeQueueSource("/b");
  assert.deepEqual([h.workspace.queue.activeRoot, h.host.scanResult, h.host.root], [null, null, ""]);
  const busy = harness({ hostOverrides: { workflowBusy: true }, workspaceSetup: (workspace) => { workspace.queue.sources = [source("/a", ["/a/1"])]; } });
  busy.controller.removeQueueSource("/a");
  assert.equal(busy.workspace.queue.sources.length, 1);
  busy.controller.removeQueueSource("/unknown");
});

test("source and file ordering helpers reorder, mirror the active scan and persist once", () => {
  const h = harness({
    workspaceSetup: (workspace) => {
      workspace.queue.sources = [source("/a", ["/a/1", "/a/2", "/a/3"]), source("/b")];
      workspace.queue.activeRoot = "/a";
    },
  });
  h.controller.moveQueueSource("/a", 1);
  assert.deepEqual(h.workspace.queue.sources.map((item) => item.root), ["/b", "/a"]);
  h.controller.moveQueueSource("/a", 1);
  h.controller.reorderQueueSources("/a", "/a");
  assert.equal(h.calls.filter((item) => item[0] === "persist").length, 1);
  h.controller.moveQueueFile("/a", "/a/1", 1);
  assert.deepEqual(h.host.scanResult.entries.map((item) => item.path), ["/a/2", "/a/1", "/a/3"]);
  h.controller.moveQueueFile("/a", "/a/3", 1);
  assert.equal(h.calls.filter((item) => item[0] === "persist").length, 2);
});

test("removing a file hides it, prunes its records and refreshes the view; restoring rescans the source", async () => {
  const h = harness({
    scanResponse: scan("/a", ["/a/1", "/a/2"]),
    workspaceSetup: (workspace) => {
      workspace.queue.sources = [source("/a", ["/a/1", "/a/2"])];
      workspace.queue.activeRoot = "/a";
      workspace.queue.selectedPaths = ["/a/1", "/a/2"];
    },
    hostOverrides: { fileTuneOverrides: { "/a/1": "none" } },
  });
  h.controller.removeQueueEntry("/a/1");
  assert.deepEqual(h.workspace.queue.sources[0].excludedPaths, ["/a/1"]);
  assert.deepEqual(h.workspace.queue.selectedPaths, ["/a/2"]);
  assert.deepEqual(h.host.fileTuneOverrides, {});
  assert.deepEqual(h.host.scanResult.entries.map((item) => item.path), ["/a/2"]);
  const before = h.calls.length;
  h.controller.removeQueueEntry("/a/missing");
  assert.equal(h.calls.length, before);
  h.controller.restoreExcludedQueueFiles("/a");
  assert.deepEqual(h.workspace.queue.sources[0].excludedPaths, []);
  await new Promise((resolve) => setImmediate(resolve));
  assert.ok(h.invokes.some((item) => item.command === "scan_folder"));
});

test("addQueueSources scans each distinct path in order and activates ones already scanned", async () => {
  const h = harness({
    scanResponse: (args) => scan(args.options.root, [`${args.options.root}/f`]),
    workspaceSetup: (workspace) => { workspace.queue.sources = [source("/known", ["/known/1"])]; },
  });
  await h.controller.addQueueSources(["/new", " /new ", "", "/known"]);
  assert.deepEqual(h.invokes.filter((item) => item.command === "scan_folder").map((item) => item.args.options.root), ["/new"]);
  assert.equal(h.host.root, "/known");
  assert.deepEqual(h.workspace.queue.sources.map((item) => item.root), ["/known", "/new"]);
});

const restorePayload = (queue, extra = {}) => ({ queueFile: "/q.json", warning: null, queue: { sources: [], selectedMediaPaths: [], runStates: {}, activeRoot: null, ...queue }, ...extra });

test("loadMediaQueue restores the saved queue and mirrors the active source into the view", async () => {
  const saved = source("/a", ["/a/1"]);
  delete saved.changedFiles;
  const h = harness({
    scanResponse: restorePayload({
      sources: [saved, source("/b")], selectedMediaPaths: ["/a/1"], activeRoot: "/b", fileTuneOverrides: { "/a/1": "none" },
      movieReviews: { "/a/1": { title: "T" } }, seriesReviews: { "/a": { title: "S" } }, matchSuggestions: { "/a/1": { status: "ready" } },
      runStates: { "/a/1": { status: "queued" } },
    }),
    workspaceSetup: (workspace) => { workspace.queue.loaded = false; },
  });
  await h.controller.loadMediaQueue();
  assert.equal(h.workspace.queue.loaded, true);
  assert.equal(h.workspace.queue.file, "/q.json");
  assert.deepEqual(h.workspace.queue.sources[0].changedFiles, []);
  assert.deepEqual(h.workspace.queue.selectedPaths, ["/a/1"]);
  assert.equal(h.workspace.queue.activeRoot, "/a");
  assert.deepEqual([h.host.root, h.host.settingsRoot, h.host.recursive, h.host.minimumSizeMib], ["/a", "/a", true, "5"]);
  assert.deepEqual(Object.keys(h.host.queueSeriesReviews["/a"]).sort(), ["episodeAssignments", "localSeasonPosterPaths", "seasonRecords", "title"]);
  assert.deepEqual(h.host.fileTuneOverrides, { "/a/1": "none" });
  assert.deepEqual(h.host.fileStreamOverrides, {});
  assert.equal(h.calls.some((item) => item[0] === "recover"), false);
});

test("loadMediaQueue starts recovery for interrupted runs, surfaces warnings and survives a load failure", async () => {
  const interrupted = harness({
    scanResponse: restorePayload({ runStates: { "/a/1": { status: "interrupted" } } }, { warning: "queue repaired" }),
    workspaceSetup: (workspace) => { workspace.queue.loaded = false; },
  });
  await interrupted.controller.loadMediaQueue();
  assert.ok(interrupted.calls.some((item) => item[0] === "recover"));
  assert.deepEqual(interrupted.calls.at(-1), ["notice", "error", "queue repaired"]);
  assert.equal(interrupted.workspace.queue.persistenceWarning, "queue repaired");
  const failing = harness({ workspaceSetup: (workspace) => { workspace.queue.loaded = false; } });
  failing.invokes.length = 0;
  const result = createQueueController({
    invoke: async () => { throw new Error("no queue"); }, isDesktop: () => true, workspace: failing.workspace, host: failing.host,
    persistence: { persistQueueSources: async () => true },
  });
  await result.loadMediaQueue();
  assert.equal(failing.workspace.queue.loaded, true);
  assert.equal(failing.workspace.queue.persistenceError, "Error: no queue");
  assert.deepEqual(failing.calls.at(-1), ["notice", "error", "Could not restore the media queue: Error: no queue"]);
});
