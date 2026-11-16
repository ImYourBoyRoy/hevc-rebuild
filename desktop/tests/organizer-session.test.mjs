// ./desktop/tests/organizer-session.test.mjs
// Characterization tests for the organizer session extracted from MetadataWorkbench (T10.7 slice 12): using the analyzed library,
// restoring and rebuilding a saved series review, choosing an analyzed series, resuming provider artwork for queued series,
// choosing and rescanning organizer folders, and changing the media kind. Plain-object host, fake bridge, dialog and actions.
// Run: pnpm test:organizer-session

import assert from "node:assert/strict";
import test from "node:test";
import { createOrganizerSession } from "../src/lib/organizer-session.ts";

const entry = (relative, root = "/lib") => ({ path: `${root}/${relative}`, relative_path: relative, name: relative.split("/").at(-1) });
const showScan = () => ({ root: "/lib", input_kind: "folder", entries: [entry("Show/Season 1/S01E01.Pilot.mkv"), entry("Show/Season 1/S01E02.Second.mkv"), entry("Film.2001.mkv")] });
const episode = (season, number, title = `E${number}`, airdate = null) => ({ season, episode: number, title, airdate, summary: null, sourceUrl: "", artwork: [] });
const file = (path, over = {}) => ({ path, name: path.split("/").at(-1), parentPath: "/lib", extension: "mkv", season: null, episode: null, multipleEpisodes: false, ...over });
const folderScan = (over = {}) => ({ root: "/lib/Show", renameRoot: "/lib", files: [file("/lib/Show/a.mkv", { season: 1, episode: 1 }), file("/lib/Show/b.mkv", { season: 1, episode: 2, multipleEpisodes: true }), file("/lib/Show/c.mkv")], seasonDirectories: [{ path: "/lib/Show/Season 1", name: "Season 1", season: 1 }], episodeGaps: [], errors: [], truncated: false, ...over });

function harness({ desktop = true, over = {}, payloads = {}, pick = null, clues = {}, suggestions = {}, ensure = async () => ({ root: "/lib" }) } = {}) {
  const calls = [];
  const host = {
    scanResult: null, metadataAutoLookup: true, seriesAssignments: {}, selectedMediaPaths: [], organizerSource: null, selectedAnalysisSeriesRoot: "", organizerScan: null, episodeAssignments: {},
    selectedArtwork: {}, tvRenameScope: "episodes-only", seriesYearConflictConfirmed: true, query: "stale", mediaKind: "movie", provider: "tmdb", selected: null, detail: null, results: [{ id: "x" }],
    searchWarnings: ["w"], searchTarget: { kind: "movie" }, error: "e", renamePreview: { previewId: "p" }, renameError: "e", renameBusy: false, confirmRename: true, pendingArtworkSave: { label: "x" },
    artworkError: "e", artworkMessage: "m", lookupBusy: false, localAssets: { root: "/x" }, localAssetGeneration: 3, localArtworkPreviews: { "/p": "d" }, localArtworkPreviewErrors: { "/p": "e" }, ...over,
  };
  const invoke = async (command, args) => {
    calls.push({ command, args });
    const value = payloads[command];
    if (value instanceof Error) throw value;
    return typeof value === "function" ? value(args) : value;
  };
  const record = (command) => (...args) => { calls.push({ command, args }); };
  const actions = {
    openDialog: async (options) => { calls.push({ command: "openDialog", args: options }); return pick; },
    seedMovieDrafts: async (files) => { calls.push({ command: "seedMovieDrafts", args: files.map((f) => f.path) }); },
    syncEpisodeAssignments: record("sync"),
    filenameClue: async (name) => { calls.push({ command: "filenameClue", args: name }); const value = clues[name]; if (value instanceof Error) throw value; return value ?? { title: `clean ${name}` }; },
    episodeForKey: (key) => { const [season, number] = (key ?? "").split("/").map(Number); return host.detail?.episodes.find((item) => item.season === season && item.episode === number); },
    ensureLocalAssets: async (...args) => { calls.push({ command: "ensureLocalAssets", args }); return ensure(...args); },
    findMovieMatches: record("findMovieMatches"), findSeriesMatches: record("findSeriesMatches"),
    suggestionForPath: (path) => suggestions[path], seedSeriesAssignment: (root, row) => { calls.push({ command: "seedSeriesAssignment", args: [root, row.id] }); host.seriesAssignments = { ...host.seriesAssignments, [root]: { title: row.title, artworkCandidates: [], localSeasonPosterPaths: {}, episodes: [], episodeAssignments: {}, seasonRecords: {} } }; },
    hydrateSeriesProvider: record("hydrate"),
  };
  return { session: createOrganizerSession({ invoke, isDesktop: () => desktop, host, actions }), host, calls };
}
const commands = (h) => h.calls.map((call) => call.command);
const tick = () => new Promise((resolve) => setImmediate(resolve));
const saved = (over = {}) => ({
  title: "Breaking Bad", year: "2008", certification: "TV-MA", recordId: "tvdb:81189", shortDescription: "Short", plot: "Plot", genresText: "Crime; Drama, Thriller", ratingText: "9.5", sourceUrl: "https://tvdb",
  comment: "note", artworkCandidates: [{ id: "p" }], posterId: "p", localPosterPath: "", localSeasonPosterPaths: {}, seasonRecords: {}, episodes: [{ season: 2, episode: 1, title: "Back", airdate: null, summary: null, sourceUrl: "" }, { season: 1, episode: 1, title: "Pilot", airdate: null, summary: null, sourceUrl: "" }],
  episodeAssignments: { "/lib/Show/a.mkv": { season: 1, episode: 1, title: "Pilot", airdate: null, summary: null, sourceUrl: "" } }, ...over,
});

