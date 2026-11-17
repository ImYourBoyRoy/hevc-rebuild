// ./desktop/tests/tool-paths.test.mjs
// Characterization tests for the tool paths module extracted from the route (helper slice): the six
// executable path drafts, saving and rechecking them, choosing or clearing one, the shortcut to the
// toolchain settings and copying the host setup command. Dialogs and the DOM are injected fakes.
// Run: pnpm test:tool-paths

import assert from "node:assert/strict";
import test from "node:test";
import { createToolPaths } from "../src/lib/tool-paths.ts";

function harness({ pick = null, hostOver = {}, persistResult = true, loadResult = true, clipboardFail = false, element = null } = {}) {
  const log = [];
  const calls = [];
  const dialogs = [];
  const written = [];
  const state = {
    ffmpegPath: "", ffprobePath: "", mediainfoPath: "", mkvpropeditPath: "", mkvmergePath: "", mkvextractPath: "", desktopShell: true, runId: "", runStarting: false,
    repairId: "", workflowBusy: false, hostSetupCommand: "sudo apt install ffmpeg", hostSetupShell: "Terminal", ...hostOver,
  };
  const host = state;
  host.setNotice = (message, kind = "info") => log.push([kind, message]);
  const actions = {
    persistPreferences: async (patch) => { calls.push(["persistPreferences", patch]); return persistResult; },
    loadEnvironment: async () => { calls.push(["loadEnvironment"]); return loadResult; },
    openDialog: async (options) => { dialogs.push(options); return pick; },
    openPage: (page) => calls.push(["openPage", page]),
    tick: async () => { calls.push(["tick"]); },
  };
  const env = {
    clipboard: { writeText: async (value) => { if (clipboardFail) throw new Error("denied"); written.push(value); } },
    getElementById: (id) => { calls.push(["getElementById", id]); return element; },
  };
  return { ctl: createToolPaths({ host, actions, env }), host, log, calls, dialogs, written };
}

test("setToolPathValue and toolPathValue address each of the six fields", () => {
  const h = harness();
  const keys = ["ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract"];
  keys.forEach((key) => h.ctl.setToolPathValue(key, `/bin/${key}`));
  assert.deepEqual(keys.map((key) => h.ctl.toolPathValue(key)), keys.map((key) => `/bin/${key}`));
  assert.deepEqual([h.host.ffmpegPath, h.host.mkvextractPath], ["/bin/ffmpeg", "/bin/mkvextract"]);
});

test("currentToolPaths trims values and maps blanks to null", () => {
  const h = harness({ hostOver: { ffmpegPath: "  /opt/ffmpeg  ", ffprobePath: "", mediainfoPath: " " } });
  assert.deepEqual(h.ctl.currentToolPaths(), { ffmpeg: "/opt/ffmpeg", ffprobe: null, mediainfo: null, mkvpropedit: null, mkvmerge: null, mkvextract: null });
});

test("applyToolPaths fills the drafts from saved paths and clears missing ones", () => {
  const h = harness({ hostOver: { mkvmergePath: "/old" } });
  h.ctl.applyToolPaths({ ffmpeg: "/a", ffprobe: null, mediainfo: "/c", mkvpropedit: null, mkvmerge: null, mkvextract: "/e" });
  assert.deepEqual([h.host.ffmpegPath, h.host.ffprobePath, h.host.mediainfoPath, h.host.mkvpropeditPath, h.host.mkvmergePath, h.host.mkvextractPath], ["/a", "", "/c", "", "", "/e"]);
});

test("saveToolPaths persists, rechecks the environment and confirms", async () => {
  const h = harness({ hostOver: { ffmpegPath: "/opt/ffmpeg" } });
  await h.ctl.saveToolPaths();
  assert.deepEqual(h.calls.map((call) => call[0]), ["persistPreferences", "loadEnvironment"]);
  assert.equal(h.calls[0][1].toolPaths.ffmpeg, "/opt/ffmpeg");
  assert.deepEqual(h.log, [["success", "Saved executable paths and checked the selected tools."]]);
  const noRecheck = harness();
  await noRecheck.ctl.saveToolPaths(false);
  assert.deepEqual(noRecheck.calls.map((call) => call[0]), ["persistPreferences"]);
  assert.equal(noRecheck.log.length, 1);
});

