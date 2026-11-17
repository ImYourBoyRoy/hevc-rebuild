// ./desktop/tests/app-boot.test.mjs
// Characterization tests for the app boot and listener module extracted from the route (boot slice):
// initialize() (preferences, legacy migration, window restore, queue, parallel loads), the browser
// preview preference load, the mount sequence (preview vs desktop), every native event listener, the
// drag-drop and resize handlers, cleanup, and the external-link click handler. The DOM, window,
// storage, Tauri listen/window API and the route's actions are injected fakes.
// Run: pnpm test:app-boot

import assert from "node:assert/strict";
import test from "node:test";
import { createAppBoot } from "../src/lib/app-boot.ts";

globalThis.Element ??= class Element {};
globalThis.HTMLAnchorElement ??= class HTMLAnchorElement extends globalThis.Element {};

const defaultTemplates = { movie: "{title}", series: "{series_title}", season: "Season {season:02}", episode: "S{season:02}E{episode:02}" };

const prefs = (over = {}) => ({
  theme: "dark", windowState: { width: 1600, height: 1000, maximized: false }, lastPage: "run", root: "/saved", toolPaths: { ffmpeg: "/opt/ffmpeg", ffprobe: null },
  recursive: false, minimumSizeMib: "9", stopOnError: true, youtubeResolution: "4k", youtubeQualityMode: "recommended", youtubeSourceMultiplier: 3, youtubeMode: "optimize",
  youtubeSdrCodec: "hevc", youtubePreset: "fast", youtubeShrink: "on", youtubeQualityCheck: "strict", youtubeOutputDir: "/yt", metadataLanguage: "fr-FR",
  organizerTemplates: { ...defaultTemplates }, metadataSources: { tmdb: true, wikidata: true, tvmaze: true, tvdb: false }, ...over,
});

function harness({ desktop = true, invokeImpl = {}, hostOver = {}, storage = {}, tauriFlags = {}, listenFail = false, registerFail = false, persistResult = true } = {}) {
  const log = [];
  const calls = [];
  const invokes = [];
  const store = new Map(Object.entries(storage));
  const listeners = {};
  const unlistened = [];
  const windowHandlers = {};
  const docListeners = [];
  const docRemoved = [];
  const buffer = { events: [], line: (p) => buffer.events.push(["line", p]), update: (p) => buffer.events.push(["update", p]), sample: (p) => buffer.events.push(["sample", p]), finish: (p) => buffer.events.push(["finish", p]) };
  const state = {
    preferenceStorage: null, appPreferences: prefs({ metadataSources: { tmdb: true, wikidata: true, tvmaze: true, tvdb: false } }), metadataLanguageDraft: "", minimumSizeMib: "", recursive: true, stopOnError: false,
    youtubeResolution: "", youtubeQualityMode: "", youtubeSourceMultiplier: 0, youtubeMode: "", youtubeSdrCodec: "", youtubePreset: "", youtubeShrink: "", youtubeQualityCheck: "", youtubeOutputDir: "x",
    root: "", page: "library", environmentError: "", booting: true, desktopShell: false, scanProgress: null, libraryDropActive: false, runLines: [], runResources: null,
    theme: "light", environment: { os: { package_manager: "apt", package_manager_available: true } }, workflowBusy: false, runId: "", runStarting: false, queueActiveRoot: null,
    ...hostOver,
  };
  const host = state;
  host.setNotice = (message, kind = "info") => log.push(["notice", kind, message]);
  const act = (name, impl = async () => {}) => async (...args) => { calls.push([name, ...args]); return impl(...args); };
  const actions = {
    restoreWindowState: act("restoreWindowState"), publishTheme: (value) => { calls.push(["publishTheme", value]); state.theme = value; },
    applyToolPaths: (value) => calls.push(["applyToolPaths", value]), persistPreferences: act("persistPreferences", async () => persistResult), loadMediaQueue: act("loadMediaQueue"),
    loadEnvironment: act("loadEnvironment"), loadSettings: act("loadSettings"), loadReports: act("loadReports"), refreshToolSetupLogs: act("refreshToolSetupLogs"),
    refreshTmdbCredentialStatus: act("refreshTmdbCredentialStatus"), refreshTvdbCredentialStatus: act("refreshTvdbCredentialStatus"), checkUbuntuPackageUpdates: act("checkUbuntuPackageUpdates"), checkSetupUpdates: act("checkSetupUpdates"),
    noteRunProgress: (payload) => calls.push(["noteRunProgress", payload]), finishRun: act("finishRun"), handleRepairLine: (p) => calls.push(["repairLine", p]),
    handleRepairLogWarning: (p) => calls.push(["repairWarning", p]), handleRepairFinished: act("repairFinished"), addQueueSources: act("addQueueSources"), openPage: (p) => calls.push(["openPage", p]),
    scheduleWindowStateSave: () => calls.push(["scheduleWindowStateSave"]), handleAgentControlRequest: act("agentRequest"), handleUtilityContextMenu: () => {}, closeContextMenu: () => calls.push(["closeContextMenu"]),
  };
  const invoke = async (command, args) => {
    invokes.push({ command, args });
    const impl = invokeImpl[command];
    if (impl === undefined) return null;
    return typeof impl === "function" ? impl(args) : impl;
  };
  const currentWindow = {
    onDragDropEvent: async (cb) => { windowHandlers.drag = cb; return () => unlistened.push("drag"); },
    onResized: async (cb) => { windowHandlers.resize = cb; return () => unlistened.push("resize"); },
  };
  const env = {
    document: { addEventListener: (type, fn) => docListeners.push([type, fn]), removeEventListener: (type, fn) => docRemoved.push([type, fn]) },
    window: { ...tauriFlags },
    localStorage: { getItem: (key) => store.get(key) ?? null, setItem: (key, value) => store.set(key, String(value)), removeItem: (key) => store.delete(key) },
    isTauri: () => desktop,
    listen: async (event, handler) => {
      if (registerFail && event === "scan-progress") throw new Error("listen failed");
      if (listenFail && event === "agent-control-request") throw new Error("no agent channel");
      listeners[event] = handler;
      return () => unlistened.push(event);
    },
    getCurrentWindow: () => currentWindow,
    openUrl: async (url) => { calls.push(["openUrl", url]); },
  };
  const boot = createAppBoot({
    invoke, host, actions, env, runEventBuffer: buffer,
    constants: { browserPreferencesKey: "browser-prefs", defaultFilenameExcludePatterns: ["*sample*"], defaultOrganizerTemplates: defaultTemplates },
  });
  return { boot, host, log, calls, invokes, store, listeners, unlistened, windowHandlers, docListeners, docRemoved, buffer, env };
}