test("restoreSavedSeriesReview rebuilds the open match, its detail and the episode assignments from a saved review", () => {
  const h = harness();
  h.session.restoreSavedSeriesReview(saved());
  assert.deepEqual([h.host.provider, h.host.selected.id, h.host.selected.title, h.host.selected.rating, h.host.selected.description, h.host.selected.confidence], ["tvdb", "81189", "Breaking Bad", 9.5, "Plot", 1]);
  assert.deepEqual([h.host.detail.genres, h.host.detail.seasons.map((season) => season.number), h.host.detail.episodes.length, h.host.detail.attribution, h.host.detail.certification, h.host.detail.rating], [["Crime", "Drama", "Thriller"], [1, 2], 2, "Saved review from TVDB", "TV-MA", 9.5]);
  assert.deepEqual([h.host.selectedArtwork, h.host.episodeAssignments], [{ series: "p" }, { "/lib/Show/a.mkv": "1/1" }]);
  const local = harness();
  local.session.restoreSavedSeriesReview(saved({ recordId: "weird", year: "", plot: "", shortDescription: "", ratingText: "", certification: "" }));
  assert.deepEqual([local.host.provider, local.host.selected.id, local.host.selected.year, local.host.selected.description, local.host.selected.rating, local.host.detail.attribution, local.host.detail.certification], ["local", "", null, null, null, "Saved review from an earlier session", null]);
  const record = harness();
  record.session.restoreSavedSeriesReview(saved({ recordId: "tmdb:1:2" }));
  assert.equal(record.host.selected.id, "1:2");
});

const detail = (over = {}) => ({ title: "Show", year: "2008", certification: "TV-MA", shortDescription: "S", longDescription: "L", genres: ["Drama", "Crime"], rating: 9, sourceUrl: "https://src", artwork: [{ id: "p" }], episodes: [episode(1, 1, "Pilot", "2008-01-20"), episode(1, 2, "Second")], ...over });
const tvState = (over = {}) => ({ mediaKind: "tv", detail: detail(), selected: { id: "5" }, provider: "tvdb", searchTarget: { kind: "tv", root: "/lib/Show" }, episodeAssignments: { "/lib/Show/a.mkv": "1/1", "/lib/Show/b.mkv": "1/2" }, selectedArtwork: { series: "sp" }, ...over });

