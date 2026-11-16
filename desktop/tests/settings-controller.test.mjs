// ./desktop/tests/settings-controller.test.mjs
// Characterization tests for the settings controller extracted from the route (settings slice):
// effective-settings load/reset/save, the work-directory preview, the metadata language draft, the
// metadata provider toggles, the title-cleanup blacklist and the filename include/exclude filters.
// The native bridge and preference persistence are injected fakes; the route's state is a plain host.
// Run: pnpm test:settings-controller

import assert from "node:assert/strict";
import test from "node:test";
import { createSettingsController } from "../src/lib/settings-controller.ts";

const baseSettings = () => ({ video: { crf: 20 }, metadata: { comment: "c", text_blacklist: [] }, paths: { work_root: "auto" } });

function harness({ desktop = true, invokeImpl = {}, hostOver = {}, persistResult = true } = {}) {
  const log = [];
  const invokes = [];
  const persisted = [];
  const state = {
    settings: null, settingsBaseline: "", librarySettingsBaseline: "", settingsRoot: "", settingsSources: [], settingsPath: "", workDirectory: null,
    metadataLanguageDraft: "en-US", newMetadataBlacklistPattern: "", metadataBlacklistError: "", newFilenameIncludePattern: "", newFilenameExcludePattern: "", filenameFilterError: "",
    appPreferences: { metadataLanguage: "en-US", settings: null, metadataSources: { tmdb: true, wikidata: true, tvmaze: true, tvdb: false }, filenameIncludePatterns: [], filenameExcludePatterns: [] },
    root: "/lib", activeRoot: "", workflowBusy: false,
    ...hostOver,
  };
  const host = state;
  Object.defineProperty(host, "settingsChanged", { get: () => Boolean(state.settings && state.settingsBaseline && JSON.stringify(state.settings) !== state.settingsBaseline) });
  host.setNotice = (message, kind = "info") => log.push([kind, message]);
  const invoke = async (command, args) => {
    invokes.push({ command, args });
    const impl = invokeImpl[command];
    if (impl === undefined) throw new Error(`unexpected ${command}`);
    return typeof impl === "function" ? impl(args) : impl;
  };
  const persistPreferences = async (patch, message) => { persisted.push([patch, message]); return typeof persistResult === "function" ? persistResult(patch) : persistResult; };
  const controller = createSettingsController({ invoke, isDesktop: () => desktop, host, persistPreferences });
  return { controller, host, log, invokes, persisted };
}

const load = (over = {}) => ({ settings: baseSettings(), sources: ["defaults", "user"], user_config: "/home/u/.config/hevc.toml", work_directory: { path: "/lib/.work", reason: "adjacent" }, ...over });

test("loadSettings stores the effective settings, baselines, sources and work directory", async () => {
  const payload = load();
  const h = harness({ invokeImpl: { load_settings: payload } });
  await h.controller.loadSettings("  /lib/movies  ");
  assert.deepEqual(h.invokes[0], { command: "load_settings", args: { root: "/lib/movies" } });
  assert.deepEqual(h.host.settings, payload.settings);
  assert.equal(h.host.settingsBaseline, JSON.stringify(payload.settings));
  assert.equal(h.host.librarySettingsBaseline, JSON.stringify(payload.settings));
  assert.deepEqual([h.host.settingsRoot, h.host.settingsSources, h.host.settingsPath, h.host.workDirectory], ["/lib/movies", ["defaults", "user"], "/home/u/.config/hevc.toml", payload.work_directory]);
});

test("loadSettings ignores a blank folder and reports a failure", async () => {
  const blank = harness();
  await blank.controller.loadSettings("   ");
  assert.equal(blank.invokes.length, 0);
  const failed = harness({ invokeImpl: { load_settings: () => { throw new Error("bad toml"); } } });
  await failed.controller.loadSettings("/lib");
  assert.deepEqual(failed.log, [["error", "Could not load the effective settings: Error: bad toml"]]);
  assert.equal(failed.host.settings, null);
});

