// ./desktop/tests/file-pickers.test.mjs
// Characterization tests for the file pickers module extracted from the route (helper slice): the
// native folder and file dialogs that add queue sources or choose output folders, adding the typed
// path, opening a file location and cancelling a scan. The dialogs and native bridge are injected
// fakes. Run: pnpm test:file-pickers

import assert from "node:assert/strict";
import test from "node:test";
import { createFilePickers } from "../src/lib/file-pickers.ts";

function harness({ pick = null, invokeImpl = {}, hostOver = {} } = {}) {
  const log = [];
  const calls = [];
  const dialogs = [];
  const state = {
    desktopShell: true, root: "/typed", activeRoot: null, appPreferences: { root: "/saved", fileOutputDirs: {} }, youtubeOutputDir: "", runId: "", runStarting: false,
    repairId: "", hdrInstalling: false, scanning: false, workflowBusy: false, ...hostOver,
  };
  const host = state;
  host.setNotice = (message, kind = "info") => log.push([kind, message]);
  const invoke = async (command, args) => {
    calls.push(["invoke", command, args]);
    const impl = invokeImpl[command];
    if (impl === undefined) throw new Error(`unexpected ${command}`);
    return typeof impl === "function" ? impl(args) : impl;
  };
  const actions = {
    openDialog: async (options) => { dialogs.push(options); return typeof pick === "function" ? pick(options) : pick; },
    addQueueSources: async (paths) => { calls.push(["addQueueSources", paths]); },
    scanQueueSource: async (...args) => { calls.push(["scanQueueSource", ...args]); },
    persistPreferences: async (patch, message) => { calls.push(["persistPreferences", patch, message]); return true; },
  };
  return { ctl: createFilePickers({ invoke, host, actions }), host, log, calls, dialogs };
}

test("browseFolder is blocked when not on desktop or while something is running", async () => {
  for (const hostOver of [{ desktopShell: false }, { runId: "r" }, { runStarting: true }, { repairId: "j" }, { hdrInstalling: true }, { scanning: true }]) {
    const h = harness({ pick: ["/x"], hostOver });
    await h.ctl.browseFolder();
    assert.equal(h.dialogs.length, 0);
  }
});

test("browseFolder starts in the typed folder, then the active root, then the saved root", async () => {
  const typed = harness({ pick: ["/a", "/b"] });
  await typed.ctl.browseFolder();
  assert.deepEqual(typed.dialogs[0], { directory: true, multiple: true, title: "Add media folder(s) to the queue", defaultPath: "/typed" });
  assert.deepEqual(typed.calls.at(-1), ["addQueueSources", ["/a", "/b"]]);
  const active = harness({ pick: "/one", hostOver: { root: " ", activeRoot: "/active" } });
  await active.ctl.browseFolder();
  assert.equal(active.dialogs[0].defaultPath, "/active");
  assert.deepEqual(active.calls.at(-1), ["addQueueSources", ["/one"]]);
  const saved = harness({ pick: null, hostOver: { root: "", activeRoot: null } });
  await saved.ctl.browseFolder();
  assert.deepEqual([saved.dialogs[0].defaultPath, saved.calls.some((call) => call[0] === "addQueueSources")], ["/saved", false]);
});

test("browseFolder turns a file path into its folder and falls back to the default media root", async () => {
  const file = harness({ pick: [], hostOver: { root: "/media/show/ep.MKV" } });
  await file.ctl.browseFolder();
  assert.equal(file.dialogs[0].defaultPath, "/media/show");
  const windows = harness({ pick: [], hostOver: { root: "C:\\media\\ep.mp4" } });
  await windows.ctl.browseFolder();
  assert.equal(windows.dialogs[0].defaultPath, "C:\\media");
  const bare = harness({ pick: [], invokeImpl: { default_media_root: "/home/u/Videos" }, hostOver: { root: "ep.mkv" } });
  await bare.ctl.browseFolder();
  assert.deepEqual([bare.dialogs[0].defaultPath, bare.calls[0]], ["/home/u/Videos", ["invoke", "default_media_root", undefined]]);
  const failing = harness({ pick: [], invokeImpl: { default_media_root: () => { throw new Error("none"); } }, hostOver: { root: "", appPreferences: { root: null, fileOutputDirs: {} } } });
  await failing.ctl.browseFolder();
  assert.equal(failing.dialogs[0].defaultPath, undefined);
});

test("browseFile offers video files and queues the picked paths", async () => {
  const h = harness({ pick: ["/a.mkv"] });
  await h.ctl.browseFile();
  assert.deepEqual(h.dialogs[0], { directory: false, multiple: true, title: "Add media file(s) to the queue", filters: [{ name: "Video files", extensions: ["mkv", "mp4", "m4v", "mov", "avi", "ts", "m2ts", "mts", "webm"] }] });
  assert.deepEqual(h.calls.at(-1), ["addQueueSources", ["/a.mkv"]]);
  const none = harness({ pick: null });
  await none.ctl.browseFile();
  assert.equal(none.calls.length, 0);
  const blocked = harness({ pick: ["/a"], hostOver: { scanning: true } });
  await blocked.ctl.browseFile();
  assert.equal(blocked.dialogs.length, 0);
});