test("makeCurrentSeriesReview builds the review from the open match, keeping what was saved before", () => {
  const h = harness({ over: tvState() });
  const review = h.session.makeCurrentSeriesReview();
  assert.deepEqual([review.title, review.year, review.certification, review.recordId, review.genresText, review.ratingText, review.posterId, review.comment, review.sourceUrl], ["Show", "2008", "TV-MA", "tvdb:5", "Drama, Crime", "9", "sp", "", "https://src"]);
  assert.deepEqual(Object.keys(review.episodeAssignments).sort(), ["/lib/Show/a.mkv", "/lib/Show/b.mkv"]);
  assert.deepEqual([review.episodes.length, review.episodeAssignments["/lib/Show/a.mkv"].airdate], [2, "2008-01-20"]);
  assert.notEqual(review.artworkCandidates[0], h.host.detail.artwork[0]);
  const previous = saved({ certification: "TV-14", comment: "keep", posterId: "old", localPosterPath: "/l.jpg", localSeasonPosterPaths: { 1: "/s.jpg" }, episodeAssignments: { "/lib/Show/a.mkv": { season: 1, episode: 1, title: "Mine", airdate: " ", summary: null, sourceUrl: "" }, "/lib/Show/gone.mkv": { season: 9, episode: 9, title: "g", airdate: "2000-01-01", summary: null, sourceUrl: "" } } });
  const keepsDate = harness({ over: tvState({ seriesAssignments: { "/lib/Show": saved({ episodeAssignments: { "/lib/Show/a.mkv": { season: 1, episode: 1, title: "Mine", airdate: "1999-12-31", summary: null, sourceUrl: "" } } }) }, episodeAssignments: {} }) }).session.makeCurrentSeriesReview();
  assert.equal(keepsDate.episodeAssignments["/lib/Show/a.mkv"].airdate, "1999-12-31");
  const merged = harness({ over: tvState({ seriesAssignments: { "/lib/Show": previous }, selectedArtwork: {}, detail: detail({ certification: null, sourceUrl: "", artwork: [] }) }) }).session.makeCurrentSeriesReview();
  assert.deepEqual([merged.certification, merged.comment, merged.posterId, merged.localPosterPath, merged.localSeasonPosterPaths, merged.sourceUrl, merged.artworkCandidates, merged.episodeAssignments["/lib/Show/a.mkv"].title, merged.episodeAssignments["/lib/Show/a.mkv"].airdate, merged.episodeAssignments["/lib/Show/b.mkv"].title], ["TV-14", "keep", "old", "/l.jpg", { 1: "/s.jpg" }, "https://tvdb", previous.artworkCandidates, "Mine", "2008-01-20", "Second"]);
});

test("makeCurrentSeriesReview needs a series match with a root, drops cleared episodes and keeps the saved record id for a local match", () => {
  for (const over of [{ detail: null }, { selected: null }, { mediaKind: "movie" }, { searchTarget: null, organizerScan: null, selectedAnalysisSeriesRoot: "" }]) assert.equal(harness({ over: tvState(over) }).session.makeCurrentSeriesReview(), null, JSON.stringify(Object.keys(over)));
  const viaScan = harness({ over: tvState({ searchTarget: null, organizerScan: folderScan() }) }).session.makeCurrentSeriesReview();
  assert.ok(viaScan);
  const cleared = harness({ over: tvState({ seriesAssignments: { "/lib/Show": saved() }, episodeAssignments: { "/lib/Show/a.mkv": "" } }) }).session.makeCurrentSeriesReview();
  assert.equal("/lib/Show/a.mkv" in cleared.episodeAssignments, false);
  const local = harness({ over: tvState({ provider: "local", selected: { id: "x" }, seriesAssignments: { "/lib/Show": saved({ recordId: "tvdb:77" }) } }) }).session.makeCurrentSeriesReview();
  assert.equal(local.recordId, "tvdb:77");
  const stillLocal = harness({ over: tvState({ provider: "local", selected: { id: "x" }, seriesAssignments: { "/lib/Show": saved({ recordId: "local-1" }) } }) }).session.makeCurrentSeriesReview();
  assert.equal(stillLocal.recordId, "local:x");
  const noEpisodes = harness({ over: tvState({ detail: detail({ episodes: [] }), seriesAssignments: { "/lib/Show": saved() } }) }).session.makeCurrentSeriesReview();
  assert.equal(noEpisodes.episodes.length, 2);
  const capped = harness({ over: tvState({ detail: detail({ episodes: Array.from({ length: 10_001 }, (_, i) => episode(1, i + 1)) }) }) }).session.makeCurrentSeriesReview();
  assert.equal(capped.episodes.length, 10_000);
});

