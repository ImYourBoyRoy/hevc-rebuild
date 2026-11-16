// ./desktop/tests/local-assets.test.mjs
// Characterization tests for the local artwork discovery controller extracted from MetadataWorkbench (T10.7 slice 2):
// scanning folders for artwork beside media, resetting when the library root changes, ignoring stale scans, looking up
// a file's or folder's assets and loading poster previews (at most four per call, never twice). The native bridge is a fake.
// Run: pnpm test:local-assets

import assert from "node:assert/strict";
import test from "node:test";
import { createLocalAssets } from "../src/lib/local-assets.ts";

const file = (path) => ({ path });
const asset = (path, ...artwork) => ({ path, artwork: artwork.map((name) => ({ path: `${path}/${name}`, name, role: "poster" })) });

function harness({ desktop = true, over = {}, scans = [], previews = {} } = {}) {
  const calls = [];
  const host = { localAssets: null, localAssetScanError: "", localAssetGeneration: 0, localArtworkPreviews: {}, localArtworkPreviewErrors: {}, localArtworkPreviewBusy: {}, ...over };
  const queue = [...scans];
  const invoke = async (command, args) => {
    calls.push({ command, args });
    if (command === "scan_local_library_assets") { const next = queue.shift(); if (typeof next === "function") return next(args); if (next instanceof Error) throw next; return next; }
    if (command === "read_local_library_artwork_preview") { const value = previews[args.request.path]; if (value instanceof Error) throw value; return { dataUrl: value ?? "data:x" }; }
    throw new Error(`unexpected ${command}`);
  };
  return { ctl: createLocalAssets({ invoke, isDesktop: () => desktop, host }), host, calls };
}

test("ensureLocalAssets does nothing outside the desktop shell", async () => {
  const h = harness({ desktop: false });
  assert.equal(await h.ctl.ensureLocalAssets("/lib", [file("/lib/a.mkv")]), null);
  assert.deepEqual([h.calls.length, h.host.localAssets], [0, null]);
});

test("ensureLocalAssets scans unknown media and folders, then merges the result", async () => {
  const found = { root: "/lib", files: [asset("/lib/a.mkv", "a-poster.jpg")], directories: [asset("/lib/Show", "poster.jpg")] };
  const h = harness({ scans: [found] });
  const result = await h.ctl.ensureLocalAssets("\\lib", [file("/lib/a.mkv")], ["/lib/Show", "/lib//Show"]);
  assert.deepEqual(h.calls[0], { command: "scan_local_library_assets", args: { request: { root: "/lib", mediaPaths: ["/lib/a.mkv"], directoryPaths: ["/lib/Show"] } } });
  assert.deepEqual(result, { root: "/lib", files: found.files, directories: found.directories });
  assert.equal(h.host.localAssets, result);
  assert.equal(h.host.localAssetGeneration, 1);
  assert.equal(h.host.localAssetScanError, "");
});

test("ensureLocalAssets skips the scan when everything is known and keeps merged entries by normalized path", async () => {
  const first = { root: "/lib", files: [asset("/lib/a.mkv", "old.jpg")], directories: [] };
  const second = { root: "/lib", files: [asset("/lib/a.mkv", "new.jpg"), asset("/lib/b.mkv")], directories: [] };
  const h = harness({ scans: [first, second] });
  await h.ctl.ensureLocalAssets("/lib", [file("/lib/a.mkv")]);
  const known = await h.ctl.ensureLocalAssets("/lib", [file("/lib//a.mkv")]);
  assert.equal(h.calls.length, 1);
  assert.equal(known, h.host.localAssets);
  const merged = await h.ctl.ensureLocalAssets("/lib", [file("/lib/a.mkv"), file("/lib/b.mkv")]);
  assert.deepEqual(h.calls[1].args.request.mediaPaths, ["/lib/b.mkv"]);
  assert.deepEqual(merged.files.map((entry) => [entry.path, entry.artwork.length]), [["/lib/a.mkv", 1], ["/lib/b.mkv", 0]]);
  assert.equal(merged.files[0].artwork[0].name, "new.jpg");
});

test("ensureLocalAssets keeps the known root for a folder inside it, and resets for an unrelated or enclosing root", async () => {
  const h = harness({ scans: [{ root: "/lib", files: [], directories: [] }, { root: "/lib", files: [], directories: [] }, { root: "/other", files: [], directories: [] }, { root: "/", files: [], directories: [] }] });
  await h.ctl.ensureLocalAssets("/lib", [file("/lib/a.mkv")]);
  h.host.localArtworkPreviews = { "/p": "data" }; h.host.localArtworkPreviewErrors = { "/p": "x" };
  await h.ctl.ensureLocalAssets("/lib/Show", [file("/lib/Show/b.mkv")]);
  assert.deepEqual([h.calls[1].args.request.root, h.host.localAssetGeneration, Object.keys(h.host.localArtworkPreviews).length], ["/lib", 1, 1]);
  await h.ctl.ensureLocalAssets("/other", [file("/other/c.mkv")]);
  assert.deepEqual([h.calls[2].args.request.root, h.host.localAssetGeneration, h.host.localArtworkPreviews, h.host.localArtworkPreviewErrors], ["/other", 3, {}, {}]);
  await h.ctl.ensureLocalAssets("/", [file("/other/c.mkv")]);
  assert.deepEqual([h.calls[3].args.request.root, h.host.localAssetGeneration], ["/", 5]);
});