test("chooseYoutubeOutput saves the chosen folder and confirms", async () => {
  const h = harness({ pick: "/exports" });
  await h.ctl.chooseYoutubeOutput();
  assert.equal(h.host.youtubeOutputDir, "/exports");
  assert.deepEqual(h.calls.at(-1), ["persistPreferences", { youtubeOutputDir: "/exports" }, undefined]);
  assert.deepEqual(h.log, [["success", "YouTube exports will be saved to the selected folder."]]);
  const cancelled = harness({ pick: null });
  await cancelled.ctl.chooseYoutubeOutput();
  assert.deepEqual([cancelled.host.youtubeOutputDir, cancelled.log.length], ["", 0]);
  const blocked = harness({ pick: "/x", hostOver: { runStarting: true } });
  await blocked.ctl.chooseYoutubeOutput();
  assert.equal(blocked.dialogs.length, 0);
});

test("chooseRebuildOutput saves the default output folder unless the workspace is busy", async () => {
  const h = harness({ pick: "/out" });
  await h.ctl.chooseRebuildOutput();
  assert.deepEqual(h.calls.at(-1), ["persistPreferences", { rebuildOutputDir: "/out" }, "Default rebuild outputs will be saved to this folder. Original files will stay where they are."]);
  for (const hostOver of [{ workflowBusy: true }, { desktopShell: false }]) {
    const blocked = harness({ pick: "/out", hostOver });
    await blocked.ctl.chooseRebuildOutput();
    assert.equal(blocked.dialogs.length, 0);
  }
  const cancelled = harness({ pick: null });
  await cancelled.ctl.chooseRebuildOutput();
  assert.equal(cancelled.calls.length, 0);
});

test("chooseFileOutput adds a per-file folder, capped at 512 files", async () => {
  const h = harness({ pick: "/out/a" });
  await h.ctl.chooseFileOutput("/lib/Film.mkv");
  assert.equal(h.dialogs[0].title, "Choose output folder for Film.mkv");
  assert.deepEqual(h.calls.at(-1), ["persistPreferences", { fileOutputDirs: { "/lib/Film.mkv": "/out/a" } }, undefined]);
  const full = harness({ pick: "/out/z", hostOver: { appPreferences: { root: null, fileOutputDirs: Object.fromEntries(Array.from({ length: 512 }, (_, index) => [`/f${index}`, "/o"])) } } });
  await full.ctl.chooseFileOutput("/new.mkv");
  assert.deepEqual([full.calls.length, full.log], [0, [["error", "Per-file output folders are limited to 512 media files."]]]);
  const replace = harness({ pick: "/out/z", hostOver: { appPreferences: { root: null, fileOutputDirs: { "/f0": "/o" } } } });
  await replace.ctl.chooseFileOutput("/f0");
  assert.deepEqual(replace.calls.at(-1)[1], { fileOutputDirs: { "/f0": "/out/z" } });
  const blocked = harness({ pick: "/x", hostOver: { workflowBusy: true } });
  await blocked.ctl.chooseFileOutput("/a.mkv");
  assert.equal(blocked.dialogs.length, 0);
});

test("clearFileOutput saves the map without that file", () => {
  const h = harness({ hostOver: { appPreferences: { root: null, fileOutputDirs: { "/a": "/o1", "/b": "/o2" } } } });
  h.ctl.clearFileOutput("/a");
  assert.deepEqual(h.calls.at(-1), ["persistPreferences", { fileOutputDirs: { "/b": "/o2" } }, undefined]);
});

test("analyzeFolder queues the trimmed typed path or asks for one", async () => {
  const h = harness({ hostOver: { root: "  /lib/movies  " } });
  await h.ctl.analyzeFolder();
  assert.deepEqual(h.calls.at(-1), ["scanQueueSource", "/lib/movies", true, true]);
  const empty = harness({ hostOver: { root: "   " } });
  await empty.ctl.analyzeFolder();
  assert.deepEqual([empty.calls.length, empty.log], [0, [["error", "Choose a media file or folder to add to the queue."]]]);
});

test("refreshQueueSource rescans and activates the source", () => {
  const h = harness();
  h.ctl.refreshQueueSource("/lib");
  assert.deepEqual(h.calls.at(-1), ["scanQueueSource", "/lib", true, true]);
});

test("openFileLocation refuses empty and virtual season paths and reports a failure", async () => {
  const h = harness({ invokeImpl: { open_file_location: null } });
  await h.ctl.openFileLocation("/lib/a.mkv");
  assert.deepEqual(h.calls.at(-1), ["invoke", "open_file_location", { path: "/lib/a.mkv" }]);
  for (const path of ["", "/lib/Show::season:2"]) {
    const bad = harness();
    await bad.ctl.openFileLocation(path);
    assert.deepEqual([bad.calls.length, bad.log], [0, [["error", "That folder is not available."]]]);
  }
  const failed = harness({ invokeImpl: { open_file_location: () => { throw new Error("no file manager"); } } });
  await failed.ctl.openFileLocation("/lib/a.mkv");
  assert.deepEqual(failed.log, [["error", "Error: no file manager"]]);
});

test("cancelScan asks the backend to stop and only reports a failure while scanning", async () => {
  const h = harness({ invokeImpl: { cancel_scan: null } });
  await h.ctl.cancelScan();
  assert.deepEqual(h.log, [["info", "Stopping the scan after the current file…"]]);
  const failedScanning = harness({ invokeImpl: { cancel_scan: () => { throw new Error("no scan"); } }, hostOver: { scanning: true } });
  await failedScanning.ctl.cancelScan();
  assert.deepEqual(failedScanning.log, [["error", "Could not cancel the scan: Error: no scan"]]);
  const failedIdle = harness({ invokeImpl: { cancel_scan: () => { throw new Error("no scan"); } } });
  await failedIdle.ctl.cancelScan();
  assert.deepEqual(failedIdle.log, []);
});