test("useAnalyzedLibrary prepares the organizer scan, search query, saved review and episode assignments for the analyzed scan", async () => {
  const none = harness();
  none.session.useAnalyzedLibrary("tv");
  assert.deepEqual([none.host.organizerSource, none.calls.length], [null, 0]);
  const tv = harness({ over: { scanResult: showScan(), seriesAssignments: { "/lib/Show": saved() } }, clues: { Show: { title: "Show Clean" } } });
  tv.session.useAnalyzedLibrary("tv");
  assert.deepEqual([tv.host.organizerSource, tv.host.selectedAnalysisSeriesRoot, tv.host.organizerScan.root, tv.host.query, tv.host.provider, tv.host.selected.id], ["analysis", "/lib/Show", "/lib/Show", "Show", "tvdb", "81189"]);
  assert.equal(commands(tv).includes("sync"), true);
  await tick();
  assert.equal(tv.host.query, "Show Clean");
  const tvFresh = harness({ over: { scanResult: showScan() } });
  tvFresh.session.useAnalyzedLibrary("tv", "/lib/Show");
  assert.deepEqual([commands(tvFresh).includes("seedMovieDrafts"), commands(tvFresh).includes("sync"), tvFresh.host.selected], [false, false, null]);
  const explicit = harness({ over: { scanResult: showScan() } });
  explicit.session.useAnalyzedLibrary("tv", "/lib/Other");
  assert.deepEqual([explicit.host.selectedAnalysisSeriesRoot, explicit.host.organizerScan, explicit.host.query], ["/lib/Other", null, "Other"]);
  const movie = harness({ over: { scanResult: showScan() } });
  movie.session.useAnalyzedLibrary("movie");
  assert.deepEqual([movie.host.organizerScan.files.map((f) => f.name), movie.host.query, commands(movie).includes("seedMovieDrafts"), movie.host.episodeAssignments], [["Film.2001.mkv"], "Film.2001.mkv", true, {}]);
  const two = harness({ over: { scanResult: { root: "/lib", input_kind: "folder", entries: [entry("A.mkv"), entry("B.mkv")] } } });
  two.session.useAnalyzedLibrary("movie");
  assert.equal(two.host.query, "");
  const manySeries = harness({ over: { scanResult: { root: "/lib", input_kind: "folder", entries: [entry("A/S01E01.mkv"), entry("B/S01E01.mkv")] } } });
  manySeries.session.useAnalyzedLibrary("tv");
  assert.deepEqual([manySeries.host.selectedAnalysisSeriesRoot, manySeries.host.query, manySeries.host.organizerScan], ["", "", null]);
  const edited = harness({ over: { scanResult: showScan() }, clues: { Show: { title: "Clean" } } });
  edited.session.useAnalyzedLibrary("tv");
  edited.host.query = "typed";
  await tick();
  assert.equal(edited.host.query, "typed");
  const failed = harness({ over: { scanResult: showScan() }, clues: { Show: new Error("x") } });
  failed.session.useAnalyzedLibrary("tv");
  await tick();
  assert.equal(failed.host.query, "Show");
});

test("chooseAnalyzedSeries clears the open match and its dependents before using the series", () => {
  const h = harness({ over: { scanResult: showScan(), results: [{ id: "x" }], selected: { id: "s" }, detail: { title: "d" } } });
  h.session.chooseAnalyzedSeries("/lib/Show");
  assert.deepEqual([h.host.results, h.host.selected, h.host.detail, h.host.seriesYearConflictConfirmed, h.host.error, h.host.renamePreview, h.host.confirmRename, h.host.pendingArtworkSave, h.host.artworkError, h.host.artworkMessage], [[], null, null, false, "", null, false, null, "", ""]);
  assert.deepEqual([h.host.selectedAnalysisSeriesRoot, h.host.organizerSource], ["/lib/Show", "analysis"]);
});

