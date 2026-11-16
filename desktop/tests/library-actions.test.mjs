// ./desktop/tests/library-actions.test.mjs
// Characterization tests for the library actions extracted from the route (helper slice): bulk selection
// of the visible cards or rows, selecting files to review, whether a source still needs a metadata lookup,
// opening the metadata review dialog, choosing a poster image, opening a library poster and resolving the
// media entries and folder a review targets. Dialogs and DOM timing are injected fakes.
// Run: pnpm test:library-actions

import assert from "node:assert/strict";
import test from "node:test";
import { createLibraryActions } from "../src/lib/library-actions.ts";

const entry = (path) => ({ path, name: path.split("/").pop() });
const series = (over = {}) => ({ recordId: "", artworkCandidates: [], localSeasonPosterPaths: {}, certification: "", ...over });

function harness({ hostOver = {}, pick = null, seriesGroups = {}, seriesReviews = {}, runStates = {} } = {}) {
  const log = [];
  const calls = [];
  const host = {
    selectedPaths: ["/keep"], libraryPosterCards: [], visibleEntries: [], currentPageEntries: [], queuedEntries: [], runReviewEntries: [], librarySeriesGroups: [],
    appPreferences: { metadataAutoLookup: true }, queueMovieReviews: {}, queueMatchSuggestions: {}, metadataFocusPath: "", libraryTvReview: null,
    metadataDialogOpen: false, metadataDialogElement: null, libraryBrowse: null, ...hostOver,
  };
  host.setNotice = (message, kind = "info") => log.push([kind, message]);
  const actions = {
    queueRunStateForPath: (path) => runStates[path],
    reviewedSeriesForPath: (path) => seriesReviews[path],
    seriesGroupForRoot: (root) => seriesGroups[root],
    ensureQueueArtworkPreview: async (...args) => { calls.push(["ensureQueueArtworkPreview", ...args]); },
    patchSeriesReview: (root, update) => { calls.push(["patchSeriesReview", root, update({ localSeasonPosterPaths: { 1: "/old" }, localPosterPath: "", posterId: "p" })]); },
    openMetadataForEntry: async (item) => { calls.push(["openMetadataForEntry", item.path]); },
    openDialog: async (options) => { calls.push(["openDialog", options]); return pick; },
    tick: async () => { calls.push(["tick"]); },
  };
  return { lib: createLibraryActions({ host, actions }), host, log, calls };
}

test("setVisibleSelection selects or deselects the media of the visible cards", () => {
  const cards = [{ mediaPaths: ["/a", "/b"] }, { mediaPaths: ["/b", "/c"] }];
  const h = harness({ hostOver: { libraryPosterCards: cards } });
  h.lib.setVisibleSelection(true);
  assert.deepEqual(h.host.selectedPaths, ["/keep", "/a", "/b", "/c"]);
  h.lib.setVisibleSelection(false);
  assert.deepEqual(h.host.selectedPaths, ["/keep"]);
});

test("setFilteredSelection selects or deselects the filtered table rows", () => {
  const h = harness({ hostOver: { visibleEntries: [entry("/x"), entry("/y")], selectedPaths: ["/y", "/z"] } });
  h.lib.setFilteredSelection(true);
  assert.deepEqual(h.host.selectedPaths, ["/y", "/z", "/x"]);
  h.lib.setFilteredSelection(false);
  assert.deepEqual(h.host.selectedPaths, ["/z"]);
});

test("selectFilesToReview selects the unfinished files and words the notice by interruption and count", () => {
  const one = harness({ hostOver: { runReviewEntries: [entry("/a")] } });
  one.lib.selectFilesToReview();
  assert.deepEqual([one.host.selectedPaths, one.log], [["/a"], [["info", "1 unfinished or skipped file selected for review."]]]);
  const many = harness({ hostOver: { runReviewEntries: [entry("/a"), entry("/b")] } });
  many.lib.selectFilesToReview();
  assert.deepEqual(many.log, [["info", "2 unfinished or skipped files selected for review."]]);
  const none = harness({ hostOver: { runReviewEntries: [] } });
  none.lib.selectFilesToReview();
  assert.deepEqual(none.log, [["info", "0 unfinished or skipped files selected for review."]]);
  const interrupted = harness({ hostOver: { runReviewEntries: [entry("/a"), entry("/b")] }, runStates: { "/b": { status: "interrupted" } } });
  interrupted.lib.selectFilesToReview();
  assert.deepEqual(interrupted.log, [["info", "Interrupted files are selected. Check their last report and output before retrying; existing outputs are never overwritten."]]);
});