const flush = async () => { for (let index = 0; index < 8; index += 1) await new Promise((resolve) => setImmediate(resolve)); };
const names = (h) => h.calls.map((call) => call[0]);
const stored = (over = {}) => ({ hasSavedSettings: true, storageWarning: null, preferences: prefs(over) });

test("initialize applies stored preferences to the route state and loads everything", async () => {
  const h = harness({ invokeImpl: { load_app_preferences: stored(), default_media_root: "/default" } });
  await h.boot.initialize();
  assert.deepEqual([h.host.metadataLanguageDraft, h.host.minimumSizeMib, h.host.recursive, h.host.stopOnError, h.host.root, h.host.page], ["fr-FR", "9", false, true, "/saved", "run"]);
  assert.deepEqual([h.host.youtubeResolution, h.host.youtubeQualityMode, h.host.youtubeSourceMultiplier, h.host.youtubeMode, h.host.youtubeSdrCodec, h.host.youtubePreset, h.host.youtubeShrink, h.host.youtubeQualityCheck, h.host.youtubeOutputDir], ["4k", "recommended", 3, "optimize", "hevc", "fast", "on", "strict", "/yt"]);
  assert.equal(h.host.preferenceStorage.hasSavedSettings, true);
  assert.deepEqual([h.host.booting, h.host.environmentError], [false, ""]);
  assert.deepEqual(h.calls.find((call) => call[0] === "applyToolPaths")[1], { ffmpeg: "/opt/ffmpeg", ffprobe: null });
  assert.deepEqual(names(h).filter((name) => ["restoreWindowState", "publishTheme", "loadMediaQueue"].includes(name)), ["restoreWindowState", "publishTheme", "loadMediaQueue"]);
  for (const name of ["loadEnvironment", "loadSettings", "loadReports", "refreshToolSetupLogs", "refreshTmdbCredentialStatus", "refreshTvdbCredentialStatus", "checkUbuntuPackageUpdates", "checkSetupUpdates"]) assert.equal(names(h).includes(name), true, name);
  assert.deepEqual(h.calls.find((call) => call[0] === "loadSettings"), ["loadSettings", "/saved"]);
  assert.deepEqual(h.calls.find((call) => call[0] === "checkUbuntuPackageUpdates"), ["checkUbuntuPackageUpdates", true]);
});