const ready = (kind = "tv", recordId = "tvdb:81189") => ({ status: "ready", kind, provider: recordId.split(":")[0], recordId, title: "Show", year: "2008", description: "d", sourceUrl: "https://s", autoAssociated: false, artwork: null, error: "" });

test("resumeSeriesArtwork loads provider details for queued series whose artwork or text is incomplete, once each", () => {
  const scan = showScan();
  const key = "/lib/Show/Season 1/S01E01.Pilot.mkv";
  const h = harness({ over: { scanResult: scan }, suggestions: { [key]: ready() } });
  h.session.resumeSeriesArtwork(scan);
  assert.deepEqual(h.calls.filter((c) => c.command === "seedSeriesAssignment").map((c) => c.args), [["/lib/Show", "81189"]]);
  assert.deepEqual(h.calls.filter((c) => c.command === "hydrate").map((c) => [c.args[0], c.args[1].id]), [["/lib/Show", "81189"]]);
  h.session.resumeSeriesArtwork(scan);
  assert.equal(h.calls.filter((c) => c.command === "hydrate").length, 1);
  const done = harness({ over: { scanResult: scan, seriesAssignments: { "/lib/Show": { title: "Show", artworkCandidates: [{ imageUrl: "https://i/p.jpg", thumbnailUrl: "" }], localSeasonPosterPaths: { 1: "https://p/s1.jpg" }, episodes: [{ season: 1, episode: 1, title: "x" }], episodeAssignments: { [key]: { season: 1, episode: 1, title: "x", airdate: null, summary: "" }, "/lib/Show/Season 1/S01E02.Second.mkv": { season: 1, episode: 2, title: "y", airdate: null, summary: "" } }, seasonRecords: { 1: { title: "One" } } } } }, suggestions: { [key]: ready() } });
  done.session.resumeSeriesArtwork(scan);
  assert.equal(done.calls.filter((c) => c.command === "hydrate").length, 0);
  const incompleteText = harness({ over: { scanResult: scan, seriesAssignments: { "/lib/Show": { title: "Show", artworkCandidates: [{ imageUrl: "https://i/p.jpg", thumbnailUrl: "" }], localSeasonPosterPaths: { 1: "https://p/s1.jpg" }, episodes: [{ season: 1, episode: 1, title: "x" }, { season: 1, episode: 2, title: "y" }], episodeAssignments: { [key]: { season: 1, episode: 1, title: "x", airdate: null, summary: "" } }, seasonRecords: { 1: { title: "One" } } } } }, suggestions: { [key]: ready() } });
  incompleteText.session.resumeSeriesArtwork(scan);
  assert.equal(incompleteText.calls.filter((c) => c.command === "hydrate").length, 1);
  const flat = { root: "/lib", input_kind: "folder", entries: [entry("Show/S01E01.Pilot.mkv")] };
  const flatKey = "/lib/Show/S01E01.Pilot.mkv";
  const noSeasonFolders = harness({ over: { scanResult: flat, seriesAssignments: { "/lib/Show": { title: "Show", artworkCandidates: [{ imageUrl: "https://i/p.jpg", thumbnailUrl: "" }], localSeasonPosterPaths: {}, episodes: [{ season: 1, episode: 1, title: "x" }], episodeAssignments: { [flatKey]: { season: 1, episode: 1, title: "x", airdate: null, summary: "" } }, seasonRecords: {} } } }, suggestions: { [flatKey]: ready() } });
  noSeasonFolders.session.resumeSeriesArtwork(flat);
  assert.equal(noSeasonFolders.calls.filter((c) => c.command === "hydrate").length, 1);
  for (const over of [{ metadataAutoLookup: false }, { lookupBusy: true }]) {
    const blocked = harness({ over: { scanResult: scan, ...over }, suggestions: { [key]: ready() } });
    blocked.session.resumeSeriesArtwork(scan);
    assert.equal(blocked.calls.length, 0);
  }
  const preview = harness({ desktop: false, suggestions: { [key]: ready() } });
  preview.session.resumeSeriesArtwork(scan);
  assert.equal(preview.calls.length, 0);
  const movieOnly = harness({ suggestions: { [key]: ready("movie", "tmdb:1") } });
  movieOnly.session.resumeSeriesArtwork(scan);
  assert.equal(movieOnly.calls.length, 0);
  const skipped = harness({ over: { scanResult: scan } });
  skipped.session.resumeSeriesArtwork({ root: "/lib/TV", input_kind: "folder", entries: [entry("Alpha.S01E01.mkv", "/lib/TV"), entry("Beta.S01E01.mkv", "/lib/TV")] });
  assert.equal(skipped.calls.length, 0);
});

