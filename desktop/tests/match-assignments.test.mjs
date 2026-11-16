// ./desktop/tests/match-assignments.test.mjs
// Characterization tests for the saved-match and assignment helpers extracted from MetadataWorkbench (T10.7 slice 6): saved
// suggestions as match sets, whether a film still needs a search and its button label, poster choices, automatic and seeded
// movie and series assignments, filename drafts for films, and the catalog and season text applied to a series. Plain-object host.
// Run: pnpm test:match-assignments

import assert from "node:assert/strict";
import test from "node:test";
import { createMatchAssignments } from "../src/lib/match-assignments.ts";
import { episodeIdentity, fillEpisodeFromCatalog, fillSeasonFromProvider } from "../src/lib/library-browse.ts";
import { suggestionMatchSet } from "../src/lib/metadata-candidates.ts";

const art = (id, over = {}) => ({ id, title: `Poster ${id}`, imageUrl: `https://img/${id}.jpg`, thumbnailUrl: "", sourceUrl: "https://src/" + id, ...over });
const result = (over = {}) => ({ id: "603", provider: "tmdb", kind: "movie", title: "The Matrix", year: "1999-03-31", description: "A hacker.", rating: 8.2, sourceUrl: "https://tmdb/603", confidence: 1, artwork: [art("p1")], ...over });
const clue = (title, year = null, certification = null) => ({ title, year, yearEnd: null, certification });
const suggestion = (over = {}) => ({ status: "ready", kind: "movie", recordId: "tmdb:603", provider: "tmdb", title: "The Matrix", year: "1999", description: "A hacker.", sourceUrl: "https://tmdb/603", autoAssociated: false, artwork: art("sug"), error: "", ...over });
const file = (path, over = {}) => ({ path, name: path.split("/").at(-1), parentPath: path.split("/").slice(0, -1).join("/"), extension: "mkv", season: null, episode: null, multipleEpisodes: false, ...over });
const entry = (relative, root = "/lib") => ({ path: `${root}/${relative}`, relative_path: relative, name: relative.split("/").at(-1) });
const showScan = () => ({ root: "/lib", input_kind: "folder", entries: [entry("Show/Season 1/S01E01.Pilot.mkv"), entry("Show/Season 1/S01E02.Second.mkv"), entry("Show/Season 1/S01E03.Third.mkv"), entry("Show/Season 2/S02E01.Back.mkv")] });

function harness({ desktop = true, over = {}, ensure = async () => ({ root: "/lib", files: [], directories: [] }), clues = [] } = {}) {
  const calls = [];
  const host = {
    movieAssignments: {}, seriesAssignments: {}, queueSuggestions: {}, scanResult: null, textBlacklistPatterns: ["rip*"], organizerScan: null,
    localAssetScanError: "", lookupMessage: "", lookupMessageScope: null, ...over,
  };
  const invoke = async (command, args) => {
    calls.push({ command, args });
    if (command !== "metadata_filename_clues") throw new Error(`unexpected ${command}`);
    if (clues instanceof Error) throw clues;
    return clues;
  };
  const actions = { ensureLocalAssets: async (...args) => { calls.push({ command: "ensureLocalAssets", args }); return ensure(...args); } };
  return { match: createMatchAssignments({ invoke, isDesktop: () => desktop, host, actions }), host, calls };
}

test("suggestionForPath finds a saved suggestion by exact or normalized path", () => {
  const exact = suggestion({ title: "Exact" });
  const h = harness({ over: { queueSuggestions: { "/lib/a.mkv": exact, "C:\\lib\\b.mkv": suggestion({ title: "Win" }) } } });
  assert.equal(h.match.suggestionForPath("/lib/a.mkv"), exact);
  assert.equal(h.match.suggestionForPath("C:/lib//b.mkv").title, "Win");
  assert.equal(h.match.suggestionForPath("/lib/none.mkv"), undefined);
});

