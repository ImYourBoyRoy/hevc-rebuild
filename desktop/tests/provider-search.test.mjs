// ./desktop/tests/provider-search.test.mjs
// Characterization tests for the provider search extracted from MetadataWorkbench (T10.7 slice 3): which sources are
// asked for a movie or a series, how each provider's rows become results, skipped sources and their warnings, partial
// and total failures, de-duplication and the 45-result cap, plus the filename clue call. The native bridge is a fake.
// Run: pnpm test:provider-search

import assert from "node:assert/strict";
import test from "node:test";
import { createProviderSearch } from "../src/lib/provider-search.ts";

const ready = { configured: true, secureStoreAvailable: true, subscriberPinConfigured: false, message: null };
const missing = { configured: false, secureStoreAvailable: true, subscriberPinConfigured: false, message: null };

function harness({ sources = {}, tmdb = ready, tvdb = ready, rows = {}, desktop = true, language = "fr-FR", patterns = ["rip*"] } = {}) {
  const calls = [];
  const host = { metadataSources: { tmdb: false, wikidata: false, tvmaze: false, tvdb: false, ...sources }, tmdbCredentialStatus: tmdb, tvdbCredentialStatus: tvdb, metadataLanguage: language, textBlacklistPatterns: patterns };
  const invoke = async (command, args) => {
    calls.push({ command, args });
    const value = rows[command];
    if (value instanceof Error) throw value;
    return typeof value === "function" ? value(args) : value ?? [];
  };
  return { search: createProviderSearch({ invoke, isDesktop: () => desktop, host }), calls };
}

test("a ready TMDB source searches movies or series in the chosen language and maps every field", async () => {
  const row = { id: 603, title: "The Matrix", year: "1999", overview: "A hacker.", rating: 8.2, sourceUrl: "https://tmdb/603", confidence: 0.9, artwork: [{ id: "a" }] };
  const movie = harness({ sources: { tmdb: true }, rows: { search_tmdb_movies: [row] } });
  const found = await movie.search.searchPublicTitles("movie", "Matrix");
  assert.deepEqual(movie.calls, [{ command: "search_tmdb_movies", args: { query: "Matrix", language: "fr-FR" } }]);
  assert.deepEqual(found, { results: [{ id: "603", provider: "tmdb", kind: "movie", title: "The Matrix", year: "1999", description: "A hacker.", rating: 8.2, sourceUrl: "https://tmdb/603", confidence: 0.9, artwork: [{ id: "a" }] }], warnings: [], successfulProviders: ["tmdb"] });
  const tv = harness({ sources: { tmdb: true }, rows: { search_tmdb_tv: [{ id: 1 }] } });
  const bare = await tv.search.searchPublicTitles("tv", "Show");
  assert.equal(tv.calls[0].command, "search_tmdb_tv");
  assert.deepEqual(bare.results[0], { id: "1", provider: "tmdb", kind: "tv", title: "Untitled", year: null, description: null, rating: null, sourceUrl: "https://www.themoviedb.org", confidence: 0, artwork: [] });
});

test("Wikidata is a movie-only source and maps entity rows", async () => {
  const wiki = harness({ sources: { wikidata: true }, rows: { search_public_films: [{ entityId: "Q83495", title: "The Matrix", year: "1999", description: "1999 film", confidence: 0.5 }, { entityId: "Q2" }] } });
  const found = await wiki.search.searchPublicTitles("movie", "Matrix");
  assert.deepEqual(wiki.calls, [{ command: "search_public_films", args: { query: "Matrix", language: "fr-FR" } }]);
  assert.deepEqual(found.results[0], { id: "Q83495", provider: "wikidata", kind: "movie", title: "The Matrix", year: "1999", description: "1999 film", rating: null, sourceUrl: "https://www.wikidata.org", confidence: 0.5, artwork: [] });
  assert.deepEqual([found.results[1].title, found.results[1].year, found.results[1].description], ["Untitled", null, null]);
  const tv = harness({ sources: { wikidata: true } });
  await assert.rejects(() => tv.search.searchPublicTitles("tv", "Show"), /No series source is ready\./);
  assert.equal(tv.calls.length, 0);
});

test("TheTVDB asks for movie or series records and is skipped with a warning when not set up", async () => {
  const tvdb = harness({ sources: { tvdb: true }, rows: { search_tvdb_titles: [{ id: 81189, title: "Breaking Bad", year: "2008", overview: "Chemistry.", sourceUrl: "https://tvdb/81189", confidence: 0.7, artwork: [{ id: "t" }] }] } });
  const found = await tvdb.search.searchPublicTitles("tv", "Breaking");
  assert.deepEqual(tvdb.calls, [{ command: "search_tvdb_titles", args: { query: "Breaking", kind: "series", language: "fr-FR" } }]);
  assert.deepEqual(found.results[0], { id: "81189", provider: "tvdb", kind: "tv", title: "Breaking Bad", year: "2008", description: "Chemistry.", rating: null, sourceUrl: "https://tvdb/81189", confidence: 0.7, artwork: [{ id: "t" }] });
  const movie = harness({ sources: { tvdb: true } });
  await movie.search.searchPublicTitles("movie", "Heat");
  assert.equal(movie.calls[0].args.kind, "movie");
  const unconfigured = harness({ sources: { tvdb: true, wikidata: true }, tvdb: missing, rows: { search_public_films: [{ entityId: "Q1" }] } });
  const partial = await unconfigured.search.searchPublicTitles("movie", "Heat");
  assert.deepEqual(partial.warnings, ["TheTVDB was skipped because its project API key is not saved. Set up TheTVDB access in Settings to include its results."]);
  assert.deepEqual(partial.successfulProviders, ["wikidata"]);
});

