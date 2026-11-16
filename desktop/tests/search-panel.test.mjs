// ./desktop/tests/search-panel.test.mjs
// Characterization tests for the title search panel extracted from MetadataWorkbench (T10.7 slice 5): opening a search for a movie
// or series record, running the search with its guards and state resets, closing it, the series year mismatch, the collapsed
// candidate lists and the series root being reviewed. The host is a plain object; the provider calls are fakes.
// Run: pnpm test:search-panel

import assert from "node:assert/strict";
import test from "node:test";
import { createSearchPanel, initialCandidateCount } from "../src/lib/search-panel.ts";
import { candidateEvidence } from "../src/lib/metadata-candidates.ts";
import { rankMetadataMatches } from "../src/lib/metadata-match.ts";

const result = (id, title, year, provider = "tmdb", kind = "movie") => ({ id, provider, kind, title, year, description: null, rating: null, sourceUrl: "", confidence: 0, artwork: [] });
const clue = (title, year = null) => ({ title, year, yearEnd: null, certification: null });

function harness({ desktop = true, over = {}, clues = {}, search = () => ({ results: [], warnings: [], successfulProviders: [] }) } = {}) {
  const calls = [];
  const host = {
    searchTarget: { kind: "movie", path: "/old", label: "old" }, mediaKind: "movie", query: "before", results: [result("x", "Old", "2000")], searchWarnings: ["w"], selected: result("s", "Sel", "2001"), detail: { title: "d" },
    manualSearchClue: clue("old"), expandedMatchLists: { keep: true, "manual-search": true }, error: "stale", searching: false,
    seriesYearConflictConfirmed: true, organizerSource: null, selectedAnalysisSeriesRoot: "", organizerScan: null,
    renamePreview: { previewId: "p" }, confirmRename: true, pendingArtworkSave: { label: "x" }, artworkError: "e", artworkMessage: "m",
    seriesSuggestions: {}, selectedMediaPaths: ["/keep"], ...over,
  };
  const actions = {
    chooseAnalyzedSeries: (root) => calls.push(["chooseAnalyzedSeries", root]),
    filenameClue: async (title) => { calls.push(["filenameClue", title]); const value = clues[title]; if (value instanceof Error) throw value; return value ?? clue(title); },
    searchPublicTitles: async (kind, title) => { calls.push(["searchPublicTitles", kind, title]); const value = search(kind, title); if (value instanceof Error) throw value; return value; },
  };
  return { panel: createSearchPanel({ host, actions, isDesktop: () => desktop }), host, calls };
}
const tick = () => new Promise((resolve) => setImmediate(resolve));

test("searchMovieRecord selects the file, opens a movie search and resets the dependent state", async () => {
  const h = harness();
  h.panel.searchMovieRecord({ path: "/lib/Heat.1995.mkv", name: "Heat.1995.mkv" });
  assert.deepEqual(h.host.selectedMediaPaths, ["/keep", "/lib/Heat.1995.mkv"]);
  assert.deepEqual(h.host.searchTarget, { kind: "movie", path: "/lib/Heat.1995.mkv", label: "Heat.1995.mkv" });
  assert.deepEqual([h.host.mediaKind, h.host.query, h.host.results, h.host.searchWarnings, h.host.selected, h.host.detail, h.host.manualSearchClue], ["movie", "Heat.1995.mkv", [], [], null, null, null]);
  assert.deepEqual([h.host.seriesYearConflictConfirmed, h.host.error, h.host.renamePreview, h.host.confirmRename, h.host.pendingArtworkSave, h.host.artworkError, h.host.artworkMessage], [false, "", null, false, null, "", ""]);
  assert.deepEqual(h.host.expandedMatchLists, { keep: true, "manual-search": false });
  await tick();
  assert.equal(h.host.query, "Heat.1995.mkv");
  h.panel.searchMovieRecord({ path: "/keep", name: "k.mkv" }, "  Preferred Title  ");
  assert.deepEqual([h.host.selectedMediaPaths, h.host.query], [["/keep", "/lib/Heat.1995.mkv"], "Preferred Title"]);
});