test("chooseOrganizerFolder scans the picked folder, resets the local artwork, selects episodes and starts the lookups", async () => {
  const scan = folderScan();
  const h = harness({ payloads: { scan_organizer_folder: scan }, pick: "/lib/Show", over: { mediaKind: "tv", selectedMediaPaths: ["/keep"] } });
  await h.session.chooseOrganizerFolder();
  assert.deepEqual(h.calls[0], { command: "openDialog", args: { directory: true, multiple: false, title: "Choose one TV series folder" } });
  assert.deepEqual(h.calls[1], { command: "scan_organizer_folder", args: { root: "/lib/Show" } });
  assert.deepEqual(h.calls[2], { command: "ensureLocalAssets", args: ["/lib/Show", scan.files, ["/lib/Show", "/lib/Show/Season 1"]] });
  assert.deepEqual([h.host.organizerScan, h.host.organizerSource, h.host.selectedAnalysisSeriesRoot, h.host.localAssets, h.host.localAssetGeneration, h.host.localArtworkPreviews, h.host.localArtworkPreviewErrors], [scan, "folder", "", null, 4, {}, {}]);
  assert.deepEqual(h.host.selectedMediaPaths, ["/keep", "/lib/Show/a.mkv"]);
  assert.deepEqual([commands(h).includes("sync"), commands(h).includes("seedMovieDrafts"), h.calls.find((c) => c.command === "findSeriesMatches").args], [true, false, [[{ root: "/lib/Show", files: scan.files, seasonDirectories: scan.seasonDirectories, ambiguous: false }]]]);
  assert.deepEqual([h.host.renameBusy, h.host.renameError, h.host.renamePreview, h.host.pendingArtworkSave], [false, "", null, null]);
  const movie = harness({ payloads: { scan_organizer_folder: scan }, pick: "/lib/Movies" });
  await movie.session.chooseOrganizerFolder();
  assert.equal(movie.calls[0].args.title, "Choose a movie folder");
  assert.deepEqual([commands(movie).includes("seedMovieDrafts"), commands(movie).includes("findMovieMatches"), movie.host.selectedMediaPaths], [true, true, []]);
  const noAssets = harness({ payloads: { scan_organizer_folder: scan }, pick: "/lib/Movies", ensure: async () => null });
  await noAssets.session.chooseOrganizerFolder();
  assert.deepEqual([commands(noAssets).includes("seedMovieDrafts"), commands(noAssets).includes("findMovieMatches"), commands(noAssets).includes("sync")], [false, false, true]);
  const noAuto = harness({ payloads: { scan_organizer_folder: scan }, pick: "/lib/Movies", over: { metadataAutoLookup: false } });
  await noAuto.session.chooseOrganizerFolder();
  assert.equal(commands(noAuto).includes("findMovieMatches"), false);
});

test("chooseOrganizerFolder explains a missing desktop shell, ignores a cancelled pick and reports a failed scan", async () => {
  const preview = harness({ desktop: false });
  await preview.session.chooseOrganizerFolder();
  assert.deepEqual([preview.host.renameError, preview.calls.length], ["Folder selection is available in the installed desktop app.", 0]);
  for (const pick of [null, ["/a", "/b"]]) {
    const h = harness({ pick });
    await h.session.chooseOrganizerFolder();
    assert.deepEqual([commands(h), h.host.renameBusy], [["openDialog"], false]);
  }
  const failing = harness({ pick: "/lib/Show", payloads: { scan_organizer_folder: new Error("denied") }, over: { organizerScan: folderScan() } });
  await failing.session.chooseOrganizerFolder();
  assert.deepEqual([failing.host.organizerScan, failing.host.renameError, failing.host.renameBusy], [null, "Error: denied", false]);
});