test("saved match sets come from the file's or the series' first television suggestion", () => {
  const movie = suggestion();
  const h = harness({ over: { queueSuggestions: { "/lib/Film.mkv": movie, "/lib/Show/e2.mkv": suggestion({ kind: "tv", recordId: "tvdb:5" }) } } });
  assert.deepEqual(h.match.savedMovieMatchSet(file("/lib/Film.mkv")), suggestionMatchSet(movie, "Film.mkv", "movie"));
  assert.equal(h.match.savedMovieMatchSet(file("/lib/Other.mkv")), null);
  const group = { root: "/lib/Show", files: [file("/lib/Show/e1.mkv"), file("/lib/Show/e2.mkv")] };
  assert.equal(h.match.savedSeriesMatchSet(group).results[0].id, "5");
  // A film suggestion on an earlier file must not hide the series suggestion.
  const mixed = harness({ over: { queueSuggestions: { "/lib/Show/e1.mkv": suggestion(), "/lib/Show/e2.mkv": suggestion({ kind: "tv", recordId: "tvdb:5" }) } } });
  assert.equal(mixed.match.savedSeriesMatchSet(group).results[0].id, "5");
  assert.equal(h.match.savedSeriesMatchSet({ root: "/lib/Show", files: [file("/lib/Film.mkv")] }), null);
});

test("a film needs a search until it has a record or a ready suggestion, and the label follows its status", () => {
  const h = harness({ over: { movieAssignments: { "/a": { recordId: "tmdb:1" }, "/b": { recordId: "" } }, queueSuggestions: { "/c": suggestion(), "/d": suggestion({ status: "searching" }) } } });
  const needs = (path) => h.match.movieNeedsSearch(file(path));
  assert.deepEqual(["/a", "/b", "/c", "/d", "/e"].map(needs), [false, true, false, true, true]);
  assert.equal(h.match.movieSearchCount(["/a", "/b", "/c", "/d", "/e"].map((path) => file(path))), 3);
  assert.equal(h.match.movieLookupLabel(file("/x"), { status: "searching" }), "Searching…");
  assert.equal(h.match.movieLookupLabel(file("/c"), null), "Review suggestion");
  assert.equal(h.match.movieLookupLabel(file("/e"), { status: "ready" }), "Find matches");
});

test("reviewPosterOptions puts the saved suggestion poster first and adds each result's preview once", () => {
  const h = harness({ over: { queueSuggestions: { "/a": suggestion({ artwork: art("sug") }) } } });
  const assignment = { artworkCandidates: [art("own")] };
  const rows = [result({ artwork: [art("r1")] }), result({ artwork: [art("own")] }), result({ artwork: [art("none", { imageUrl: "", thumbnailUrl: "" })] }), result({ artwork: [] })];
  assert.deepEqual(h.match.reviewPosterOptions("/a", assignment, rows).map((image) => image.id), ["sug", "own", "r1"]);
  assert.deepEqual(h.match.reviewPosterOptions("/none", undefined).map((image) => image.id), []);
  assert.deepEqual(h.match.reviewPosterOptions("/a", { artworkCandidates: [art("sug")] }, []).map((image) => image.id), ["sug"]);
  assert.deepEqual(assignment.artworkCandidates.map((image) => image.id), ["own"]);
});

test("autoAssignMovieMatch fills an empty or same-titled draft from the candidate and leaves a reviewed one alone", () => {
  const h = harness({ over: { movieAssignments: { "/other": { title: "keep" } } } });
  assert.equal(h.match.autoAssignMovieMatch("/a", clue("The Matrix", "1999"), result()), true);
  assert.deepEqual(h.host.movieAssignments["/a"], {
    title: "The Matrix", year: "1999", certification: "", recordId: "tmdb:603", shortDescription: "A hacker.", plot: "A hacker.", genresText: "", ratingText: "8.2",
    sourceUrl: "https://tmdb/603", artworkCandidates: [art("p1")], posterId: "p1", localPosterPath: "",
  });
  assert.deepEqual(h.host.movieAssignments["/other"], { title: "keep" });
  const draft = harness({ over: { movieAssignments: { "/a": { title: "the matrix!", year: "", certification: "R", recordId: "", shortDescription: "mine", plot: "", genresText: "Sci-Fi", ratingText: "", sourceUrl: "", artworkCandidates: [], posterId: "chosen", localPosterPath: "/p.jpg", note: "x" } } } });
  assert.equal(draft.match.autoAssignMovieMatch("/a", clue("The Matrix"), result({ year: null, rating: null, description: null })), true);
  const kept = draft.host.movieAssignments["/a"];
  assert.deepEqual([kept.certification, kept.shortDescription, kept.plot, kept.genresText, kept.ratingText, kept.posterId, kept.localPosterPath, kept.note, kept.year, kept.recordId], ["R", "mine", "", "Sci-Fi", "", "chosen", "/p.jpg", "x", "", "tmdb:603"]);
  const reviewed = harness({ over: { movieAssignments: { "/a": { title: "The Matrix", recordId: "tmdb:1" } } } });
  assert.equal(reviewed.match.autoAssignMovieMatch("/a", clue("The Matrix"), result()), false);
  const different = harness({ over: { movieAssignments: { "/a": { title: "Something else", recordId: "" } } } });
  assert.equal(different.match.autoAssignMovieMatch("/a", clue("The Matrix"), result()), false);
  assert.equal(different.host.movieAssignments["/a"].title, "Something else");
  assert.equal(h.match.autoAssignMovieMatch("/b", clue("X", "2005"), result({ year: "n/a" })), true);
  assert.equal(h.host.movieAssignments["/b"].year, "2005");
});

