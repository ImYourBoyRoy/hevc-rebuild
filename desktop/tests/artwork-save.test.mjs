// ./desktop/tests/artwork-save.test.mjs
// Characterization tests for the poster saving extracted from MetadataWorkbench (T10.7 slice 9): staging a provider poster for
// saving, the planned file paths, saving it beside media or into the private cache and attaching it to the reviewed selections,
// choosing and saving a local poster for a film, the film poster shortcuts and the selected artwork lookups. Fake bridge and dialog.
// Run: pnpm test:artwork-save

import assert from "node:assert/strict";
import test from "node:test";
import { createArtworkSave } from "../src/lib/artwork-save.ts";

const art = (id, ext = "jpg") => ({ id, title: id, imageUrl: `https://img/${id}.${ext}`, thumbnailUrl: "", sourceUrl: "" });
const file = (path) => ({ path, name: path.split("/").at(-1), parentPath: path.split("/").slice(0, -1).join("/"), extension: "mkv" });
const target = (directory, basename) => ({ directory, basename });

function harness({ desktop = true, over = {}, payloads = {}, pick = null, selectedPaths = [] } = {}) {
  const calls = [];
  const host = {
    savePosterBesideMedia: true, movieAssignments: {}, seriesAssignments: {}, selectedArtwork: {}, organizerScan: { root: "/lib", files: [file("/lib/A.mkv"), file("/lib/B.mkv")] },
    selected: null, provider: "tmdb", detail: null, pendingArtworkSave: null, artworkBusy: false, artworkError: "e", artworkMessage: "m",
    pendingLocalPoster: null, pendingBatchLocalPoster: null, localPosterBusy: false, localPosterError: "e", localPosterMessage: "m", ...over,
  };
  const invoke = async (command, args) => {
    calls.push({ command, args });
    const value = payloads[command];
    if (value instanceof Error) throw value;
    return typeof value === "function" ? value(args) : value;
  };
  const actions = {
    openDialog: async (options) => { calls.push({ command: "openDialog", args: options }); return pick; },
    selectedPathsInOrganizer: () => selectedPaths,
    chooseMovieLocalPoster: (...args) => calls.push({ command: "chooseMovieLocalPoster", args }),
    loadLocalArtworkPreviews: async (images) => { calls.push({ command: "loadLocalArtworkPreviews", args: images }); },
  };
  return { art: createArtworkSave({ invoke, isDesktop: () => desktop, host, actions }), host, calls };
}
const commands = (h) => h.calls.map((call) => call.command);

test("stageArtworkSave stores the pending save with the current setting, and needs a poster, a scan and targets", () => {
  const h = harness();
  h.art.stageArtworkSave(art("p"), [target("/lib", "poster")], "label", [null]);
  assert.deepEqual(h.host.pendingArtworkSave, { candidate: art("p"), targets: [target("/lib", "poster")], label: "label", queueBindings: [null], saveBesideMedia: true });
  assert.deepEqual([h.host.artworkError, h.host.artworkMessage], ["", ""]);
  const cache = harness({ over: { savePosterBesideMedia: false } });
  cache.art.stageArtworkSave(art("p"), [target("/lib", "poster")], "label");
  assert.deepEqual([cache.host.pendingArtworkSave.saveBesideMedia, cache.host.pendingArtworkSave.queueBindings], [false, []]);
  for (const [candidate, targets, over] of [[undefined, [target("/a", "b")], {}], [art("p"), [], {}], [art("p"), [target("/a", "b")], { organizerScan: null }]]) {
    const blocked = harness({ over });
    blocked.art.stageArtworkSave(candidate, targets, "x");
    assert.deepEqual([blocked.host.pendingArtworkSave, blocked.host.artworkError], [null, "e"]);
  }
});

test("plannedArtworkPaths lists the files to write, or the private cache", () => {
  assert.deepEqual(harness().art.plannedArtworkPaths(), []);
  const beside = harness({ over: { pendingArtworkSave: { candidate: art("p", "png"), targets: [target("/lib/Film/", "Film-poster"), target("C:\\lib\\Show\\", "folder")], saveBesideMedia: true } } });
  assert.deepEqual(beside.art.plannedArtworkPaths(), ["/lib/Film/Film-poster.png", "C:\\lib\\Show/folder.png"]);
  const cache = harness({ over: { pendingArtworkSave: { candidate: art("p"), targets: [], saveBesideMedia: false } } });
  assert.deepEqual(cache.art.plannedArtworkPaths(), ["Private application artwork cache", "Poster source and license details saved with the cached image"]);
  assert.deepEqual(harness({ over: { pendingArtworkSave: { candidate: art("p", "gif"), targets: [target("/a", "b")], saveBesideMedia: true } } }).art.plannedArtworkPaths(), ["/a/b.jpg"]);
});