test("saveToolPaths stops when busy, when saving fails or when the recheck fails", async () => {
  const busy = harness({ hostOver: { workflowBusy: true } });
  await busy.ctl.saveToolPaths();
  assert.deepEqual([busy.calls.length, busy.log.length], [0, 0]);
  const notSaved = harness({ persistResult: false });
  await notSaved.ctl.saveToolPaths();
  assert.deepEqual([notSaved.calls.map((call) => call[0]), notSaved.log.length], [["persistPreferences"], 0]);
  const notChecked = harness({ loadResult: false });
  await notChecked.ctl.saveToolPaths();
  assert.deepEqual([notChecked.calls.map((call) => call[0]), notChecked.log.length], [["persistPreferences", "loadEnvironment"], 0]);
});

test("chooseToolPath stores the picked executable and saves", async () => {
  const h = harness({ pick: "/opt/ffmpeg" });
  await h.ctl.chooseToolPath("ffmpeg", "FFmpeg");
  assert.deepEqual(h.dialogs[0], { directory: false, multiple: false, title: "Choose FFmpeg executable" });
  assert.deepEqual([h.host.ffmpegPath, h.calls.map((call) => call[0])], ["/opt/ffmpeg", ["persistPreferences", "loadEnvironment"]]);
  const cancelled = harness({ pick: null });
  await cancelled.ctl.chooseToolPath("ffmpeg", "FFmpeg");
  assert.deepEqual([cancelled.host.ffmpegPath, cancelled.calls.length], ["", 0]);
  for (const hostOver of [{ desktopShell: false }, { runId: "r" }, { runStarting: true }, { repairId: "j" }]) {
    const blocked = harness({ pick: "/x", hostOver });
    await blocked.ctl.chooseToolPath("ffmpeg", "FFmpeg");
    assert.equal(blocked.dialogs.length, 0);
  }
});

test("clearToolPath blanks one path and saves unless busy; the ffmpeg shortcuts delegate", async () => {
  const h = harness({ hostOver: { mkvmergePath: "/bin/mkvmerge" } });
  await h.ctl.clearToolPath("mkvmerge");
  assert.deepEqual([h.host.mkvmergePath, h.calls.map((call) => call[0])], ["", ["persistPreferences", "loadEnvironment"]]);
  const busy = harness({ hostOver: { workflowBusy: true, mkvmergePath: "/bin/mkvmerge" } });
  await busy.ctl.clearToolPath("mkvmerge");
  assert.deepEqual([busy.host.mkvmergePath, busy.calls.length], ["/bin/mkvmerge", 0]);
  const ff = harness({ pick: "/x/ffmpeg", hostOver: { ffmpegPath: "/old" } });
  await ff.ctl.chooseFfmpeg();
  assert.equal(ff.host.ffmpegPath, "/x/ffmpeg");
  await ff.ctl.clearFfmpegOverride();
  assert.equal(ff.host.ffmpegPath, "");
});

test("openToolchainSettings opens Settings and scrolls to the capabilities section", async () => {
  const scrolls = [];
  const h = harness({ element: { scrollIntoView: (options) => scrolls.push(options) } });
  await h.ctl.openToolchainSettings();
  assert.deepEqual(h.calls.map((call) => call.slice(0, 2)), [["openPage", "settings"], ["tick"], ["getElementById", "environment-capabilities"]]);
  assert.deepEqual(scrolls, [{ behavior: "smooth", block: "start" }]);
  const missing = harness();
  await missing.ctl.openToolchainSettings();
  assert.equal(missing.calls.at(-1)[1], "environment-capabilities");
});

test("copyHostSetupCommand copies the command and names the shell, or explains the failure", async () => {
  const h = harness();
  await h.ctl.copyHostSetupCommand();
  assert.deepEqual([h.written, h.log], [["sudo apt install ffmpeg"], [["success", "Setup command copied. Paste it into Terminal."]]]);
  const failed = harness({ clipboardFail: true });
  await failed.ctl.copyHostSetupCommand();
  assert.deepEqual(failed.log, [["error", "Clipboard access is unavailable. Select the command and copy it instead."]]);
});
