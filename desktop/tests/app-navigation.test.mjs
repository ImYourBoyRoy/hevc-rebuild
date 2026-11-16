// ./desktop/tests/app-navigation.test.mjs
// Characterization tests for the page navigation helpers extracted from the route (helper slice): which
// sidebar item is active, switching pages (and the YouTube mode that follows them), jumping to the run
// progress details and opening Settings at a metadata provider credential field. DOM access is injected.
// Run: pnpm test:app-navigation

import assert from "node:assert/strict";
import test from "node:test";
import { createAppNavigation } from "../src/lib/app-navigation.ts";

function harness({ hostOver = {}, elements = {} } = {}) {
  const calls = [];
  const host = { page: "library", mode: "check", appPreferences: { lastPage: "library" }, runMode: "check", ...hostOver };
  const actions = { persistPreferences: async (patch) => { calls.push(["persistPreferences", patch]); return true; }, tick: async () => { calls.push(["tick"]); } };
  const env = {
    getElementById: (id) => { calls.push(["getElementById", id]); return elements[id] ?? null; },
    requestAnimationFrame: (callback) => { calls.push(["raf"]); callback(0); },
  };
  return { nav: createAppNavigation({ host, actions, env }), host, calls };
}

test("navItemActive keeps Library lit for the library, metadata and home pages", () => {
  for (const page of ["library", "metadata", "home"]) assert.equal(harness({ hostOver: { page } }).nav.navItemActive("library"), true, page);
  assert.equal(harness({ hostOver: { page: "run" } }).nav.navItemActive("library"), false);
  assert.equal(harness({ hostOver: { page: "run" } }).nav.navItemActive("run"), true);
  assert.equal(harness({ hostOver: { page: "library" } }).nav.navItemActive("metadata"), false);
  assert.equal(harness({ hostOver: { page: "settings" } }).nav.navItemActive("settings"), true);
});

test("openPage switches the page, maps home to library and saves a changed last page", () => {
  const h = harness();
  h.nav.openPage("run");
  assert.deepEqual([h.host.page, h.host.mode], ["run", "check"]);
  assert.deepEqual(h.calls, [["persistPreferences", { lastPage: "run" }]]);
  const same = harness({ hostOver: { appPreferences: { lastPage: "library" } } });
  same.nav.openPage("home");
  assert.deepEqual([same.host.page, same.calls.length], ["library", 0]);
});

test("openPage couples the YouTube page with the youtube run mode and resets it on leaving", () => {
  const into = harness({ hostOver: { mode: "rebuild" } });
  into.nav.openPage("youtube");
  assert.equal(into.host.mode, "youtube");
  const out = harness({ hostOver: { page: "youtube", mode: "youtube" } });
  out.nav.openPage("library");
  assert.equal(out.host.mode, "check");
  const keep = harness({ hostOver: { mode: "rebuild" } });
  keep.nav.openPage("settings");
  assert.equal(keep.host.mode, "rebuild");
});

test("openProgressDetails opens the right page and scrolls to the activity log after two frames", () => {
  const scrolls = [];
  const element = { scrollIntoView: (options) => scrolls.push(options) };
  const h = harness({ elements: { "run-activity": element } });
  h.nav.openProgressDetails();
  assert.equal(h.host.page, "run");
  assert.deepEqual(h.calls.filter((call) => call[0] !== "persistPreferences").map((call) => call[0]), ["raf", "raf", "getElementById"]);
  assert.deepEqual(scrolls, [{ behavior: "smooth", block: "start" }]);
  const youtube = harness({ hostOver: { runMode: "youtube" } });
  youtube.nav.openProgressDetails();
  assert.deepEqual([youtube.host.page, youtube.host.mode], ["youtube", "youtube"]);
});

test("openMetadataProviderSettings opens Settings, scrolls to the provider card and focuses its field", async () => {
  const log = [];
  const elements = {
    "tmdb-credential-settings": { scrollIntoView: (options) => log.push(["scroll", "tmdb", options]) },
    "tmdb-api-token": { focus: (options) => log.push(["focus", "tmdb", options]) },
    "tvdb-credential-settings": { scrollIntoView: (options) => log.push(["scroll", "tvdb", options]) },
    "tvdb-project-key": { focus: (options) => log.push(["focus", "tvdb", options]) },
  };
  const tmdb = harness({ elements });
  await tmdb.nav.openMetadataProviderSettings("tmdb");
  assert.equal(tmdb.host.page, "settings");
  assert.deepEqual(log, [["scroll", "tmdb", { behavior: "smooth", block: "center" }], ["focus", "tmdb", { preventScroll: true }]]);
  log.length = 0;
  await harness({ elements }).nav.openMetadataProviderSettings("tvdb");
  assert.deepEqual(log.map((entry) => entry.slice(0, 2)), [["scroll", "tvdb"], ["focus", "tvdb"]]);
  log.length = 0;
  const none = harness({ elements });
  await none.nav.openMetadataProviderSettings();
  assert.deepEqual([log, none.host.page, none.calls.filter((call) => call[0] === "getElementById")], [[], "settings", []]);
});