const pendingSave = (over = {}) => ({ candidate: art("p"), targets: [target("/lib", "poster"), target("/lib", "folder")], label: "x", saveBesideMedia: true, queueBindings: [], ...over });
const saved = { files: [{ path: "/lib/poster.jpg", attributionPath: "/lib/poster.txt" }, { path: "/lib/folder.jpg", attributionPath: "" }], bytesPerFile: 10, message: "Saved poster." };

test("applyArtworkSave writes beside media or into the cache, attaches matching selections and reports the outcome", async () => {
  const bindings = [{ kind: "movie", mediaPath: "/lib/A.mkv", candidateId: "p" }, { kind: "series", seriesRoot: "/lib/Show", candidateId: "p" }];
  const h = harness({ payloads: { save_library_artwork: saved }, over: { pendingArtworkSave: pendingSave({ queueBindings: bindings }), movieAssignments: { "/lib/A.mkv": { posterId: "p", localPosterPath: "" } }, seriesAssignments: { "/lib/Show": { posterId: "p", localPosterPath: "" } } } });
  const pending = h.art.applyArtworkSave();
  assert.deepEqual([h.host.artworkBusy, h.host.artworkError, h.host.artworkMessage], [true, "", ""]);
  await pending;
  assert.deepEqual(h.calls[0], { command: "save_library_artwork", args: { request: { root: "/lib", candidate: art("p"), targets: [target("/lib", "poster"), target("/lib", "folder")] } } });
  assert.deepEqual([h.host.movieAssignments["/lib/A.mkv"].localPosterPath, h.host.seriesAssignments["/lib/Show"].localPosterPath], ["/lib/poster.jpg", "/lib/folder.jpg"]);
  assert.equal(h.host.artworkMessage, "Saved poster. Saved 2 images. 2 reviewed poster selections are ready for the next MKV output.");
  assert.deepEqual([h.host.pendingArtworkSave, h.host.artworkBusy], [null, false]);
  const cache = harness({ payloads: { cache_library_artwork: { files: [{ path: "/cache/p.jpg", attributionPath: "" }], bytesPerFile: 1, message: "Cached." } }, over: { pendingArtworkSave: pendingSave({ saveBesideMedia: false, queueBindings: [bindings[0]] }), movieAssignments: { "/lib/A.mkv": { posterId: "p" } } } });
  await cache.art.applyArtworkSave();
  assert.deepEqual(cache.calls[0], { command: "cache_library_artwork", args: { candidate: art("p") } });
  assert.equal(cache.host.movieAssignments["/lib/A.mkv"].localPosterPath, "/cache/p.jpg");
  assert.equal(cache.host.artworkMessage, "Cached. Saved 1 image. 1 reviewed poster selection is ready for the next MKV output.");
  const two = harness({ payloads: { cache_library_artwork: { files: [{ path: "/cache/p.jpg", attributionPath: "" }], bytesPerFile: 1, message: "Cached." } }, over: { pendingArtworkSave: pendingSave({ saveBesideMedia: false, queueBindings: [bindings[0], { kind: "movie", mediaPath: "/lib/B.mkv", candidateId: "p" }] }), movieAssignments: { "/lib/A.mkv": { posterId: "p" }, "/lib/B.mkv": { posterId: "p" } } } });
  await two.art.applyArtworkSave();
  assert.deepEqual([two.host.movieAssignments["/lib/A.mkv"].localPosterPath, two.host.movieAssignments["/lib/B.mkv"].localPosterPath], ["/cache/p.jpg", "/cache/p.jpg"]);
});