const source = (...paths) => ({ scanResult: { entries: paths.map(entry) } });

test("sourceNeedsMetadataLookup is false without auto lookup or a scan", () => {
  assert.equal(harness({ hostOver: { appPreferences: { metadataAutoLookup: false } } }).lib.sourceNeedsMetadataLookup(source("/a")), false);
  assert.equal(harness().lib.sourceNeedsMetadataLookup({ scanResult: null }), false);
  assert.equal(harness().lib.sourceNeedsMetadataLookup(source()), false);
});

test("sourceNeedsMetadataLookup treats matched movies as done and unsearched or searching files as pending", () => {
  const matched = harness({ hostOver: { queueMovieReviews: { "/a": { recordId: "m1" } } } });
  assert.equal(matched.lib.sourceNeedsMetadataLookup(source("/a")), false);
  const fresh = harness();
  assert.equal(fresh.lib.sourceNeedsMetadataLookup(source("/a")), true);
  for (const [status, expected] of [["searching", true], ["matched", false], ["none", false]]) {
    const h = harness({ hostOver: { queueMatchSuggestions: { "/a": { status } } } });
    assert.equal(h.lib.sourceNeedsMetadataLookup(source("/a")), expected, status);
  }
  const any = harness({ hostOver: { queueMovieReviews: { "/a": { recordId: "m1" } } } });
  assert.equal(any.lib.sourceNeedsMetadataLookup(source("/a", "/b")), true);
});

test("sourceNeedsMetadataLookup for provider series wants provider art, season art and a certificate", () => {
  const art = [{ imageUrl: "https://img/p.jpg", thumbnailUrl: "" }];
  const full = series({ recordId: "tmdb1", artworkCandidates: art, localSeasonPosterPaths: { 1: "https://img/s1.jpg" }, certification: "TV-14" });
  const group = { episodePaths: ["/Show/e1.mkv"], seasons: [{ season: 1 }, { season: 0 }] };
  const make = (review, groups = [group]) => harness({ seriesReviews: { "/Show/e1.mkv": review }, hostOver: { librarySeriesGroups: groups } });
  assert.equal(make(full).lib.sourceNeedsMetadataLookup(source("/Show/e1.mkv")), false);
  assert.equal(make({ ...full, artworkCandidates: [] }).lib.sourceNeedsMetadataLookup(source("/Show/e1.mkv")), true);
  assert.equal(make({ ...full, artworkCandidates: [{ imageUrl: "/local", thumbnailUrl: "https://t/x" }] }).lib.sourceNeedsMetadataLookup(source("/Show/e1.mkv")), false);
  assert.equal(make({ ...full, localSeasonPosterPaths: { 1: "/local.jpg" } }).lib.sourceNeedsMetadataLookup(source("/Show/e1.mkv")), true);
  assert.equal(make({ ...full, certification: "  " }).lib.sourceNeedsMetadataLookup(source("/Show/e1.mkv")), true);
  assert.equal(make({ ...full, localSeasonPosterPaths: { 7: "https://img/x" } }, []).lib.sourceNeedsMetadataLookup(source("/Show/e1.mkv")), false);
  assert.equal(make({ ...full, localSeasonPosterPaths: { 7: "/x" } }, []).lib.sourceNeedsMetadataLookup(source("/Show/e1.mkv")), true);
  const local = series({ recordId: "local-1" });
  assert.equal(make(local).lib.sourceNeedsMetadataLookup(source("/Show/e1.mkv")), true);
  // A local record skips the provider-art rules and follows the file's own suggestion status instead.
  const localMatched = harness({ seriesReviews: { "/Show/e1.mkv": local }, hostOver: { librarySeriesGroups: [group], queueMatchSuggestions: { "/Show/e1.mkv": { status: "matched" } } } });
  assert.equal(localMatched.lib.sourceNeedsMetadataLookup(source("/Show/e1.mkv")), false);
  const localSearching = harness({ seriesReviews: { "/Show/e1.mkv": local }, hostOver: { librarySeriesGroups: [group], queueMatchSuggestions: { "/Show/e1.mkv": { status: "searching" } } } });
  assert.equal(localSearching.lib.sourceNeedsMetadataLookup(source("/Show/e1.mkv")), true);
  // A review without a record id also follows the suggestion status.
  const noRecord = harness({ seriesReviews: { "/Show/e1.mkv": series() }, hostOver: { queueMatchSuggestions: { "/Show/e1.mkv": { status: "none" } } } });
  assert.equal(noRecord.lib.sourceNeedsMetadataLookup(source("/Show/e1.mkv")), false);
});