test("autoAssignSeriesMatch does the same for a series root and carries its seasons, episodes and posters", () => {
  const h = harness();
  assert.equal(h.match.autoAssignSeriesMatch("/lib/Show", clue("Breaking Bad", "2008"), result({ provider: "tvdb", id: "81189", kind: "tv", title: "Breaking Bad", year: "2008-01-20", rating: null })), true);
  assert.deepEqual(h.host.seriesAssignments["/lib/Show"], {
    title: "Breaking Bad", year: "2008", certification: "", recordId: "tvdb:81189", shortDescription: "A hacker.", plot: "A hacker.", genresText: "", ratingText: "",
    sourceUrl: "https://tmdb/603", artworkCandidates: [art("p1")], posterId: "p1", localPosterPath: "", localSeasonPosterPaths: {}, seasonRecords: {}, episodes: [], episodeAssignments: {},
  });
  const previous = { title: "Breaking Bad", recordId: "", localSeasonPosterPaths: { 1: "/s1.jpg" }, seasonRecords: { 1: { title: "One" } }, episodes: [{ season: 1 }], episodeAssignments: { "/e": { title: "x" } } };
  const kept = harness({ over: { seriesAssignments: { "/lib/Show": previous } } });
  assert.equal(kept.match.autoAssignSeriesMatch("/lib/Show", clue("Breaking Bad"), result({ kind: "tv" })), true);
  const next = kept.host.seriesAssignments["/lib/Show"];
  assert.deepEqual([next.localSeasonPosterPaths, next.seasonRecords, next.episodes.length, Object.keys(next.episodeAssignments)], [{ 1: "/s1.jpg" }, { 1: { title: "One" } }, 1, ["/e"]]);
  const reviewed = harness({ over: { seriesAssignments: { "/lib/Show": { title: "Breaking Bad", recordId: "tvdb:1" } } } });
  assert.equal(reviewed.match.autoAssignSeriesMatch("/lib/Show", clue("Breaking Bad"), result({ kind: "tv" })), false);
  const other = harness({ over: { seriesAssignments: { "/lib/Show": { title: "Other", recordId: "" } } } });
  assert.equal(other.match.autoAssignSeriesMatch("/lib/Show", clue("Breaking Bad"), result({ kind: "tv" })), false);
});

test("seedSeriesAssignment creates a series record from a search row once", () => {
  const h = harness();
  h.match.seedSeriesAssignment("/lib/Show", result({ provider: "tvmaze", id: "169", kind: "tv", title: "Breaking Bad", year: null, rating: 9.3, description: "Teacher.", artwork: [art("a"), art("b")] }));
  assert.deepEqual(h.host.seriesAssignments["/lib/Show"], {
    title: "Breaking Bad", year: "", certification: "", recordId: "tvmaze:169", shortDescription: "Teacher.", plot: "Teacher.", genresText: "", ratingText: "9.3", sourceUrl: "https://tmdb/603",
    artworkCandidates: [art("a"), art("b")], posterId: "a", localPosterPath: "", localSeasonPosterPaths: {}, seasonRecords: {}, episodes: [], episodeAssignments: {},
  });
  const before = h.host.seriesAssignments["/lib/Show"];
  h.match.seedSeriesAssignment("/lib/Show", result({ title: "Replaced" }));
  assert.equal(h.host.seriesAssignments["/lib/Show"], before);
  const bare = harness();
  bare.match.seedSeriesAssignment("/s", { provider: "tmdb", id: "1", title: "T", year: "2001", description: null, rating: undefined, sourceUrl: "u" });
  assert.deepEqual([bare.host.seriesAssignments["/s"].ratingText, bare.host.seriesAssignments["/s"].artworkCandidates, bare.host.seriesAssignments["/s"].posterId, bare.host.seriesAssignments["/s"].shortDescription], ["", [], "", ""]);
});

