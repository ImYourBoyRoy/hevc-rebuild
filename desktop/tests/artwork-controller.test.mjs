// ./desktop/tests/artwork-controller.test.mjs
// Characterization tests for the artwork controller extracted from the route (artwork slice):
// queue poster previews (library-root checks, the 24-entry cache, lazy loading through an
// intersection observer) and the app artwork cache inspect/purge flow. The native bridge and the
// observer are injected fakes. Run: pnpm test:artwork-controller

import assert from "node:assert/strict";
import test from "node:test";
import { createArtworkController } from "../src/lib/artwork-controller.ts";
import { queuePathKey } from "../src/lib/queue-paths.ts";

const source = (root, paths, inputKind = "folder") => ({ root, scanResult: { root, input_kind: inputKind, entries: paths.map((path) => ({ path })) } });

function harness({ desktop = true, invokeImpl = {}, hostOver = {}, observerFactory } = {}) {
  const log = [];
  const invokes = [];
  const host = {
    queueArtworkPreviews: {}, queueArtworkPreviewErrors: {}, queueArtworkPreviewBusy: {},
    artworkCacheInfo: null, artworkCacheLoading: false, artworkCachePurging: false, artworkCacheConfirm: false, artworkCacheError: "",
    queueSources: [source("/lib", ["/lib/a.mkv", "/lib/b.mkv"])],
    setNotice: (message, kind = "info") => log.push([kind, message]),
    ...hostOver,
  };
  const invoke = async (command, args) => {
    invokes.push({ command, args });
    const impl = invokeImpl[command];
    if (impl === undefined) throw new Error(`unexpected ${command}`);
    return typeof impl === "function" ? impl(args) : impl;
  };
  const controller = createArtworkController({ invoke, isDesktop: () => desktop, host, createObserver: observerFactory ?? null });
  return { controller, host, log, invokes };
}

const preview = (dataUrl) => ({ read_local_library_artwork_preview: { dataUrl } });

test("queueArtworkRoot finds the library root that holds the media or the poster", () => {
  const h = harness({ hostOver: { queueSources: [source("/one", ["/one/a.mkv"]), source("/two", ["/two/b.mkv"])] } });
  assert.equal(h.controller.queueArtworkRoot("/two/b.mkv", "/two/poster.jpg"), "/two");
  assert.equal(h.controller.queueArtworkRoot("/unknown.mkv", "/one/poster.jpg"), "/one", "falls back to every source and matches the poster");
  assert.equal(h.controller.queueArtworkRoot("/elsewhere/c.mkv", "/elsewhere/p.jpg"), null);
});

test("queueArtworkRoot uses the containing folder for a single-file source and skips unscanned sources", () => {
  const file = harness({ hostOver: { queueSources: [{ root: "/x", scanResult: null }, source("/lib/movie.mkv", ["/lib/movie.mkv"], "file")] } });
  assert.equal(file.controller.queueArtworkRoot("/lib/movie.mkv", "/lib/poster.jpg"), "/lib/");
  const windows = harness({ hostOver: { queueSources: [source("C:\\lib\\movie.mkv", ["C:\\lib\\movie.mkv"], "file")] } });
  assert.equal(windows.controller.queueArtworkRoot("C:\\lib\\movie.mkv", "C:\\lib\\poster.jpg"), "C:\\lib\\");
});

test("ensureQueueArtworkPreview loads once, stores the data url and clears busy and error", async () => {
  const h = harness({ invokeImpl: preview("data:image/png;base64,AAA") });
  const key = queuePathKey("/lib/poster.jpg");
  h.host.queueArtworkPreviewErrors = { [key]: "old error" };
  const pending = h.controller.ensureQueueArtworkPreview("/lib/a.mkv", "/lib/poster.jpg");
  assert.equal(h.host.queueArtworkPreviewBusy[key], true);
  assert.equal(h.host.queueArtworkPreviewErrors[key], "");
  await pending;
  assert.deepEqual(h.invokes, [{ command: "read_local_library_artwork_preview", args: { request: { root: "/lib", path: "/lib/poster.jpg" } } }]);
  assert.equal(h.host.queueArtworkPreviews[key], "data:image/png;base64,AAA");
  assert.equal(h.host.queueArtworkPreviewBusy[key], false);
});