test("previewWorkDirectory resolves the source folder with the typed work root", async () => {
  const preview = { path: "/work/x", reason: "explicit" };
  const h = harness({ invokeImpl: { resolve_work_directory_command: preview } });
  await h.controller.previewWorkDirectory("  /work/x  ");
  assert.deepEqual(h.invokes[0], { command: "resolve_work_directory_command", args: { root: "/lib", workRoot: "/work/x" } });
  assert.deepEqual(h.host.workDirectory, preview);
  await h.controller.previewWorkDirectory("   ");
  assert.equal(h.invokes[1].args.workRoot, "auto");
  h.host.root = "  ";
  h.host.activeRoot = "/active";
  await h.controller.previewWorkDirectory("auto");
  assert.equal(h.invokes[2].args.root, "/active");
});

test("previewWorkDirectory does nothing without a source or off desktop and keeps the last preview on error", async () => {
  const none = harness({ hostOver: { root: " ", activeRoot: "" } });
  await none.controller.previewWorkDirectory("auto");
  const off = harness({ desktop: false });
  await off.controller.previewWorkDirectory("auto");
  assert.deepEqual([none.invokes.length, off.invokes.length], [0, 0]);
  const kept = { path: "/old", reason: "adjacent" };
  const failed = harness({ invokeImpl: { resolve_work_directory_command: () => { throw new Error("invalid"); } }, hostOver: { workDirectory: kept } });
  await failed.controller.previewWorkDirectory("bad");
  assert.equal(failed.host.workDirectory, kept);
  assert.deepEqual(failed.log, []);
});

test("resetSettings restores the effective settings when no profile is saved", async () => {
  const baseline = baseSettings();
  const h = harness({ hostOver: { settings: { ...baseSettings(), video: { crf: 30 } }, settingsBaseline: JSON.stringify(baseline) } });
  await h.controller.resetSettings();
  assert.deepEqual(h.host.settings, baseline);
  assert.deepEqual(h.log, [["success", "Restored the effective settings for this library."]]);
  assert.equal(h.persisted.length, 0);
});

test("resetSettings removes a saved profile and reloads the folder's effective settings", async () => {
  const h = harness({ invokeImpl: { load_settings: load() }, hostOver: { settingsBaseline: "{}", settingsRoot: "/saved", root: " /current ", appPreferences: { settings: { saved: true } } } });
  await h.controller.resetSettings();
  assert.deepEqual(h.persisted, [[{ settings: null }, "Removed the saved application profile."]]);
  assert.deepEqual(h.invokes[0], { command: "load_settings", args: { root: "/current" } });
  const fallback = harness({ invokeImpl: { load_settings: load() }, hostOver: { settingsBaseline: "{}", settingsRoot: "/saved", root: " ", appPreferences: { settings: { saved: true } } } });
  await fallback.controller.resetSettings();
  assert.equal(fallback.invokes[0].args.root, "/saved");
  const failedPersist = harness({ persistResult: false, hostOver: { settingsBaseline: "{}", settingsRoot: "/saved", appPreferences: { settings: { saved: true } } } });
  await failedPersist.controller.resetSettings();
  assert.equal(failedPersist.invokes.length, 0);
  const noFolder = harness({ hostOver: { settingsBaseline: "{}", settingsRoot: "", root: " ", appPreferences: { settings: { saved: true } } } });
  await noFolder.controller.resetSettings();
  assert.equal(noFolder.invokes.length, 0);
});

test("resetSettings is blocked without a baseline or while busy, and reports a restore failure", async () => {
  for (const hostOver of [{ settingsBaseline: "" }, { settingsBaseline: "{}", workflowBusy: true }]) {
    const h = harness({ hostOver });
    await h.controller.resetSettings();
    assert.deepEqual([h.persisted.length, h.log.length], [0, 0]);
  }
  const broken = harness({ hostOver: { settingsBaseline: "{not json" } });
  await broken.controller.resetSettings();
  assert.equal(broken.log[0][0], "error");
  assert.match(broken.log[0][1], /^Could not restore the effective settings: /);
});

test("saveSettings persists a deep copy, updates the baselines and records the saved profile source", async () => {
  const settings = { ...baseSettings(), video: { crf: 18 } };
  const h = harness({ hostOver: { settings, settingsBaseline: JSON.stringify(baseSettings()), settingsSources: ["defaults"] } });
  await h.controller.saveSettings();
  assert.equal(h.persisted.length, 1);
  assert.deepEqual(h.persisted[0][0].settings, settings);
  assert.notEqual(h.persisted[0][0].settings, settings);
  assert.equal(h.persisted[0][1], "Saved these rebuild settings as this app's defaults.");
  assert.equal(h.host.settingsBaseline, JSON.stringify(settings));
  assert.equal(h.host.librarySettingsBaseline, JSON.stringify(settings));
  assert.deepEqual(h.host.settingsSources, ["defaults", "Saved application profile"]);
  const again = harness({ hostOver: { settings, settingsBaseline: "{}", librarySettingsBaseline: "kept", settingsSources: ["Saved application profile"] } });
  await again.controller.saveSettings();
  assert.deepEqual([again.host.librarySettingsBaseline, again.host.settingsSources], ["kept", ["Saved application profile"]]);
});