test("initialize falls back to the default media root and the library page", async () => {
  const h = harness({ invokeImpl: { load_app_preferences: stored({ root: null, lastPage: "bogus" }), default_media_root: "/default" } });
  await h.boot.initialize();
  assert.deepEqual([h.host.root, h.host.page], ["/default", "library"]);
  assert.deepEqual(h.calls.find((call) => call[0] === "persistPreferences")[1].root, "/default");
});

test("initialize migrates the old series template and saves it", async () => {
  const h = harness({ invokeImpl: { load_app_preferences: stored({ organizerTemplates: { ...defaultTemplates, series: "{series_title} [[({year_range})]]" } }) } });
  await h.boot.initialize();
  assert.equal(h.host.appPreferences.organizerTemplates.series, defaultTemplates.series);
  const persisted = h.calls.filter((call) => call[0] === "persistPreferences");
  assert.deepEqual(persisted.map((call) => Object.keys(call[1])), [["organizerTemplates", "root"]]);
});

test("initialize imports and then clears legacy browser keys when no settings were saved", async () => {
  const storage = { "hevc-rebuild-theme": "phosphor", "hevc-rebuild.root": "/legacy", "hevc-rebuild.ffmpeg": "/legacy/ffmpeg", "hevc-rebuild.recursive": "false", "hevc-rebuild.minimumSizeMib": "3" };
  const h = harness({ storage, invokeImpl: { load_app_preferences: { hasSavedSettings: false, storageWarning: null, preferences: prefs({ root: null, theme: "light", recursive: true, minimumSizeMib: "5", toolPaths: {} }) }, default_media_root: "/default" } });
  await h.boot.initialize();
  const persisted = h.calls.find((call) => call[0] === "persistPreferences")[1];
  assert.deepEqual([persisted.theme, persisted.root, persisted.recursive, persisted.minimumSizeMib], ["phosphor", "/legacy", false, "3"]);
  assert.equal(persisted.toolPaths.ffmpeg, "/legacy/ffmpeg");
  assert.deepEqual([...h.store.keys()], []);
  assert.deepEqual([h.host.root, h.host.recursive, h.host.minimumSizeMib], ["/legacy", false, "3"]);
});

test("initialize keeps the legacy keys when the migration could not be saved", async () => {
  const h = harness({ persistResult: false, storage: { "hevc-rebuild.root": "/legacy" }, invokeImpl: { load_app_preferences: { hasSavedSettings: false, storageWarning: null, preferences: prefs({ root: null }) }, default_media_root: "/d" } });
  await h.boot.initialize();
  assert.deepEqual([...h.store.keys()], ["hevc-rebuild.root"]);
  assert.equal(h.host.booting, false);
  const blocked = harness({ storage: { "hevc-rebuild.root": "/legacy" }, invokeImpl: { load_app_preferences: { hasSavedSettings: false, storageWarning: null, preferences: prefs({ root: null }) }, default_media_root: "/d" } });
  blocked.env.localStorage.removeItem = () => { throw new Error("storage disabled"); };
  await blocked.boot.initialize();
  assert.deepEqual([blocked.host.booting, blocked.host.environmentError], [false, ""]);
});

test("initialize prefers the queue's active root and otherwise saves the resolved root", async () => {
  const active = harness({ invokeImpl: { load_app_preferences: stored() }, hostOver: {} });
  active.host.queueActiveRoot = "/queue-root";
  await active.boot.initialize();
  assert.equal(active.calls.filter((call) => call[0] === "persistPreferences").length, 1, "an active root is persisted as the migrated root");
  assert.equal(active.calls.find((call) => call[0] === "persistPreferences")[1].root, "/queue-root");
  const settled = harness({ invokeImpl: { load_app_preferences: stored() } });
  await settled.boot.initialize();
  const saves = settled.calls.filter((call) => call[0] === "persistPreferences");
  assert.deepEqual(saves.map((call) => call[1]), [{ root: "/saved" }], "with no active root the resolved root is saved once");
});