test("showLibraryReview focuses a sample path, opens the dialog and shows the modal when still closed", async () => {
  const shown = [];
  const dialog = { open: false, showModal: () => shown.push("modal") };
  const target = { level: "episode", seriesRoot: "/Show", season: 1, episodePath: "/Show/e1.mkv" };
  const h = harness({ hostOver: { metadataDialogElement: dialog } });
  await h.lib.showLibraryReview(target);
  assert.deepEqual([h.host.metadataFocusPath, h.host.libraryTvReview, h.host.metadataDialogOpen, shown], ["/Show/e1.mkv", target, true, ["modal"]]);
  const open = harness({ hostOver: { metadataDialogElement: { open: true, showModal: () => shown.push("again") } } });
  await open.lib.showLibraryReview(target);
  assert.deepEqual(shown, ["modal"]);
  const bySeries = harness({ seriesGroups: { "/Show": { episodePaths: ["/Show/first.mkv"] } }, hostOver: { metadataFocusPath: "/prior" } });
  await bySeries.lib.showLibraryReview({ level: "series", seriesRoot: "/Show", season: null, episodePath: null });
  assert.equal(bySeries.host.metadataFocusPath, "/Show/first.mkv");
  const unknown = harness({ hostOver: { metadataFocusPath: "/prior" } });
  await unknown.lib.showLibraryReview({ level: "series", seriesRoot: "/Nowhere", season: null, episodePath: null });
  assert.equal(unknown.host.metadataFocusPath, "/prior");
});

test("chooseReviewImage patches the series, season or episode poster with the picked image", async () => {
  const series1 = { level: "series", seriesRoot: "/Show", season: null, episodePath: null };
  const h = harness({ pick: "/img/p.png", hostOver: { libraryTvReview: series1 }, seriesGroups: { "/Show": { episodePaths: ["/Show/e1.mkv"] } } });
  await h.lib.chooseReviewImage();
  assert.deepEqual(h.calls[0], ["openDialog", { directory: false, multiple: false, title: "Choose a poster image", defaultPath: "/Show", filters: [{ name: "Images", extensions: ["jpg", "jpeg", "png", "webp"] }] }]);
  assert.deepEqual(h.calls.slice(1), [["ensureQueueArtworkPreview", "/Show/e1.mkv", "/img/p.png"], ["patchSeriesReview", "/Show", { localSeasonPosterPaths: { 1: "/old" }, localPosterPath: "/img/p.png", posterId: "" }]]);
  for (const level of ["season", "episode"]) {
    const target = { level, seriesRoot: "/Show", season: 2, episodePath: level === "episode" ? "/Show/e9.mkv" : null };
    const s = harness({ pick: ["/img/s.png", "/ignored"], hostOver: { libraryTvReview: target }, seriesGroups: { "/Show": { episodePaths: ["/Show/e1.mkv"] } } });
    await s.lib.chooseReviewImage();
    assert.deepEqual(s.calls.at(-1), ["patchSeriesReview", "/Show", { localSeasonPosterPaths: { 1: "/old", 2: "/img/s.png" }, localPosterPath: "", posterId: "p" }]);
    assert.equal(s.calls.some((call) => call[0] === "ensureQueueArtworkPreview"), true);
  }
  const noSample = harness({ pick: "/i.png", hostOver: { libraryTvReview: series1 } });
  await noSample.lib.chooseReviewImage();
  assert.equal(noSample.calls.some((call) => call[0] === "ensureQueueArtworkPreview"), false);
  const cancelled = harness({ pick: null, hostOver: { libraryTvReview: series1 } });
  await cancelled.lib.chooseReviewImage();
  assert.deepEqual(cancelled.calls.map((call) => call[0]), ["openDialog"]);
  const noTarget = harness({ pick: "/i.png" });
  await noTarget.lib.chooseReviewImage();
  assert.equal(noTarget.calls.length, 0);
  const seasonNull = harness({ pick: "/i.png", hostOver: { libraryTvReview: { level: "season", seriesRoot: "/Show", season: null, episodePath: null } } });
  await seasonNull.lib.chooseReviewImage();
  assert.deepEqual(seasonNull.calls.at(-1)[2], { localSeasonPosterPaths: { 1: "/old" }, localPosterPath: "/i.png", posterId: "" });
});