test("ensureLocalAssets reports a scan failure unless a newer scan already replaced the state", async () => {
  const failed = harness({ scans: [new Error("denied")] });
  assert.equal(await failed.ctl.ensureLocalAssets("/lib", [file("/lib/a.mkv")]), null);
  assert.equal(failed.host.localAssetScanError, "Error: denied");
  const stale = harness({ scans: [(args) => { stale.host.localAssetGeneration += 1; throw new Error("late"); }] });
  assert.equal(await stale.ctl.ensureLocalAssets("/lib", [file("/lib/a.mkv")]), null);
  assert.equal(stale.host.localAssetScanError, "");
  const staleOk = harness({ scans: [() => { staleOk.host.localAssetGeneration += 1; return { root: "/lib", files: [asset("/lib/a.mkv")], directories: [] }; }] });
  assert.equal(await staleOk.ctl.ensureLocalAssets("/lib", [file("/lib/a.mkv")]), null);
  assert.deepEqual(staleOk.host.localAssets.files, []);
  const cleared = harness({ over: { localAssetScanError: "old" }, scans: [{ root: "/lib", files: [], directories: [] }] });
  await cleared.ctl.ensureLocalAssets("/lib", [file("/lib/a.mkv")]);
  assert.equal(cleared.host.localAssetScanError, "");
});

test("localFileAssets and localDirectoryAssets find entries by normalized path", () => {
  const h = harness({ over: { localAssets: { root: "/lib", files: [asset("/lib/a.mkv")], directories: [asset("/lib/Show")] } } });
  assert.equal(h.ctl.localFileAssets("/lib//a.mkv").path, "/lib/a.mkv");
  assert.equal(h.ctl.localFileAssets("/lib/none.mkv"), undefined);
  assert.equal(h.ctl.localDirectoryAssets("\\lib\\Show").path, "/lib/Show");
  assert.equal(harness().ctl.localFileAssets("/lib/a.mkv"), undefined);
  assert.equal(harness().ctl.localDirectoryAssets("/lib"), undefined);
});

test("loadLocalArtworkPreviews loads up to four previews, once each, and records failures", async () => {
  const images = ["a", "b", "c", "d", "e"].map((name) => ({ path: `/lib/${name}.jpg`, name, role: "poster" }));
  const h = harness({ over: { localAssets: { root: "/lib", files: [], directories: [] } }, previews: { "/lib/b.jpg": new Error("unreadable"), "/lib/a.jpg": "data:a" } });
  await h.ctl.loadLocalArtworkPreviews(images);
  assert.deepEqual(h.calls.map((call) => call.args.request), images.slice(0, 4).map((image) => ({ root: "/lib", path: image.path })));
  assert.deepEqual(h.host.localArtworkPreviews, { "/lib/a.jpg": "data:a", "/lib/c.jpg": "data:x", "/lib/d.jpg": "data:x" });
  assert.deepEqual(h.host.localArtworkPreviewErrors, { "/lib/a.jpg": "", "/lib/b.jpg": "Error: unreadable", "/lib/c.jpg": "", "/lib/d.jpg": "" });
  assert.equal(Object.values(h.host.localArtworkPreviewBusy).some(Boolean), false);
  await h.ctl.loadLocalArtworkPreviews(images.slice(0, 1));
  assert.equal(h.calls.length, 4);
  h.host.localArtworkPreviewBusy = { "/lib/e.jpg": true };
  await h.ctl.loadLocalArtworkPreviews(images.slice(4));
  assert.equal(h.calls.length, 4);
});

test("loadLocalArtworkPreviews needs the desktop shell and a known asset root", async () => {
  const images = [{ path: "/lib/a.jpg", name: "a", role: "poster" }];
  const noDesktop = harness({ desktop: false, over: { localAssets: { root: "/lib", files: [], directories: [] } } });
  await noDesktop.ctl.loadLocalArtworkPreviews(images);
  const noRoot = harness();
  await noRoot.ctl.loadLocalArtworkPreviews(images);
  assert.deepEqual([noDesktop.calls.length, noRoot.calls.length], [0, 0]);
  // Without a root nothing may be marked busy or failed either.
  assert.deepEqual([noRoot.host.localArtworkPreviewBusy, noRoot.host.localArtworkPreviewErrors, noDesktop.host.localArtworkPreviewBusy], [{}, {}, {}]);
});

test("previewLocalArtwork returns an attachment that starts the preview load", async () => {
  const images = [{ path: "/lib/a.jpg", name: "a", role: "poster" }];
  const h = harness({ over: { localAssets: { root: "/lib", files: [], directories: [] } } });
  const attachment = h.ctl.previewLocalArtwork(images);
  assert.equal(h.calls.length, 0);
  attachment({});
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(h.calls.map((call) => call.command), ["read_local_library_artwork_preview"]);
});