test("saveSettings does nothing when unchanged, busy or the save fails", async () => {
  const settings = baseSettings();
  const unchanged = harness({ hostOver: { settings, settingsBaseline: JSON.stringify(settings) } });
  await unchanged.controller.saveSettings();
  const busy = harness({ hostOver: { settings: { ...settings, video: { crf: 1 } }, settingsBaseline: JSON.stringify(settings), workflowBusy: true } });
  await busy.controller.saveSettings();
  const none = harness();
  await none.controller.saveSettings();
  assert.deepEqual([unchanged.persisted.length, busy.persisted.length, none.persisted.length], [0, 0, 0]);
  const failed = harness({ persistResult: false, hostOver: { settings: { ...settings, video: { crf: 1 } }, settingsBaseline: JSON.stringify(settings) } });
  await failed.controller.saveSettings();
  assert.equal(failed.host.settingsBaseline, JSON.stringify(settings));
});

test("saveMetadataLanguageDraft normalizes a valid code and saves only a change", () => {
  const h = harness();
  h.host.metadataLanguageDraft = " fr-fr ";
  h.controller.saveMetadataLanguageDraft();
  assert.deepEqual([h.host.metadataLanguageDraft, h.persisted], ["fr-FR", [[{ metadataLanguage: "fr-FR" }, undefined]]]);
  h.host.metadataLanguageDraft = "EN";
  h.controller.saveMetadataLanguageDraft();
  assert.deepEqual([h.host.metadataLanguageDraft, h.persisted.at(-1)[0]], ["en", { metadataLanguage: "en" }]);
  const same = harness();
  same.host.metadataLanguageDraft = "en-us";
  same.controller.saveMetadataLanguageDraft();
  assert.deepEqual([same.host.metadataLanguageDraft, same.persisted.length], ["en-US", 0]);
});

test("saveMetadataLanguageDraft rejects an invalid code and restores the saved language", () => {
  for (const draft of ["", "x", "english", "en_US", "en-USA", "e1-US"]) {
    const h = harness();
    h.host.metadataLanguageDraft = draft;
    h.controller.saveMetadataLanguageDraft();
    assert.equal(h.host.metadataLanguageDraft, "en-US", draft);
    assert.deepEqual(h.log, [["error", "Use a two-letter language code with an optional two-letter region, such as en-US, fr-FR, or ja-JP."]]);
    assert.equal(h.persisted.length, 0);
  }
});

test("updateMetadataSources merges the patch into the saved sources", () => {
  const h = harness();
  h.controller.updateMetadataSources({ tvdb: true });
  assert.deepEqual(h.persisted, [[{ metadataSources: { tmdb: true, wikidata: true, tvmaze: true, tvdb: true } }, undefined]]);
});

test("addMetadataBlacklistPattern validates, then appends and clears the draft", () => {
  const h = harness({ hostOver: { settings: baseSettings() } });
  const attempt = (value) => { h.host.newMetadataBlacklistPattern = value; h.controller.addMetadataBlacklistPattern(); return h.host.metadataBlacklistError; };
  assert.equal(attempt("   "), "Enter a pattern first.");
  assert.equal(attempt("x".repeat(129)), "Patterns can be up to 128 UTF-8 bytes.");
  assert.equal(attempt("é".repeat(65)), "Patterns can be up to 128 UTF-8 bytes.");
  assert.equal(attempt("***"), "Add at least one letter or number; a wildcard-only pattern would remove all text.");
  assert.equal(attempt("bad\u0007name"), "Control characters are not allowed in patterns.");
  assert.equal(attempt(" Rip*Group "), "");
  assert.deepEqual([h.host.settings.metadata.text_blacklist, h.host.newMetadataBlacklistPattern], [["Rip*Group"], ""]);
  assert.equal(attempt("rip*GROUP"), "That pattern is already in the list.");
  h.host.settings.metadata.text_blacklist = Array.from({ length: 100 }, (_, index) => `p${index}`);
  assert.equal(attempt("another"), "The list is limited to 100 patterns.");
  const none = harness();
  none.host.newMetadataBlacklistPattern = "x";
  none.controller.addMetadataBlacklistPattern();
  assert.equal(none.host.metadataBlacklistError, "");
});

