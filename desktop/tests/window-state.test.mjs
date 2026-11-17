// ./desktop/tests/window-state.test.mjs
// Characterization tests for the window-state controller extracted from the route (window-state
// slice B): theme publishing, the serialized preference writes, restoring the saved window size
// with monitor clamping, and debounced saving of resizes. The native window, monitor, `invoke`,
// storage and timers are all injected fakes. Run: pnpm test:window-state-controller

import assert from "node:assert/strict";
import test from "node:test";
import { createWindowStateController } from "../src/lib/window-state-controller.ts";

const saved = (extra = {}) => ({ width: 1600, height: 1000, maximized: false, ...extra });

function harness({ desktop = true, monitor = null, windowScale = 1, maximized = false, inner = { width: 1600, height: 1000 }, invokeImpl, windowErrors = {}, appPreferences, themeTransition } = {}) {
  const calls = [];
  const dataset = {};
  const stored = [];
  const host = {
    appPreferences: appPreferences ?? { theme: "light", lastPage: "library", windowState: saved(), metadataLanguage: "en-US" },
    preferenceStorage: null, theme: "light", metadataLanguageDraft: "en-US",
    setNotice: (message, kind = "info") => calls.push(["notice", kind, message]),
  };
  const windowLike = {
    scaleFactor: async () => windowScale,
    setSize: async (size) => { if (windowErrors.setSize) throw windowErrors.setSize; calls.push(["setSize", size]); },
    maximize: async () => { calls.push(["maximize"]); },
    isMaximized: async () => { if (windowErrors.isMaximized) throw windowErrors.isMaximized; return maximized; },
    innerSize: async () => inner,
  };
  const invokes = [];
  const invoke = async (command, args) => {
    invokes.push({ command, args });
    if (invokeImpl) return invokeImpl(command, args);
    return { saved: true, preferences: args.preferences };
  };
  const controller = createWindowStateController({
    invoke, isDesktop: () => desktop, host, themeTransition,
    windowApi: { currentWindow: () => windowLike, currentMonitor: async () => monitor, logicalSize: (width, height) => ({ width, height }) },
    storage: { setItem: (key, value) => stored.push([key, value]) }, browserPreferencesKey: "browser-prefs", root: { dataset },
  });
  return { controller, host, calls, invokes, dataset, stored };
}

const monitorOf = (logicalWidth, logicalHeight, scaleFactor = 2) => ({
  scaleFactor,
  workArea: { size: { toLogical: (factor) => ({ width: (logicalWidth * scaleFactor) / factor, height: (logicalHeight * scaleFactor) / factor }) } },
});

test("publishTheme publishes the base family, the exact palette and only instrument skins get a data-instrument", () => {
  const h = harness();
  h.controller.publishTheme("dark");
  assert.deepEqual([h.host.theme, h.dataset.theme, h.dataset.palette, h.dataset.instrument], ["dark", "dark", "dark", undefined]);
  for (const [theme, family] of [["cinema", "dark"], ["oled", "dark"], ["dos", "dark"]]) {
    h.controller.publishTheme(theme);
    assert.deepEqual([h.host.theme, h.dataset.theme, h.dataset.palette, h.dataset.instrument], [theme, family, theme, undefined], theme);
  }
  for (const skin of ["phosphor", "ion", "breach"]) {
    h.controller.publishTheme(skin);
    assert.deepEqual([h.dataset.theme, h.dataset.palette, h.dataset.instrument], [skin, skin, skin]);
  }
  h.controller.publishTheme("light");
  assert.equal("instrument" in h.dataset, false);
});

test("applyTheme normalizes the value, publishes it and saves it", async () => {
  const h = harness();
  h.controller.applyTheme("hacker");
  assert.deepEqual([h.host.theme, h.dataset.theme, h.dataset.instrument], ["phosphor", "phosphor", "phosphor"]);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(h.invokes[0].args.preferences.theme, "phosphor");
  h.controller.applyTheme("nonsense");
  assert.equal(h.host.theme, "light");
});