test("the cleaned filename title replaces the query only while the same search is still open", async () => {
  const h = harness({ clues: { "Heat.1995.mkv": clue("Heat", "1995") } });
  h.panel.searchMovieRecord({ path: "/a", name: "Heat.1995.mkv" });
  await tick();
  assert.equal(h.host.query, "Heat");
  const edited = harness({ clues: { "Heat.1995.mkv": clue("Heat") } });
  edited.panel.searchMovieRecord({ path: "/a", name: "Heat.1995.mkv" });
  edited.host.query = "typed by user";
  await tick();
  assert.equal(edited.host.query, "typed by user");
  const moved = harness({ clues: { "Heat.1995.mkv": clue("Heat") } });
  moved.panel.searchMovieRecord({ path: "/a", name: "Heat.1995.mkv" });
  moved.panel.closeTitleSearch();
  await tick();
  assert.equal(moved.host.query, "Heat.1995.mkv");
  const failed = harness({ clues: { "Heat.1995.mkv": new Error("no engine") } });
  failed.panel.searchMovieRecord({ path: "/a", name: "Heat.1995.mkv" });
  await tick();
  assert.equal(failed.host.query, "Heat.1995.mkv");
});

test("searchSeriesRecord skips ambiguous groups, picks the analyzed series first and names the target", () => {
  const group = { root: "/lib/Show A", files: [{ name: "Show.A.S01E01.mkv" }], seasonDirectories: [], ambiguous: false };
  const ambiguous = harness();
  ambiguous.panel.searchSeriesRecord({ ...group, ambiguous: true });
  assert.deepEqual([ambiguous.host.query, ambiguous.calls.length], ["before", 0]);
  const analysis = harness({ over: { organizerSource: "analysis", selectedAnalysisSeriesRoot: "/lib/Other" } });
  analysis.panel.searchSeriesRecord(group);
  assert.deepEqual(analysis.calls[0], ["chooseAnalyzedSeries", "/lib/Show A"]);
  assert.deepEqual([analysis.host.searchTarget, analysis.host.mediaKind, analysis.host.query], [{ kind: "tv", root: "/lib/Show A", label: "Show A" }, "tv", "Show A"]);
  const same = harness({ over: { organizerSource: "analysis", selectedAnalysisSeriesRoot: "/lib/Show A" } });
  same.panel.searchSeriesRecord(group);
  const folder = harness({ over: { organizerSource: "folder", selectedAnalysisSeriesRoot: "/lib/Other" } });
  folder.panel.searchSeriesRecord(group, "  Better Name ");
  assert.deepEqual([same.calls.filter((call) => call[0] === "chooseAnalyzedSeries").length, folder.calls.filter((call) => call[0] === "chooseAnalyzedSeries").length, folder.host.query], [0, 0, "Better Name"]);
  const generic = harness();
  generic.panel.searchSeriesRecord({ root: "/lib/TV", files: [{ name: "Alpha.S01E01.mkv" }, { name: "Alpha.S01E02.mkv" }], seasonDirectories: [], ambiguous: false });
  assert.equal(generic.host.query, "Alpha");
});

test("searchCurrentSeriesRecord searches the scanned series, and only for television", () => {
  const scan = { root: "/lib/Show", files: [{ name: "Show.S01E01.mkv" }], seasonDirectories: [{ path: "/lib/Show/Season 1", name: "Season 1", season: 1 }] };
  const none = harness({ over: { organizerScan: null, mediaKind: "tv" } });
  none.panel.searchCurrentSeriesRecord();
  const movie = harness({ over: { organizerScan: scan, mediaKind: "movie" } });
  movie.panel.searchCurrentSeriesRecord();
  assert.deepEqual([none.host.query, movie.host.query], ["before", "before"]);
  const tv = harness({ over: { organizerScan: scan, mediaKind: "tv" } });
  tv.panel.searchCurrentSeriesRecord();
  assert.deepEqual([tv.host.searchTarget, tv.host.query], [{ kind: "tv", root: "/lib/Show", label: "Show" }, "Show"]);
});