test("removeMetadataBlacklistPattern and clearMetadataBlacklist edit the list and clear the error", () => {
  const settings = baseSettings();
  settings.metadata.text_blacklist = ["a", "b", "c"];
  const h = harness({ hostOver: { settings, metadataBlacklistError: "old" } });
  h.controller.removeMetadataBlacklistPattern(1);
  assert.deepEqual([h.host.settings.metadata.text_blacklist, h.host.metadataBlacklistError], [["a", "c"], ""]);
  h.host.metadataBlacklistError = "again";
  h.controller.clearMetadataBlacklist();
  assert.deepEqual([h.host.settings.metadata.text_blacklist, h.host.metadataBlacklistError], [[], ""]);
  const none = harness({ hostOver: { metadataBlacklistError: "keep" } });
  none.controller.removeMetadataBlacklistPattern(0);
  none.controller.clearMetadataBlacklist();
  assert.equal(none.host.metadataBlacklistError, "keep");
});

test("addFilenamePattern validates per list and persists the matching preference key", () => {
  const h = harness();
  const attempt = (kind, value) => {
    if (kind === "include") h.host.newFilenameIncludePattern = value; else h.host.newFilenameExcludePattern = value;
    h.controller.addFilenamePattern(kind);
    return h.host.filenameFilterError;
  };
  assert.equal(attempt("include", " "), "Enter a filename pattern first.");
  assert.equal(attempt("include", "x".repeat(129)), "Filename patterns can be up to 128 UTF-8 bytes.");
  assert.equal(attempt("include", "a/b"), "Use a filename pattern without folder separators or control characters.");
  assert.equal(attempt("exclude", "a\\b"), "Use a filename pattern without folder separators or control characters.");
  assert.equal(attempt("exclude", "*?*"), "Add at least one filename character; wildcard-only filters can hide the entire library.");
  assert.equal(h.persisted.length, 0);
  assert.equal(attempt("include", " *.mkv "), "");
  assert.deepEqual(h.persisted.at(-1), [{ filenameIncludePatterns: ["*.mkv"] }, undefined]);
  assert.equal(h.host.newFilenameIncludePattern, "");
  assert.equal(attempt("exclude", "*sample*"), "");
  assert.deepEqual(h.persisted.at(-1), [{ filenameExcludePatterns: ["*sample*"] }, undefined]);
  assert.equal(h.host.newFilenameExcludePattern, "");
});

test("addFilenamePattern rejects duplicates (case-insensitive) and a full list", () => {
  const h = harness({ hostOver: { appPreferences: { filenameIncludePatterns: ["*.MKV"], filenameExcludePatterns: Array.from({ length: 100 }, (_, index) => `p${index}`) } } });
  h.host.newFilenameIncludePattern = "*.mkv";
  h.controller.addFilenamePattern("include");
  assert.equal(h.host.filenameFilterError, "That pattern is already in this list.");
  h.host.newFilenameExcludePattern = "new";
  h.controller.addFilenamePattern("exclude");
  assert.equal(h.host.filenameFilterError, "Each list is limited to 100 patterns.");
  assert.equal(h.persisted.length, 0);
});

test("removeFilenamePattern and clearFilenamePatterns persist the updated list and clear the error", () => {
  const h = harness({ hostOver: { filenameFilterError: "old", appPreferences: { filenameIncludePatterns: ["a", "b"], filenameExcludePatterns: ["x"] } } });
  h.controller.removeFilenamePattern("include", 0);
  assert.deepEqual([h.persisted.at(-1), h.host.filenameFilterError], [[{ filenameIncludePatterns: ["b"] }, undefined], ""]);
  h.host.filenameFilterError = "again";
  h.controller.clearFilenamePatterns("exclude");
  assert.deepEqual([h.persisted.at(-1), h.host.filenameFilterError], [[{ filenameExcludePatterns: [] }, undefined], ""]);
});