test("TVmaze is a series-only source with a year range and audience rating", async () => {
  const maze = harness({ sources: { tvmaze: true }, rows: { search_public_tv_shows: [{ id: 169, title: "Breaking Bad", yearStart: "2008", yearEnd: "2013", summary: "Teacher.", audienceRating: 9.3, sourceUrl: "https://tvmaze/169" }, { id: 2, yearStart: "2020" }, { id: 3 }] } });
  const found = await maze.search.searchPublicTitles("tv", "Breaking");
  assert.deepEqual(maze.calls, [{ command: "search_public_tv_shows", args: { query: "Breaking" } }]);
  assert.deepEqual(found.results.map((row) => row.year), ["2008–2013", "2020", null]);
  assert.deepEqual([found.results[0].description, found.results[0].rating, found.results[0].provider], ["Teacher.", 9.3, "tvmaze"]);
  assert.equal(found.results[2].sourceUrl, "https://www.tvmaze.com");
  const movie = harness({ sources: { tvmaze: true } });
  await assert.rejects(() => movie.search.searchPublicTitles("movie", "Heat"), /No movie source is ready\./);
});

test("with no usable source the search explains how to enable one, including skipped sources", async () => {
  const none = harness();
  await assert.rejects(() => none.search.searchPublicTitles("movie", "x"), (error) => error.message === "No movie source is ready. Turn on Wikidata, save a TMDB token, or set up TheTVDB in Settings.");
  await assert.rejects(() => none.search.searchPublicTitles("tv", "x"), (error) => error.message === "No series source is ready. Turn on TVmaze, save a TMDB token, or set up TheTVDB in Settings.");
  const skipped = harness({ sources: { tmdb: true, tvdb: true }, tmdb: missing, tvdb: missing });
  await assert.rejects(() => skipped.search.searchPublicTitles("movie", "x"), (error) => error.message.startsWith("TMDB was skipped because no API Read Access Token is saved.") && error.message.includes("TheTVDB was skipped") && error.message.endsWith("No movie source is ready. Turn on Wikidata, save a TMDB token, or set up TheTVDB in Settings."));
  assert.equal(skipped.calls.length, 0);
});

test("one failing source becomes a warning, and all failing sources raise the combined warnings", async () => {
  const partial = harness({ sources: { tmdb: true, wikidata: true }, rows: { search_tmdb_movies: new Error("401"), search_public_films: [{ entityId: "Q1", title: "Heat" }] } });
  const found = await partial.search.searchPublicTitles("movie", "Heat");
  assert.deepEqual(found.warnings, ["TMDB lookup failed: Error: 401"]);
  assert.deepEqual([found.successfulProviders, found.results.map((row) => row.id)], [["wikidata"], ["Q1"]]);
  const all = harness({ sources: { tmdb: true, wikidata: true }, rows: { search_tmdb_movies: new Error("401"), search_public_films: new Error("timeout") } });
  await assert.rejects(() => all.search.searchPublicTitles("movie", "Heat"), (error) => error.message === "TMDB lookup failed: Error: 401 Wikidata lookup failed: Error: timeout");
});

test("results from several providers keep request order, drop repeats of the same record and stop at 45", async () => {
  const many = Array.from({ length: 50 }, (_, index) => ({ id: index, title: `T${index}` }));
  const h = harness({ sources: { tmdb: true, wikidata: true }, rows: { search_tmdb_movies: many, search_public_films: [{ entityId: "Q1", title: "Wiki" }] } });
  const found = await h.search.searchPublicTitles("movie", "x");
  assert.equal(found.results.length, 45);
  assert.deepEqual(found.results.slice(0, 3).map((row) => row.id), ["0", "1", "2"]);
  assert.equal(found.results.some((row) => row.provider === "wikidata"), false);
  const repeated = harness({ sources: { tmdb: true }, rows: { search_tmdb_movies: [{ id: 5, title: "first" }, { id: 5, title: "second" }] } });
  const unique = await repeated.search.searchPublicTitles("movie", "x");
  assert.deepEqual(unique.results.map((row) => row.title), ["second"]);
  const sameId = harness({ sources: { tmdb: true, tvdb: true }, rows: { search_tmdb_tv: [{ id: 5 }], search_tvdb_titles: [{ id: 5 }] } });
  assert.equal((await sameId.search.searchPublicTitles("tv", "x")).results.length, 2);
});

test("filenameClue asks the engine with the blocked-text patterns, and needs the desktop app", async () => {
  const h = harness({ rows: { metadata_filename_clue: { title: "Heat", year: "1995", yearEnd: null, certification: null } } });
  assert.deepEqual(await h.search.filenameClue("Heat.1995.mkv"), { title: "Heat", year: "1995", yearEnd: null, certification: null });
  assert.deepEqual(h.calls, [{ command: "metadata_filename_clue", args: { filename: "Heat.1995.mkv", patterns: ["rip*"] } }]);
  const preview = harness({ desktop: false });
  await assert.rejects(() => preview.search.filenameClue("x.mkv"), /Filename-assisted matching is available in the installed desktop app\./);
  assert.equal(preview.calls.length, 0);
});