test("a saved retired high-contrast theme is applied and saved as light", async () => {
  const h = harness();
  h.controller.applyTheme("contrast");
  assert.deepEqual([h.host.theme, h.dataset.theme, h.dataset.palette], ["light", "light", "light"]);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(h.invokes[0].args.preferences.theme, "light");
});

test("applyTheme hands the visible switch to the transition and still saves the chosen theme at once", async () => {
  const pending = [];
  const h = harness({ themeTransition: (update) => pending.push(update) });
  h.controller.applyTheme("cinema");
  assert.equal(h.host.theme, "light");
  assert.equal(h.dataset.theme, undefined);
  assert.equal(pending.length, 1);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(h.invokes[0].args.preferences.theme, "cinema");
  pending[0]();
  assert.deepEqual([h.host.theme, h.dataset.theme, h.dataset.palette], ["cinema", "dark", "cinema"]);
});

test("persistPreferences merges at once and saves a deep copy, storing the returned payload", async () => {
  const h = harness();
  const promise = h.controller.persistPreferences({ lastPage: "run" }, "Saved it");
  assert.equal(h.host.appPreferences.lastPage, "run");
  assert.equal(await promise, true);
  assert.deepEqual(h.invokes, [{ command: "save_app_preferences", args: { preferences: { theme: "light", lastPage: "run", windowState: saved(), metadataLanguage: "en-US" } } }]);
  assert.notEqual(h.invokes[0].args.preferences, h.host.appPreferences);
  assert.equal(h.host.preferenceStorage.saved, true);
  assert.deepEqual(h.calls, [["notice", "success", "Saved it"]]);
});

test("a metadata language patch also updates its draft field", async () => {
  const h = harness();
  await h.controller.persistPreferences({ metadataLanguage: "fr-CA" });
  assert.equal(h.host.metadataLanguageDraft, "fr-CA");
  await h.controller.persistPreferences({ metadataLanguage: "" });
  assert.equal(h.host.metadataLanguageDraft, "fr-CA");
});

test("browser preview saves go to storage instead of the native command", async () => {
  const h = harness({ desktop: false });
  assert.equal(await h.controller.persistPreferences({ lastPage: "help" }), true);
  assert.deepEqual(h.invokes, []);
  assert.equal(h.stored[0][0], "browser-prefs");
  assert.equal(JSON.parse(h.stored[0][1]).lastPage, "help");
});

test("saves run one after another, and a failure reports the error and does not block the next one", async () => {
  const order = [];
  let fail = true;
  const h = harness({
    invokeImpl: async (command, args) => {
      await new Promise((resolve) => setTimeout(resolve, args.preferences.lastPage === "run" ? 25 : 0));
      order.push(args.preferences.lastPage);
      if (fail && args.preferences.lastPage === "run") throw new Error("disk full");
      return { ok: true };
    },
  });
  const results = await Promise.all([h.controller.persistPreferences({ lastPage: "run" }), h.controller.persistPreferences({ lastPage: "help" })]);
  assert.deepEqual(order, ["run", "help"]);
  assert.deepEqual(results, [false, true]);
  assert.deepEqual(h.calls, [["notice", "error", "Could not save application settings: Error: disk full"]]);
  fail = false;
});

test("restoring does nothing in browser preview", async () => {
  const h = harness({ desktop: false });
  await h.controller.restoreWindowState(saved());
  assert.deepEqual(h.calls, []);
});