test("initialize shows the storage warning and skips the update check on a non-APT host", async () => {
  const h = harness({ hostOver: { environment: { os: { package_manager: "brew", package_manager_available: true } } }, invokeImpl: { load_app_preferences: { ...stored(), storageWarning: "preferences were reset" } } });
  await h.boot.initialize();
  assert.deepEqual(h.log, [["notice", "error", "preferences were reset"]]);
  assert.equal(names(h).includes("checkUbuntuPackageUpdates"), false);
  const unavailable = harness({ hostOver: { environment: { os: { package_manager: "apt", package_manager_available: false } } }, invokeImpl: { load_app_preferences: stored() } });
  await unavailable.boot.initialize();
  assert.equal(names(unavailable).includes("checkUbuntuPackageUpdates"), false);
  assert.equal(names(h).includes("checkSetupUpdates"), true, "the tool and helper update check runs on every desktop host, not only APT");
});

test("initialize reports a backend failure, records the error and always finishes booting", async () => {
  const h = harness({ invokeImpl: { load_app_preferences: () => { throw new Error("no backend"); } } });
  await h.boot.initialize();
  assert.deepEqual([h.host.environmentError, h.host.booting], ["Error: no backend", false]);
  assert.deepEqual(h.log, [["notice", "error", "The desktop backend did not start: Error: no backend"]]);
});

test("loadBrowserPreferences applies saved browser preferences and rewrites a legacy theme", () => {
  const saved = JSON.stringify({ theme: "ion", root: "/web", recursive: false, minimumSizeMib: "7", metadataLanguage: "de-DE" });
  const h = harness({ storage: { "browser-prefs": saved } });
  h.boot.loadBrowserPreferences();
  assert.deepEqual([h.host.appPreferences.theme, h.host.root, h.host.recursive, h.host.minimumSizeMib, h.host.metadataLanguageDraft], ["ion", "/web", false, "7", "de-DE"]);
  assert.deepEqual(h.calls.find((call) => call[0] === "publishTheme"), ["publishTheme", "ion"]);
  const legacy = harness({ storage: { "browser-prefs": JSON.stringify({ theme: "hacker" }) } });
  legacy.boot.loadBrowserPreferences();
  assert.equal(JSON.parse(legacy.store.get("browser-prefs")).theme, "phosphor");
  const empty = harness({ hostOver: { appPreferences: prefs({ root: null }) } });
  empty.boot.loadBrowserPreferences();
  assert.equal(empty.host.root, "");
  assert.equal(empty.store.has("browser-prefs"), false);
});

test("startApp in the browser preview loads browser preferences and explains the missing desktop", async () => {
  const h = harness({ desktop: false });
  const cleanup = h.boot.startApp();
  assert.deepEqual([h.host.desktopShell, h.host.booting], [false, false]);
  assert.match(h.host.environmentError, /^Open HEVC Rebuild Studio from the desktop app/);
  assert.deepEqual(h.docListeners.map((entry) => entry[0]), ["contextmenu"]);
  assert.equal(h.invokes.length, 0);
  await flush();
  assert.deepEqual(names(h).filter((name) => name.startsWith("refresh")), ["refreshTmdbCredentialStatus", "refreshTvdbCredentialStatus"]);
  cleanup();
  assert.deepEqual(h.docRemoved.map((entry) => entry[0]), ["contextmenu"]);
});

test("startApp on desktop reports readiness, hooks agent control and registers every listener", async () => {
  const h = harness({ invokeImpl: { load_app_preferences: stored() } });
  const cleanup = h.boot.startApp();
  assert.equal(h.host.desktopShell, true);
  assert.deepEqual(h.docListeners.map((entry) => entry[0]), ["contextmenu", "click"]);
  await flush();
  assert.equal(h.invokes[0].command, "frontend_ready");
  assert.deepEqual(Object.keys(h.listeners).sort(), ["agent-control-request", "repair-finished", "repair-line", "repair-log-warning", "run-finished", "run-line", "run-progress", "run-resources", "scan-progress"]);
  assert.equal(typeof h.windowHandlers.drag, "function");
  assert.equal(h.host.booting, false, "initialize ran after registration");
  h.env.window.__hevcAgentControl({ id: "r1", action: "state" });
  assert.deepEqual(h.calls.find((call) => call[0] === "agentRequest"), ["agentRequest", { id: "r1", action: "state" }]);
  cleanup();
  assert.equal(h.unlistened.length, 11);
  assert.deepEqual(h.docRemoved.map((entry) => entry[0]).sort(), ["click", "contextmenu"]);
  assert.equal(names(h).includes("closeContextMenu"), true);
});