test("applyArtworkSave attaches season posters, and skips selections that changed during the download", async () => {
  const season = { kind: "season", seriesRoot: "/lib/Show", seasonNumber: 2, candidateId: "p" };
  const h = harness({ payloads: { save_library_artwork: saved }, over: { selectedArtwork: { "season:2": "p" }, pendingArtworkSave: pendingSave({ queueBindings: [season] }), seriesAssignments: { "/lib/Show": { posterId: "x", localSeasonPosterPaths: { 1: "/s1.jpg" } } } } });
  await h.art.applyArtworkSave();
  assert.deepEqual(h.host.seriesAssignments["/lib/Show"].localSeasonPosterPaths, { 1: "/s1.jpg", 2: "/lib/poster.jpg" });
  const savedThree = { ...saved, files: [...saved.files, { path: "/lib/third.jpg", attributionPath: "" }] };
  const changed = harness({ payloads: { save_library_artwork: savedThree }, over: { selectedArtwork: { "season:2": "other" }, pendingArtworkSave: pendingSave({ queueBindings: [season, { kind: "movie", mediaPath: "/lib/A.mkv", candidateId: "p" }, { kind: "series", seriesRoot: "/lib/Show", candidateId: "p" }, null] }), movieAssignments: { "/lib/A.mkv": { posterId: "different" } }, seriesAssignments: { "/lib/Show": { posterId: "different", localSeasonPosterPaths: {} } } } });
  await changed.art.applyArtworkSave();
  assert.deepEqual([changed.host.movieAssignments["/lib/A.mkv"].localPosterPath, changed.host.seriesAssignments["/lib/Show"].localPosterPath, changed.host.seriesAssignments["/lib/Show"].localSeasonPosterPaths], [undefined, undefined, {}]);
  assert.equal(changed.host.artworkMessage, "Saved poster. Saved 3 images. The saved image was not attached to this review because its selected match changed while the download was in progress. Choose it as a local poster before creating an MKV.");
  const missingSeason = harness({ payloads: { save_library_artwork: saved }, over: { selectedArtwork: { "season:2": "p" }, pendingArtworkSave: pendingSave({ queueBindings: [season] }) } });
  await missingSeason.art.applyArtworkSave();
  assert.deepEqual(missingSeason.host.seriesAssignments, {});
  const noFile = harness({ payloads: { save_library_artwork: { files: [], bytesPerFile: 0, message: "None." } }, over: { pendingArtworkSave: pendingSave({ queueBindings: [{ kind: "movie", mediaPath: "/lib/A.mkv", candidateId: "p" }] }), movieAssignments: { "/lib/A.mkv": { posterId: "p" } } } });
  await noFile.art.applyArtworkSave();
  assert.deepEqual([noFile.host.movieAssignments["/lib/A.mkv"].localPosterPath, noFile.host.artworkMessage.startsWith("None. Saved 0 images.")], [undefined, true]);
});

test("applyArtworkSave needs the desktop app, a scan and a pending save, and keeps the pending save after a failure", async () => {
  for (const [desktop, over] of [[false, {}], [true, { organizerScan: null }], [true, { pendingArtworkSave: null }]]) {
    const h = harness({ desktop, over: { pendingArtworkSave: pendingSave(), ...over } });
    await h.art.applyArtworkSave();
    assert.deepEqual([h.calls.length, h.host.artworkError, h.host.artworkBusy], [0, "e", false]);
  }
  const failing = harness({ payloads: { save_library_artwork: new Error("disk full") }, over: { pendingArtworkSave: pendingSave() } });
  await failing.art.applyArtworkSave();
  assert.deepEqual([failing.host.artworkError, failing.host.artworkBusy, failing.host.pendingArtworkSave !== null, failing.host.artworkMessage], ["Error: disk full", false, true, ""]);
});

test("chooseLocalPoster stages the picked image for the film and validates its type", async () => {
  const film = file("/lib/Film (2001)/Film (2001).mkv");
  const h = harness({ pick: "/pics/poster.JPEG" });
  await h.art.chooseLocalPoster(film);
  assert.deepEqual(h.calls[0], { command: "openDialog", args: { directory: false, multiple: false, title: "Choose poster for Film (2001).mkv", filters: [{ name: "Poster image", extensions: ["jpg", "jpeg", "png", "webp"] }] } });
  assert.deepEqual(h.host.pendingLocalPoster, { sourcePath: "/pics/poster.JPEG", target: target("/lib/Film (2001)", "Film (2001)-poster"), targetPath: "/lib/Film (2001)/Film (2001)-poster.jpg", mediaPath: film.path, saveBesideMedia: true });
  assert.deepEqual([h.host.localPosterError, h.host.localPosterMessage], ["", ""]);
  const png = harness({ pick: "C:\\pics\\p.png" });
  await png.art.chooseLocalPoster(film);
  assert.equal(png.host.pendingLocalPoster.targetPath.endsWith("-poster.png"), true);
  const cache = harness({ pick: "/pics/p.webp", over: { savePosterBesideMedia: false } });
  await cache.art.chooseLocalPoster(film);
  assert.deepEqual([cache.host.pendingLocalPoster.targetPath, cache.host.pendingLocalPoster.saveBesideMedia], ["Private application artwork cache", false]);
  const bad = harness({ pick: "/pics/p.gif" });
  await bad.art.chooseLocalPoster(film);
  assert.deepEqual([bad.host.localPosterError, bad.host.pendingLocalPoster], ["Choose a JPEG, PNG, or WebP image.", null]);
  for (const blocked of [harness({ pick: null }), harness({ pick: ["/a.png"] }), harness({ desktop: false, pick: "/a.png" }), harness({ pick: "/a.png", over: { organizerScan: null } })]) {
    await blocked.art.chooseLocalPoster(film);
    assert.equal(blocked.host.pendingLocalPoster, null);
  }
});