test("seedMovieDrafts adds filename drafts for films without an assignment, after checking nearby artwork", async () => {
  const files = [file("/lib/A.2001.mkv"), file("/lib/B.mkv"), file("/lib/C.mkv")];
  const scan = { root: "/lib", files, seasonDirectories: [{ path: "/lib/S" }] };
  const h = harness({ over: { organizerScan: scan, movieAssignments: { "/lib/B.mkv": { title: "reviewed" } } }, clues: [clue("A", "2001", "PG"), clue("B"), null] });
  await h.match.seedMovieDrafts(files);
  assert.deepEqual(h.calls[0], { command: "ensureLocalAssets", args: ["/lib", files, ["/lib", "/lib/S"]] });
  assert.deepEqual(h.calls[1], { command: "metadata_filename_clues", args: { filenames: ["A.2001.mkv", "B.mkv", "C.mkv"], patterns: ["rip*"] } });
  assert.deepEqual(h.host.movieAssignments["/lib/A.2001.mkv"], { title: "A", year: "2001", certification: "PG", recordId: "", shortDescription: "", plot: "", genresText: "", ratingText: "", sourceUrl: "", artworkCandidates: [], posterId: "", localPosterPath: "" });
  assert.deepEqual([h.host.movieAssignments["/lib/B.mkv"], "/lib/C.mkv" in h.host.movieAssignments], [{ title: "reviewed" }, false]);
});

test("seedMovieDrafts stops quietly or explains why when it cannot prepare drafts", async () => {
  const files = [file("/lib/A.mkv")];
  const scan = { root: "/lib", files, seasonDirectories: [] };
  const preview = harness({ desktop: false, over: { organizerScan: scan } });
  await preview.match.seedMovieDrafts(files);
  const empty = harness({ over: { organizerScan: scan } });
  await empty.match.seedMovieDrafts([]);
  const noScan = harness();
  await noScan.match.seedMovieDrafts(files);
  assert.deepEqual([preview.calls.length, empty.calls.length, noScan.calls.length, noScan.host.lookupMessage], [0, 0, 0, ""]);
  const silent = harness({ over: { organizerScan: scan }, ensure: async () => null });
  await silent.match.seedMovieDrafts(files);
  assert.deepEqual([silent.host.lookupMessage, silent.calls.map((call) => call.command)], ["", ["ensureLocalAssets"]]);
  const blocked = harness({ over: { organizerScan: scan, localAssetScanError: "denied" }, ensure: async () => null });
  await blocked.match.seedMovieDrafts(files);
  assert.deepEqual([blocked.host.lookupMessage, blocked.host.lookupMessageScope], ["Could not inspect local artwork files: denied", "analyzed films"]);
  const failed = harness({ over: { organizerScan: scan }, clues: new Error("engine") });
  await failed.match.seedMovieDrafts(files);
  assert.deepEqual([failed.host.lookupMessage, failed.host.lookupMessageScope], ["Could not prepare filename-based metadata drafts: Error: engine", "analyzed films"]);
});

test("seriesGroupForHydrate finds the analyzed series by normalized root", () => {
  assert.equal(harness().match.seriesGroupForHydrate("/lib/Show"), undefined);
  const h = harness({ over: { scanResult: showScan() } });
  assert.equal(h.match.seriesGroupForHydrate("/lib//Show").files.length, 4);
  assert.equal(h.match.seriesGroupForHydrate("/lib/Other"), undefined);
});