test("startApp detects desktop through the Tauri globals and tolerates a missing agent channel", async () => {
  const h = harness({ desktop: false, tauriFlags: { __TAURI_INTERNALS__: {} }, listenFail: true, invokeImpl: { load_app_preferences: stored() } });
  const warnings = [];
  const original = console.warn;
  console.warn = (...args) => warnings.push(args[0]);
  try {
    h.boot.startApp();
    await flush();
  } finally { console.warn = original; }
  assert.equal(h.host.desktopShell, true);
  assert.equal(warnings.some((message) => String(message).includes("agent-control listener could not be registered")), true);
  assert.equal("agent-control-request" in h.listeners, false);
});

test("scan, run and repair events update the route state or reach the controllers", async () => {
  const h = harness({ invokeImpl: { load_app_preferences: stored() } });
  h.boot.startApp();
  await flush();
  h.listeners["scan-progress"]({ payload: { done: 1 } });
  assert.deepEqual(h.host.scanProgress, { done: 1 });
  h.host.runId = "run-1";
  h.listeners["run-line"]({ payload: { runId: "run-1", line: "a" } });
  h.listeners["run-line"]({ payload: { runId: "other", line: "dropped" } });
  assert.deepEqual(h.host.runLines, ["a"]);
  for (let index = 0; index < 250; index += 1) h.listeners["run-line"]({ payload: { runId: "run-1", line: `l${index}` } });
  assert.equal(h.host.runLines.length, 240);
  h.listeners["run-progress"]({ payload: { runId: "run-1", percent: 5 } });
  assert.deepEqual(h.calls.find((call) => call[0] === "noteRunProgress")[1], { runId: "run-1", percent: 5 });
  h.listeners["run-resources"]({ payload: { runId: "run-1", snapshot: { cpu: 1 } } });
  assert.deepEqual(h.host.runResources, { cpu: 1 });
  h.listeners["repair-line"]({ payload: { runId: "j", line: "x" } });
  h.listeners["repair-log-warning"]({ payload: { runId: "j", line: "w" } });
  await h.listeners["repair-finished"]({ payload: { jobId: "j" } });
  assert.deepEqual(names(h).filter((name) => name.startsWith("repair")), ["repairLine", "repairWarning", "repairFinished"]);
});

test("events for a run that has not started yet are buffered, and finishing the active run is handled", async () => {
  const h = harness({ invokeImpl: { load_app_preferences: stored() } });
  h.boot.startApp();
  await flush();
  h.host.runId = "";
  h.host.runStarting = true;
  h.listeners["run-line"]({ payload: { runId: "new", line: "early" } });
  h.listeners["run-progress"]({ payload: { runId: "new" } });
  h.listeners["run-resources"]({ payload: { runId: "new", snapshot: {} } });
  await h.listeners["run-finished"]({ payload: { runId: "new" } });
  assert.deepEqual(h.buffer.events.map((entry) => entry[0]), ["line", "update", "sample", "finish"]);
  assert.equal(names(h).includes("finishRun"), false);
  h.host.runId = "new";
  h.host.runStarting = false;
  await h.listeners["run-finished"]({ payload: { runId: "new", status: "ok" } });
  assert.deepEqual(h.calls.find((call) => call[0] === "finishRun"), ["finishRun", { runId: "new", status: "ok" }]);
  h.host.runId = "other";
  h.host.runStarting = false;
  const before = h.buffer.events.length;
  await h.listeners["run-finished"]({ payload: { runId: "stale" } });
  assert.equal(h.buffer.events.length, before);
});