test("openLibraryPoster drills into series and seasons, or opens the entry's metadata", () => {
  const cards = [{ path: "/Show", kind: "series", seasonNumber: null }, { path: "/Show::season:2", kind: "season", seasonNumber: 2 }, { path: "/Film.mkv", kind: "movie", seasonNumber: null }];
  const h = harness({ hostOver: { libraryPosterCards: cards, libraryBrowse: { seriesRoot: "/Show", season: null }, queuedEntries: [entry("/Film.mkv")] } });
  h.lib.openLibraryPoster("/Show");
  assert.deepEqual(h.host.libraryBrowse, { seriesRoot: "/Show", season: null });
  h.lib.openLibraryPoster("/Show::season:2");
  assert.deepEqual(h.host.libraryBrowse, { seriesRoot: "/Show", season: 2 });
  h.lib.openLibraryPoster("/Film.mkv");
  assert.deepEqual(h.calls, [["openMetadataForEntry", "/Film.mkv"]]);
  const noBrowse = harness({ hostOver: { libraryPosterCards: [cards[1]], libraryBrowse: null } });
  noBrowse.lib.openLibraryPoster("/Show::season:2");
  assert.deepEqual([noBrowse.host.libraryBrowse, noBrowse.calls.length], [null, 0]);
});

test("openLibraryPoster looks the entry up in visible, page then queued entries and ignores unknown paths", () => {
  const order = harness({ hostOver: { visibleEntries: [entry("/v")], currentPageEntries: [entry("/p")], queuedEntries: [entry("/q")] } });
  for (const path of ["/v", "/p", "/q", "/none"]) order.lib.openLibraryPoster(path);
  assert.deepEqual(order.calls.map((call) => call[1]), ["/v", "/p", "/q"]);
});

test("reviewMediaEntries resolves episode, season and series targets to queued entries", () => {
  const group = { episodePaths: ["C:/S/e1.mkv", "C:/S/e2.mkv", "C:/S/missing.mkv"], seasons: [{ season: 2, episodePaths: ["C:/S/e2.mkv"], directory: "C:/S/Season 2" }] };
  const h = harness({ seriesGroups: { "C:/S": group }, hostOver: { queuedEntries: [entry("c:\\s\\E1.mkv"), entry("C:/S/e2.mkv")] } });
  const paths = (target) => h.lib.reviewMediaEntries(target).map((item) => item.path);
  assert.deepEqual(paths({ level: "episode", seriesRoot: "C:/S", season: 1, episodePath: "C:/S/e2.mkv" }), ["C:/S/e2.mkv"]);
  assert.deepEqual(paths({ level: "season", seriesRoot: "C:/S", season: 2, episodePath: null }), ["C:/S/e2.mkv"]);
  assert.deepEqual(paths({ level: "season", seriesRoot: "C:/S", season: 9, episodePath: null }), []);
  assert.deepEqual(paths({ level: "series", seriesRoot: "C:/S", season: null, episodePath: null }), ["c:\\s\\E1.mkv", "C:/S/e2.mkv"]);
  assert.deepEqual(paths({ level: "series", seriesRoot: "C:/Unknown", season: null, episodePath: null }), []);
  assert.deepEqual(paths({ level: "episode", seriesRoot: "C:/S", season: 1, episodePath: null }), ["c:\\s\\E1.mkv", "C:/S/e2.mkv"]);
});

test("reviewLocationPath points at the episode, season folder or series root, or the fallback", () => {
  const group = { seasons: [{ season: 2, directory: "/S/Season 2" }, { season: 3, directory: "" }] };
  const h = harness({ seriesGroups: { "/S": group } });
  assert.equal(h.lib.reviewLocationPath(null, "/fb"), "/fb");
  assert.equal(h.lib.reviewLocationPath({ level: "episode", seriesRoot: "/S", season: 2, episodePath: "/S/e.mkv" }, "/fb"), "/S/e.mkv");
  assert.equal(h.lib.reviewLocationPath({ level: "episode", seriesRoot: "/S", season: 2, episodePath: null }, "/fb"), "/S");
  assert.equal(h.lib.reviewLocationPath({ level: "season", seriesRoot: "/S", season: 2, episodePath: null }, "/fb"), "/S/Season 2");
  assert.equal(h.lib.reviewLocationPath({ level: "season", seriesRoot: "/S", season: 3, episodePath: null }, "/fb"), "/S");
  assert.equal(h.lib.reviewLocationPath({ level: "season", seriesRoot: "/X", season: 1, episodePath: null }, "/fb"), "/X");
  assert.equal(h.lib.reviewLocationPath({ level: "series", seriesRoot: "/S", season: null, episodePath: null }, "/fb"), "/S");
});