test("applyCatalogToFiles fills episode text from the provider catalog for scanned files and saved paths", () => {
  const catalog = [{ season: 1, episode: 1, title: "Pilot (provider)", summary: "Chemistry.", airdate: "2008-01-20", sourceUrl: "https://tvdb/e1" }, { season: 1, episode: 4, title: "Fourth", summary: "x", airdate: null, sourceUrl: "" }];
  const h = harness({ over: { scanResult: showScan() } });
  const existing = { title: "Show", plot: "P", episodeAssignments: { "/lib/Show/Season 1/S01E03.Third.mkv": { season: 1, episode: 3, title: "Mine", airdate: null, summary: "kept", sourceUrl: "" }, "/old/S01E04.Fourth.mkv": { season: 1, episode: 4, title: "Show", airdate: null, summary: "", sourceUrl: "" } } };
  const out = h.match.applyCatalogToFiles("/lib/Show", existing, catalog);
  const base = { season: 1, episode: 1, title: "Pilot", airdate: null, summary: "", sourceUrl: "" };
  assert.deepEqual(out["/lib/Show/Season 1/S01E01.Pilot.mkv"], fillEpisodeFromCatalog(base, catalog[0], "Show", "P", episodeIdentity("S01E01.Pilot.mkv").title));
  assert.equal("/lib/Show/Season 1/S01E02.Second.mkv" in out, false);
  assert.deepEqual(out["/lib/Show/Season 1/S01E03.Third.mkv"], existing.episodeAssignments["/lib/Show/Season 1/S01E03.Third.mkv"]);
  assert.deepEqual(out["/old/S01E04.Fourth.mkv"], fillEpisodeFromCatalog(existing.episodeAssignments["/old/S01E04.Fourth.mkv"], catalog[1], "Show", "P", episodeIdentity("S01E04.Fourth.mkv").title));
  assert.equal(out["/old/S01E04.Fourth.mkv"].title, "Fourth");
  assert.deepEqual(Object.keys(existing.episodeAssignments).length, 2);
  const none = harness();
  assert.deepEqual(none.match.applyCatalogToFiles("/lib/Show", { title: "Show", plot: "", episodeAssignments: { "/x": { season: 1, episode: 1, title: "t", summary: "" } } }, []), { "/x": { season: 1, episode: 1, title: "t", summary: "" } });
});

test("applyCatalogToFiles skips multi-episode files, zero episodes and files with no catalog entry or saved text", () => {
  const scan = { root: "/lib", input_kind: "folder", entries: [entry("Show/S01E01E02.Double.mkv"), entry("Show/S01E00.Special.mkv"), entry("Show/S01E05.Five.mkv")] };
  const h = harness({ over: { scanResult: scan } });
  const out = h.match.applyCatalogToFiles("/lib/Show", { title: "Show", plot: "", episodeAssignments: {} }, [{ season: 1, episode: 0, title: "Zero" }, { season: 1, episode: 1, title: "One" }]);
  assert.deepEqual(out, {});
});

test("applySeasonPublication builds a record for every provider season and every numbered season folder", () => {
  const providerSeasons = [{ number: 1, title: "Season 1", overview: "o1", rating: 7.5 }, { number: 0, title: "Specials", overview: "sp", rating: null }, { number: null, title: "ignored" }, { number: -1, title: "ignored" }];
  const h = harness({ over: { scanResult: showScan() } });
  const existing = { genresText: "Drama", seasonRecords: { 1: { title: "My name", year: "2008", certification: "", plot: "", genresText: "Own", ratingText: "" } } };
  const out = h.match.applySeasonPublication("/lib/Show", existing, providerSeasons);
  assert.deepEqual(Object.keys(out).sort(), ["0", "1", "2"]);
  assert.deepEqual(out["1"], fillSeasonFromProvider(existing.seasonRecords[1], { number: 1, title: "Season 1", overview: "o1", rating: 7.5 }, "Drama"));
  assert.deepEqual(out["0"], fillSeasonFromProvider(undefined, { number: 0, title: "Specials", overview: "sp", rating: null }, "Drama"));
  assert.deepEqual(out["2"], fillSeasonFromProvider(undefined, { number: 2, title: "", overview: "", rating: null }, "Drama"));
  assert.equal(out["1"].title, "My name");
  // A Specials folder alone does not create a season record; only numbered seasons do.
  const specials = harness({ over: { scanResult: { root: "/lib", input_kind: "folder", entries: [entry("Show/Specials/S00E01.Extra.mkv"), entry("Show/Season 1/S01E01.Pilot.mkv")] } } });
  assert.deepEqual(Object.keys(specials.match.applySeasonPublication("/lib/Show", { genresText: "", seasonRecords: {} }, [])), ["1"]);
  const unknown = harness({ over: { scanResult: showScan() } }).match.applySeasonPublication("/lib/Unknown", { genresText: "", seasonRecords: {} }, []);
  assert.deepEqual(unknown, {});
});