test("scanCurrentSeriesFolder rescans the open series folder and selects its episodes", async () => {
  const rescanned = folderScan({ files: [file("/lib/Show/a.mkv", { season: 1, episode: 1 }), file("/lib/Show/d.mkv", { season: 1, episode: 3 })] });
  const h = harness({ payloads: { scan_organizer_folder: rescanned }, over: { mediaKind: "tv", organizerScan: folderScan(), organizerSource: "analysis", selectedMediaPaths: ["/keep"], renamePreview: { previewId: "p" }, confirmRename: true } });
  await h.session.scanCurrentSeriesFolder();
  assert.deepEqual(h.calls[0], { command: "scan_organizer_folder", args: { root: "/lib/Show" } });
  assert.deepEqual([h.host.organizerScan, h.host.organizerSource, h.host.selectedMediaPaths, h.host.localAssetGeneration, h.host.renamePreview, h.host.confirmRename, h.host.renameBusy], [rescanned, "folder", ["/keep", "/lib/Show/a.mkv", "/lib/Show/d.mkv"], 4, null, false, false]);
  assert.deepEqual([commands(h).includes("sync"), commands(h).includes("findSeriesMatches")], [true, true]);
  const noAuto = harness({ payloads: { scan_organizer_folder: rescanned }, over: { mediaKind: "tv", organizerScan: folderScan(), metadataAutoLookup: false } });
  await noAuto.session.scanCurrentSeriesFolder();
  assert.equal(commands(noAuto).includes("findSeriesMatches"), false);
  const noAssets = harness({ payloads: { scan_organizer_folder: rescanned }, ensure: async () => null, over: { mediaKind: "tv", organizerScan: folderScan() } });
  await noAssets.session.scanCurrentSeriesFolder();
  assert.equal(commands(noAssets).includes("findSeriesMatches"), false);
  const failing = harness({ payloads: { scan_organizer_folder: new Error("gone") }, over: { mediaKind: "tv", organizerScan: folderScan() } });
  await failing.session.scanCurrentSeriesFolder();
  assert.deepEqual([failing.host.renameError, failing.host.renameBusy, failing.host.organizerScan !== null], ["Error: gone", false, true]);
  for (const [desktop, over] of [[false, { mediaKind: "tv", organizerScan: folderScan() }], [true, { mediaKind: "movie", organizerScan: folderScan() }], [true, { mediaKind: "tv", organizerScan: null }]]) {
    const blocked = harness({ desktop, over });
    await blocked.session.scanCurrentSeriesFolder();
    assert.deepEqual([blocked.calls.length, blocked.host.renameBusy], [0, false]);
  }
});

test("changeMediaKind resets the whole workbench for the new kind and prepares the analyzed library when there is one", () => {
  const h = harness({ over: { scanResult: showScan(), searchWarnings: ["w"], selected: { id: "s" }, detail: { title: "d" } } });
  h.session.changeMediaKind("tv");
  assert.equal(h.host.mediaKind, "tv");
  assert.deepEqual([h.host.results, h.host.searchWarnings, h.host.searchTarget, h.host.selected, h.host.detail, h.host.error, h.host.tvRenameScope, h.host.renamePreview, h.host.renameError, h.host.confirmRename, h.host.pendingArtworkSave, h.host.artworkError, h.host.artworkMessage], [[], [], null, null, null, "", "series-tree", null, "", false, null, "", ""]);
  assert.deepEqual([h.host.organizerSource, h.host.selectedAnalysisSeriesRoot, h.host.organizerScan.root], ["analysis", "/lib/Show", "/lib/Show"]);
  const bare = harness();
  bare.session.changeMediaKind("movie");
  assert.deepEqual([bare.host.query, bare.host.organizerScan, bare.host.organizerSource, bare.host.episodeAssignments], ["", null, null, {}]);
});