test("drag and drop highlights the library page, adds dropped paths and refuses while busy", async () => {
  const h = harness({ invokeImpl: { load_app_preferences: stored({ lastPage: "library" }) } });
  h.boot.startApp();
  await flush();
  const drag = h.windowHandlers.drag;
  drag({ payload: { type: "enter" } });
  assert.equal(h.host.libraryDropActive, true);
  h.host.page = "run";
  drag({ payload: { type: "over" } });
  assert.equal(h.host.libraryDropActive, false);
  h.host.page = "library";
  h.host.workflowBusy = true;
  drag({ payload: { type: "over" } });
  assert.equal(h.host.libraryDropActive, false);
  drag({ payload: { type: "leave" } });
  assert.equal(h.host.libraryDropActive, false);
  drag({ payload: { type: "drop", paths: ["/a/movie.mkv"] } });
  assert.deepEqual(h.log.at(-1), ["notice", "error", "Finish the current scan or run before adding dropped files."]);
  assert.equal(names(h).includes("addQueueSources"), false);
  h.host.workflowBusy = false;
  drag({ payload: { type: "drop", paths: [] } });
  drag({ payload: { type: "drop", paths: ["/a/movie.mkv", "/a/b"] } });
  assert.equal(h.calls.some((call) => call[0] === "openPage" && call[1] === "library"), true);
  assert.deepEqual(h.log.at(-1), ["notice", "info", "Adding 2 dropped paths to the queue."]);
  assert.equal(h.calls.find((call) => call[0] === "addQueueSources")[1].length, 2);
  drag({ payload: { type: "drop", paths: ["/only.mkv"] } });
  assert.deepEqual(h.log.at(-1), ["notice", "info", "Adding 1 dropped path to the queue."]);
});

test("window resize schedules a window-state save", async () => {
  const h = harness({ invokeImpl: { load_app_preferences: stored() } });
  h.boot.startApp();
  await flush();
  h.windowHandlers.resize();
  assert.equal(names(h).includes("scheduleWindowStateSave"), true);
});

test("a failure while registering listeners reports the error and finishes booting", async () => {
  const h = harness({ registerFail: true });
  h.boot.startApp();
  await flush();
  assert.deepEqual([h.host.environmentError, h.host.booting], ["Error: listen failed", false]);
  assert.deepEqual(h.log, [["notice", "error", "The desktop backend did not start: Error: listen failed"]]);
});

test("disposing before registration finishes unlistens immediately", async () => {
  const h = harness({ invokeImpl: { load_app_preferences: stored() } });
  const cleanup = h.boot.startApp();
  cleanup();
  await flush();
  assert.equal(h.unlistened.length >= 10, true);
});

const anchorEvent = (over = {}) => {
  const anchor = Object.assign(new globalThis.HTMLAnchorElement(), { protocol: "https:", href: "https://example.org/x", hostname: "example.org" });
  const target = Object.assign(new globalThis.Element(), { closest: () => anchor });
  let prevented = false;
  return { event: { defaultPrevented: false, button: 0, target, preventDefault: () => { prevented = true; }, ...over }, wasPrevented: () => prevented, anchor };
};

test("openExternalLink sends https anchors to the system browser", async () => {
  const h = harness();
  const { event, wasPrevented } = anchorEvent();
  h.boot.openExternalLink(event);
  await flush();
  assert.equal(wasPrevented(), true);
  assert.deepEqual(h.calls.find((call) => call[0] === "openUrl"), ["openUrl", "https://example.org/x"]);
});

test("openExternalLink ignores handled clicks, other buttons, non-https links and non-anchors", () => {
  const h = harness();
  for (const over of [{ defaultPrevented: true }, { button: 1 }]) {
    const { event, wasPrevented } = anchorEvent(over);
    h.boot.openExternalLink(event);
    assert.equal(wasPrevented(), false);
  }
  const http = anchorEvent();
  http.anchor.protocol = "http:";
  h.boot.openExternalLink(http.event);
  const none = anchorEvent({ target: Object.assign(new globalThis.Element(), { closest: () => null }) });
  h.boot.openExternalLink(none.event);
  const plain = anchorEvent({ target: {} });
  h.boot.openExternalLink(plain.event);
  assert.equal(h.calls.some((call) => call[0] === "openUrl"), false);
});

test("openExternalLink reports a failure to open the browser", async () => {
  const h = harness();
  h.env.openUrl = async () => { throw new Error("no browser"); };
  const failing = createAppBoot({ invoke: async () => null, host: h.host, actions: {}, env: h.env, runEventBuffer: h.buffer, constants: { browserPreferencesKey: "k", defaultFilenameExcludePatterns: [], defaultOrganizerTemplates: defaultTemplates } });
  const { event } = anchorEvent();
  failing.openExternalLink(event);
  await flush();
  assert.deepEqual(h.log.at(-1), ["notice", "error", "Could not open example.org in your browser: Error: no browser"]);
});