test("ensureQueueArtworkPreview skips cached, busy and empty requests", async () => {
  const key = queuePathKey("/lib/poster.jpg");
  const cached = harness({ hostOver: { queueArtworkPreviews: { [key]: "data:cached" } } });
  await cached.controller.ensureQueueArtworkPreview("/lib/a.mkv", "/lib/poster.jpg");
  const busy = harness({ hostOver: { queueArtworkPreviewBusy: { [key]: true } } });
  await busy.controller.ensureQueueArtworkPreview("/lib/a.mkv", "/lib/poster.jpg");
  const empty = harness();
  await empty.controller.ensureQueueArtworkPreview("/lib/a.mkv", "");
  assert.deepEqual([cached.invokes.length, busy.invokes.length, empty.invokes.length], [0, 0, 0]);
});

test("ensureQueueArtworkPreview refuses a poster outside the analyzed library", async () => {
  const h = harness({ invokeImpl: preview("x") });
  await h.controller.ensureQueueArtworkPreview("/other/a.mkv", "/other/poster.jpg");
  assert.equal(h.invokes.length, 0);
  assert.equal(h.host.queueArtworkPreviewErrors[queuePathKey("/other/poster.jpg")], "The selected poster is outside the analyzed library.");
  assert.deepEqual(h.host.queueArtworkPreviewBusy, {});
});

test("ensureQueueArtworkPreview records a read failure and always clears busy", async () => {
  const h = harness({ invokeImpl: { read_local_library_artwork_preview: () => { throw new Error("unreadable"); } } });
  await h.controller.ensureQueueArtworkPreview("/lib/a.mkv", "/lib/poster.jpg");
  const key = queuePathKey("/lib/poster.jpg");
  assert.equal(h.host.queueArtworkPreviewErrors[key], "Error: unreadable");
  assert.equal(h.host.queueArtworkPreviewBusy[key], false);
  assert.equal(h.host.queueArtworkPreviews[key], undefined);
});

test("ensureQueueArtworkPreview keeps at most 24 previews, dropping the oldest", async () => {
  const existing = Object.fromEntries(Array.from({ length: 24 }, (_, index) => [`k${index}`, `data:${index}`]));
  const h = harness({ invokeImpl: preview("data:new"), hostOver: { queueArtworkPreviews: existing } });
  await h.controller.ensureQueueArtworkPreview("/lib/a.mkv", "/lib/poster.jpg");
  const keys = Object.keys(h.host.queueArtworkPreviews);
  assert.equal(keys.length, 24);
  assert.equal(keys.includes("k0"), false);
  assert.equal(keys.includes(queuePathKey("/lib/poster.jpg")), true);
  assert.equal(keys.includes("k1"), true);
});

function fakeObserver() {
  const created = [];
  const factory = (callback, options) => {
    const observer = { callback, options, observed: [], disconnected: 0, observe(node) { this.observed.push(node); }, disconnect() { this.disconnected += 1; } };
    created.push(observer);
    return observer;
  };
  return { created, factory };
}

test("observeQueueArtwork loads immediately when no observer is available", async () => {
  const h = harness({ invokeImpl: preview("data:now") });
  const action = h.controller.observeQueueArtwork({}, { mediaPath: "/lib/a.mkv", artworkPath: "/lib/poster.jpg" });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(h.invokes.length, 1);
  action.destroy();
});

test("observeQueueArtwork waits for visibility, then loads once and disconnects", async () => {
  const fake = fakeObserver();
  const h = harness({ invokeImpl: preview("data:visible"), observerFactory: fake.factory });
  const node = { id: "card" };
  h.controller.observeQueueArtwork(node, { mediaPath: "/lib/a.mkv", artworkPath: "/lib/poster.jpg" });
  assert.equal(fake.created.length, 1);
  assert.deepEqual([fake.created[0].options, fake.created[0].observed], [{ rootMargin: "160px" }, [node]]);
  fake.created[0].callback([{ isIntersecting: false }]);
  assert.equal(h.invokes.length, 0);
  fake.created[0].callback([{ isIntersecting: false }, { isIntersecting: true }]);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(h.invokes.length, 1);
  assert.equal(fake.created[0].disconnected >= 1, true);
});

test("observeQueueArtwork ignores an empty poster and re-observes only when the params change", () => {
  const fake = fakeObserver();
  const h = harness({ observerFactory: fake.factory });
  const empty = h.controller.observeQueueArtwork({}, { mediaPath: "/lib/a.mkv", artworkPath: "" });
  assert.equal(fake.created.length, 0);
  empty.destroy();
  const node = {};
  const action = h.controller.observeQueueArtwork(node, { mediaPath: "/lib/a.mkv", artworkPath: "/lib/p1.jpg" });
  action.update({ mediaPath: "/lib/a.mkv", artworkPath: "/lib/p1.jpg" });
  assert.equal(fake.created.length, 1);
  action.update({ mediaPath: "/lib/a.mkv", artworkPath: "/lib/p2.jpg" });
  assert.equal(fake.created.length, 2);
  assert.equal(fake.created[0].disconnected, 1);
  action.destroy();
  assert.equal(fake.created[1].disconnected, 1);
});