test("restoring clamps the saved size to the monitor work area and never below 1120x720", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const clamped = harness({ monitor: monitorOf(1400, 850) });
  await clamped.controller.restoreWindowState(saved({ width: 1600, height: 1000 }));
  assert.deepEqual(clamped.calls, [["setSize", { width: 1360, height: 810 }]]);
  const small = harness({ monitor: monitorOf(1000, 600) });
  await small.controller.restoreWindowState(saved({ width: 800, height: 500 }));
  assert.deepEqual(small.calls, [["setSize", { width: 1120, height: 720 }]]);
  const roomy = harness({ monitor: monitorOf(3000, 2000) });
  await roomy.controller.restoreWindowState(saved({ width: 1700, height: 1100 }));
  assert.deepEqual(roomy.calls, [["setSize", { width: 1700, height: 1100 }]]);
});

test("without a monitor the window scale is used and the size is only floored and capped at 4096", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const h = harness({ monitor: null });
  await h.controller.restoreWindowState(saved({ width: 9000, height: 100 }));
  assert.deepEqual(h.calls, [["setSize", { width: 4096, height: 720 }]]);
  const maximized = harness();
  await maximized.controller.restoreWindowState(saved({ maximized: true }));
  assert.deepEqual(maximized.calls, [["setSize", { width: 1600, height: 1000 }], ["maximize"]]);
});

test("a failed restore is reported, and saving only becomes allowed 350 ms later", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const h = harness({ windowErrors: { setSize: new Error("no window") }, inner: { width: 2000, height: 1200 }, windowScale: 2 });
  await h.controller.restoreWindowState(saved());
  assert.deepEqual(h.calls, [["notice", "error", "Could not restore the previous window size: Error: no window"]]);
  h.controller.scheduleWindowStateSave();
  t.mock.timers.tick(2000);
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(h.invokes, []);
  t.mock.timers.tick(350);
  h.controller.scheduleWindowStateSave();
  t.mock.timers.tick(499);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(h.invokes.length, 0);
  t.mock.timers.tick(1);
  await new Promise((resolve) => setImmediate(resolve));
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(h.invokes.length, 1);
  assert.deepEqual(h.invokes[0].args.preferences.windowState, { width: 1000, height: 600, maximized: false });
});

async function ready(h, t) {
  await h.controller.restoreWindowState(h.host.appPreferences.windowState);
  t.mock.timers.tick(350);
}

test("resizes are debounced into one save, maximizing keeps the size, and unchanged state is not saved", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const h = harness({ inner: { width: 2400, height: 1500 }, windowScale: 2 });
  await ready(h, t);
  h.controller.scheduleWindowStateSave();
  t.mock.timers.tick(300);
  h.controller.scheduleWindowStateSave();
  t.mock.timers.tick(500);
  await new Promise((resolve) => setImmediate(resolve));
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(h.invokes.length, 1);
  assert.deepEqual(h.host.appPreferences.windowState, { width: 1200, height: 750, maximized: false });
  h.controller.scheduleWindowStateSave();
  t.mock.timers.tick(500);
  await new Promise((resolve) => setImmediate(resolve));
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(h.invokes.length, 1);
});

test("a maximized window records only the flag, and failures to read the window are reported", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const h = harness({ maximized: true, inner: { width: 5000, height: 3000 } });
  await ready(h, t);
  await h.controller.saveCurrentWindowState();
  assert.deepEqual(h.host.appPreferences.windowState, { width: 1600, height: 1000, maximized: true });
  assert.equal(h.invokes.length, 1);
  const broken = harness({ windowErrors: { isMaximized: new Error("gone") } });
  await ready(broken, t);
  await broken.controller.saveCurrentWindowState();
  assert.deepEqual(broken.calls.at(-1), ["notice", "error", "Could not save the window size: Error: gone"]);
});

test("saving is ignored in preview and before the window is ready", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const preview = harness({ desktop: false });
  await preview.controller.saveCurrentWindowState();
  preview.controller.scheduleWindowStateSave();
  const early = harness();
  await early.controller.saveCurrentWindowState();
  early.controller.scheduleWindowStateSave();
  t.mock.timers.tick(5000);
  assert.deepEqual([preview.invokes, early.invokes], [[], []]);
});