test("closeTitleSearch clears the target, results, warnings and error but keeps the query", () => {
  const h = harness();
  h.panel.closeTitleSearch();
  assert.deepEqual([h.host.searchTarget, h.host.results, h.host.searchWarnings, h.host.error, h.host.query], [null, [], [], "", "before"]);
});

test("searchTitles explains each blocked case without touching the providers", async () => {
  const preview = harness({ desktop: false, over: { query: "Heat" } });
  await preview.panel.searchTitles();
  assert.equal(preview.host.error, "Metadata search runs in the installed desktop app. The browser preview cannot access the native provider client.");
  const noTarget = harness({ over: { query: "Heat", searchTarget: null } });
  await noTarget.panel.searchTitles();
  assert.equal(noTarget.host.error, "Open title search from a movie or series record so the result stays connected to that item.");
  const empty = harness({ over: { query: "   " } });
  await empty.panel.searchTitles();
  assert.equal(empty.host.error, "Enter a movie or series title to search.");
  assert.deepEqual([preview.calls.length, noTarget.calls.length, empty.calls.length, empty.host.searching], [0, 0, 0, false]);
  assert.equal(empty.host.results.length, 1);
});

test("searchTitles ranks the provider results by the cleaned filename clue and keeps the warnings", async () => {
  const rows = [result("2", "Heat", "2013"), result("1", "Heat", "1995"), result("3", "Heat", "1995", "wikidata")];
  const h = harness({ over: { query: "Heat 1995", mediaKind: "movie" }, clues: { "Heat 1995": clue("Heat", "1995") }, search: () => ({ results: rows, warnings: ["TMDB skipped"], successfulProviders: ["tmdb"] }) });
  const pending = h.panel.searchTitles();
  assert.equal(h.host.searching, true);
  assert.deepEqual([h.host.selected, h.host.detail, h.host.renamePreview, h.host.confirmRename, h.host.pendingArtworkSave, h.host.artworkError, h.host.artworkMessage, h.host.error, h.host.searchWarnings], [null, null, null, false, null, "", "", "", []]);
  await pending;
  assert.deepEqual(h.calls, [["filenameClue", "Heat 1995"], ["searchPublicTitles", "movie", "Heat"]]);
  assert.deepEqual(h.host.manualSearchClue, clue("Heat", "1995"));
  assert.deepEqual(h.host.results, rankMetadataMatches(rows, { ...clue("Heat", "1995"), kind: "movie" }));
  assert.equal(h.host.results[0].year, "1995");
  assert.deepEqual([h.host.searchWarnings, h.host.error, h.host.searching], [["TMDB skipped"], "", false]);
});

test("searchTitles reports an empty result list and failures, and always stops the spinner", async () => {
  const empty = harness({ over: { query: "Nothing" } });
  await empty.panel.searchTitles();
  assert.equal(empty.host.error, "No likely matches came back. Try the original title, a shorter title, or a year.");
  const failing = harness({ over: { query: "Heat" }, search: () => new Error("All enabled metadata sources failed.") });
  await failing.panel.searchTitles();
  assert.deepEqual([failing.host.error, failing.host.results, failing.host.searching], ["Error: All enabled metadata sources failed.", [], false]);
  const clueFails = harness({ over: { query: "Heat" }, clues: { Heat: new Error("engine down") } });
  await clueFails.panel.searchTitles();
  assert.deepEqual([clueFails.host.error, clueFails.host.searching, clueFails.calls.some((call) => call[0] === "searchPublicTitles")], ["Error: engine down", false, false]);
});