test("saveLocalPoster copies the image beside the media or into the cache and selects it for the film", async () => {
  const pending = (over = {}) => ({ sourcePath: "/pics/p.png", target: target("/lib", "Film-poster"), targetPath: "/lib/Film-poster.png", mediaPath: "/lib/Film.mkv", saveBesideMedia: true, ...over });
  const h = harness({ payloads: { save_local_library_artwork: { files: ["/lib/Film-poster.png"], message: "Copied." } }, over: { pendingLocalPoster: pending() } });
  const run = h.art.saveLocalPoster();
  assert.deepEqual([h.host.localPosterBusy, h.host.localPosterError], [true, ""]);
  await run;
  assert.deepEqual(h.calls[0], { command: "save_local_library_artwork", args: { request: { root: "/lib", sourcePath: "/pics/p.png", targets: [target("/lib", "Film-poster")] } } });
  assert.deepEqual(h.calls[1], { command: "chooseMovieLocalPoster", args: ["/lib/Film.mkv", "/lib/Film-poster.png"] });
  assert.deepEqual(h.calls[2], { command: "loadLocalArtworkPreviews", args: [{ path: "/lib/Film-poster.png", name: "Film-poster.png", role: "Selected poster" }] });
  assert.deepEqual([h.host.localPosterMessage, h.host.pendingLocalPoster, h.host.localPosterBusy], ["Copied. Saved to /lib/Film-poster.png", null, false]);
  const cache = harness({ payloads: { cache_local_library_artwork: { files: ["/cache/x.png"], message: "Cached." } }, over: { pendingLocalPoster: pending({ saveBesideMedia: false }) } });
  await cache.art.saveLocalPoster();
  assert.deepEqual(cache.calls[0], { command: "cache_local_library_artwork", args: { sourcePath: "/pics/p.png" } });
  const noMedia = harness({ payloads: { save_local_library_artwork: { files: ["/lib/x.png"], message: "ok" } }, over: { pendingLocalPoster: pending({ mediaPath: "" }) } });
  await noMedia.art.saveLocalPoster();
  assert.deepEqual(commands(noMedia), ["save_local_library_artwork"]);
  const noFiles = harness({ payloads: { save_local_library_artwork: { files: [], message: "ok" } }, over: { pendingLocalPoster: pending() } });
  await noFiles.art.saveLocalPoster();
  assert.deepEqual([commands(noFiles), noFiles.host.localPosterMessage], [["save_local_library_artwork"], "ok Saved to "]);
  const failing = harness({ payloads: { save_local_library_artwork: new Error("denied") }, over: { pendingLocalPoster: pending() } });
  await failing.art.saveLocalPoster();
  assert.deepEqual([failing.host.localPosterError, failing.host.localPosterBusy, failing.host.pendingLocalPoster !== null], ["Error: denied", false, true]);
  for (const [desktop, over] of [[false, {}], [true, { organizerScan: null }], [true, { pendingLocalPoster: null }]]) {
    const blocked = harness({ desktop, over: { pendingLocalPoster: pending(), ...over } });
    await blocked.art.saveLocalPoster();
    assert.deepEqual([blocked.calls.length, blocked.host.localPosterError, blocked.host.localPosterBusy], [0, "e", false]);
  }
});

const filmState = (over = {}) => ({
  organizerScan: { root: "/lib", files: [file("/lib/A.mkv"), file("/lib/B.mkv"), file("/lib/C.mkv")] }, selected: { id: "603" }, provider: "tmdb", detail: { artwork: [art("p"), art("q")], seasons: [{ number: 1, artwork: [art("s1")] }] }, selectedArtwork: { movie: "p", series: "q", "season:1": "s1" },
  movieAssignments: {
    "/lib/A.mkv": { recordId: "tmdb:603", artworkCandidates: [art("p")], posterId: "old", localPosterPath: "/x.jpg" }, "/lib/B.mkv": { recordId: "tmdb:1", artworkCandidates: [art("p")], posterId: "old" },
    "/lib/C.mkv": { recordId: "tmdb:603", artworkCandidates: [art("z")], posterId: "old" },
  }, ...over,
});