test("refreshArtworkCache loads the cache info and resets the purge confirmation", async () => {
  const info = { available: true, imageCount: 5, purgeableImageCount: 2, previewId: "p1" };
  const h = harness({ invokeImpl: { inspect_app_artwork_cache: info }, hostOver: { artworkCacheConfirm: true, artworkCacheError: "old" } });
  const pending = h.controller.refreshArtworkCache();
  assert.equal(h.host.artworkCacheLoading, true);
  await pending;
  assert.deepEqual([h.host.artworkCacheInfo, h.host.artworkCacheLoading, h.host.artworkCacheError, h.host.artworkCacheConfirm], [info, false, "", false]);
});

test("refreshArtworkCache is skipped off desktop or while busy and reports a failure", async () => {
  for (const [options, hostOver] of [[{ desktop: false }, {}], [{}, { artworkCacheLoading: true }], [{}, { artworkCachePurging: true }]]) {
    const h = harness({ ...options, hostOver });
    await h.controller.refreshArtworkCache();
    assert.equal(h.invokes.length, 0);
  }
  const bad = harness({ invokeImpl: { inspect_app_artwork_cache: () => { throw new Error("denied"); } } });
  await bad.controller.refreshArtworkCache();
  assert.deepEqual([bad.host.artworkCacheError, bad.host.artworkCacheLoading], ["Error: denied", false]);
});

test("purgeUnusedArtworkCache needs a confirmed, purgeable preview and sends its id", async () => {
  const remaining = { purgeableImageCount: 0, previewId: "p2" };
  const h = harness({
    invokeImpl: { purge_unused_app_artwork_cache: { remaining, message: "Removed 2 images.", failedFileCount: 0 } },
    hostOver: { artworkCacheInfo: { purgeableImageCount: 2, previewId: "p1" }, artworkCacheConfirm: true },
  });
  await h.controller.purgeUnusedArtworkCache();
  assert.deepEqual(h.invokes[0], { command: "purge_unused_app_artwork_cache", args: { previewId: "p1", confirmed: true } });
  assert.deepEqual([h.host.artworkCacheInfo, h.host.artworkCacheConfirm, h.host.artworkCachePurging], [remaining, false, false]);
  assert.deepEqual(h.log, [["success", "Removed 2 images."]]);
  const partial = harness({
    invokeImpl: { purge_unused_app_artwork_cache: { remaining, message: "Removed 1, 1 failed.", failedFileCount: 1 } },
    hostOver: { artworkCacheInfo: { purgeableImageCount: 2, previewId: "p1" }, artworkCacheConfirm: true },
  });
  await partial.controller.purgeUnusedArtworkCache();
  assert.deepEqual(partial.log, [["error", "Removed 1, 1 failed."]]);
});

test("purgeUnusedArtworkCache does nothing without confirmation, purgeable images or desktop", async () => {
  const info = { purgeableImageCount: 2, previewId: "p1" };
  for (const [options, hostOver] of [
    [{}, { artworkCacheInfo: info, artworkCacheConfirm: false }],
    [{}, { artworkCacheInfo: { purgeableImageCount: 0, previewId: "p" }, artworkCacheConfirm: true }],
    [{}, { artworkCacheInfo: null, artworkCacheConfirm: true }],
    [{}, { artworkCacheInfo: info, artworkCacheConfirm: true, artworkCachePurging: true }],
    [{ desktop: false }, { artworkCacheInfo: info, artworkCacheConfirm: true }],
  ]) {
    const h = harness({ ...options, hostOver });
    await h.controller.purgeUnusedArtworkCache();
    assert.equal(h.invokes.length, 0);
  }
});

test("purgeUnusedArtworkCache failure shows the error and drops the confirmation", async () => {
  const h = harness({
    invokeImpl: { purge_unused_app_artwork_cache: () => { throw new Error("busy"); } },
    hostOver: { artworkCacheInfo: { purgeableImageCount: 2, previewId: "p1" }, artworkCacheConfirm: true },
  });
  await h.controller.purgeUnusedArtworkCache();
  assert.deepEqual([h.host.artworkCacheError, h.host.artworkCacheConfirm, h.host.artworkCachePurging], ["Error: busy", false, false]);
});