test("currentReviewedSeriesRoot prefers the open series target, then the scan root, then the chosen analysis root", () => {
  assert.equal(harness({ over: { searchTarget: { kind: "tv", root: "/lib/A", label: "A" } } }).panel.currentReviewedSeriesRoot(), "/lib/A");
  assert.equal(harness({ over: { searchTarget: { kind: "movie", path: "/m", label: "m" }, organizerScan: { root: "/lib/B" } } }).panel.currentReviewedSeriesRoot(), "/lib/B");
  assert.equal(harness({ over: { searchTarget: null, organizerScan: null, selectedAnalysisSeriesRoot: "/lib/C" } }).panel.currentReviewedSeriesRoot(), "/lib/C");
  assert.equal(harness({ over: { searchTarget: null, organizerScan: { root: "" }, selectedAnalysisSeriesRoot: "/lib/D" } }).panel.currentReviewedSeriesRoot(), "/lib/D");
});

test("currentSeriesYearMismatch needs a series result with detail and a conflicting year clue", () => {
  const selected = result("1", "Show", "2008", "tvdb", "tv");
  const base = { mediaKind: "tv", selected, detail: { title: "Show" }, searchTarget: { kind: "tv", root: "/lib/Show", label: "Show" }, manualSearchClue: clue("Show", "2015") };
  const h = harness({ over: base });
  assert.deepEqual(h.panel.currentSeriesYearMismatch(), { text: candidateEvidence(selected, clue("Show", "2015")).text, clue: clue("Show", "2015") });
  assert.equal(candidateEvidence(selected, clue("Show", "2015")).conflict, true);
  const saved = harness({ over: { ...base, manualSearchClue: null, seriesSuggestions: { "/lib/Show": { clue: clue("Show", "2012") } } } });
  assert.equal(saved.panel.currentSeriesYearMismatch().clue.year, "2012");
  const preferred = harness({ over: { ...base, seriesSuggestions: { "/lib/Show": { clue: clue("Show", "2008") } } } });
  assert.equal(preferred.panel.currentSeriesYearMismatch(), null);
  for (const over of [{ mediaKind: "movie" }, { selected: null }, { detail: null }, { manualSearchClue: null }]) assert.equal(harness({ over: { ...base, ...over } }).panel.currentSeriesYearMismatch(), null, JSON.stringify(Object.keys(over)));
});

test("candidate lists show four rows until expanded, and toggle per key", () => {
  assert.equal(initialCandidateCount, 4);
  const h = harness({ over: { expandedMatchLists: {} } });
  const items = [1, 2, 3, 4, 5, 6];
  assert.deepEqual(h.panel.visibleCandidates(items, "a"), [1, 2, 3, 4]);
  assert.equal(h.panel.candidateListExpanded("a"), false);
  h.panel.toggleCandidateList("a");
  assert.deepEqual([h.panel.visibleCandidates(items, "a"), h.panel.candidateListExpanded("a"), h.panel.visibleCandidates(items, "b")], [items, true, [1, 2, 3, 4]]);
  h.panel.toggleCandidateList("a");
  assert.equal(h.panel.candidateListExpanded("a"), false);
  assert.deepEqual(h.panel.visibleCandidates([1, 2], "a"), [1, 2]);
});

test("rankByFilenameYear ranks for the current media kind unless one is given", () => {
  const rows = [result("t", "Show", "2008", "tvdb", "tv"), result("m", "Show", "2008", "tmdb", "movie")];
  const asMovie = harness({ over: { mediaKind: "movie" } });
  assert.deepEqual(asMovie.panel.rankByFilenameYear(rows, clue("Show", "2008")).map((row) => row.id), rankMetadataMatches(rows, { ...clue("Show", "2008"), kind: "movie" }).map((row) => row.id));
  assert.equal(asMovie.panel.rankByFilenameYear(rows, clue("Show", "2008"))[0].id, "m");
  assert.equal(asMovie.panel.rankByFilenameYear(rows, clue("Show", "2008"), "tv")[0].id, "t");
});