test("the selected artwork lookups read the current detail and selection", () => {
  const h = harness({ over: filmState() });
  assert.deepEqual([h.art.selectedMovieArtwork().id, h.art.selectedSeriesArtwork().id, h.art.selectedSeasonArtwork(1).id], ["p", "q", "s1"]);
  assert.equal(h.art.selectedSeasonArtwork(null), undefined);
  const nullSeason = harness({ over: filmState({ detail: { artwork: [], seasons: [{ number: null, artwork: [art("x")] }] }, selectedArtwork: { "season:null": "x" } }) });
  assert.equal(nullSeason.art.selectedSeasonArtwork(null), undefined);
  assert.equal(h.art.selectedSeasonArtwork(2), undefined);
  const none = harness();
  assert.deepEqual([none.art.selectedMovieArtwork(), none.art.selectedSeriesArtwork(), none.art.selectedSeasonArtwork(1)], [undefined, undefined, undefined]);
  const stale = harness({ over: filmState({ selectedArtwork: { movie: "gone" } }) });
  assert.equal(stale.art.selectedMovieArtwork(), undefined);
});

test("film poster targets and bindings cover only the selected copies assigned to this film with that poster", () => {
  const h = harness({ over: filmState(), selectedPaths: ["/lib/A.mkv", "/lib/B.mkv", "/lib/C.mkv", "/lib/missing.mkv"] });
  assert.deepEqual(h.art.movieArtworkTargets(), [target("/lib", "A-poster")]);
  assert.deepEqual(h.art.movieArtworkBindings(art("p")), [{ kind: "movie", mediaPath: "/lib/A.mkv", candidateId: "p" }]);
  assert.deepEqual(h.art.movieArtworkBindings(art("z")), [{ kind: "movie", mediaPath: "/lib/C.mkv", candidateId: "z" }]);
  const noImage = harness({ over: filmState({ selectedArtwork: {} }), selectedPaths: ["/lib/A.mkv"] });
  assert.deepEqual(noImage.art.movieArtworkTargets(), []);
  const noSelection = harness({ over: filmState({ selected: null }), selectedPaths: ["/lib/A.mkv"] });
  assert.deepEqual([noSelection.art.movieArtworkTargets(), noSelection.art.movieArtworkBindings(art("p"))], [[], []]);
  const noScan = harness({ over: filmState({ organizerScan: null }), selectedPaths: ["/lib/A.mkv"] });
  assert.deepEqual([noScan.art.movieArtworkTargets(), noScan.art.movieArtworkBindings(art("p"))], [[], []]);
});

test("assignSelectedPosterToMatchingCopies and stageCurrentFilmPoster apply the poster to the matching copies and stage the save", () => {
  const h = harness({ over: filmState(), selectedPaths: ["/lib/A.mkv", "/lib/B.mkv", "/lib/C.mkv"] });
  h.art.assignSelectedPosterToMatchingCopies();
  assert.deepEqual([h.host.movieAssignments["/lib/A.mkv"].posterId, h.host.movieAssignments["/lib/A.mkv"].localPosterPath, h.host.movieAssignments["/lib/B.mkv"].posterId, h.host.movieAssignments["/lib/C.mkv"].posterId], ["p", "", "old", "old"]);
  const idle = harness({ over: filmState({ selectedArtwork: {} }), selectedPaths: ["/lib/A.mkv"] });
  idle.art.assignSelectedPosterToMatchingCopies();
  idle.art.stageCurrentFilmPoster();
  assert.deepEqual([idle.host.movieAssignments["/lib/A.mkv"].posterId, idle.host.pendingArtworkSave], ["old", null]);
  const staged = harness({ over: filmState(), selectedPaths: ["/lib/A.mkv", "/lib/B.mkv"] });
  staged.art.stageCurrentFilmPoster();
  assert.deepEqual(staged.host.pendingArtworkSave, { candidate: art("p"), targets: [target("/lib", "A-poster")], label: "poster for selected copies assigned to this film", queueBindings: [{ kind: "movie", mediaPath: "/lib/A.mkv", candidateId: "p" }], saveBesideMedia: true });
  const noScan = harness({ over: filmState({ organizerScan: null }), selectedPaths: ["/lib/A.mkv"] });
  noScan.art.stageCurrentFilmPoster();
  assert.equal(noScan.host.pendingArtworkSave, null);
});
