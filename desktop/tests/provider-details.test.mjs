// ./desktop/tests/provider-details.test.mjs
// Characterization tests for the provider detail loading extracted from MetadataWorkbench (T10.7 slice 7): opening a match
// (one branch per provider and kind), loading a series' catalog into its assignment, applying TMDB film details to a draft,
// and the review shortcuts that open a candidate for a film, a series or the open search target. Provider payloads are fakes.
// Run: pnpm test:provider-details

import assert from "node:assert/strict";
import test from "node:test";
import { createProviderDetails } from "../src/lib/provider-details.ts";

const art = (id, url = `https://img/${id}.jpg`) => ({ id, title: `Poster ${id}`, imageUrl: url, thumbnailUrl: "", sourceUrl: "https://src/" + id });
const row = (over = {}) => ({ id: "603", provider: "tmdb", kind: "movie", title: "The Matrix", year: "1999", description: "Row text", rating: 7, sourceUrl: "https://row", confidence: 1, artwork: [art("row")], ...over });

function harness({ payloads = {}, over = {}, language = "fr-FR", tmdbConfigured = true } = {}) {
  const calls = [];
  const host = {
    metadataLanguage: language, tmdbCredentialStatus: { configured: tmdbConfigured }, seriesAssignments: {}, movieAssignments: {}, selectedMediaPaths: ["/keep"],
    provider: "wikidata", mediaKind: "movie", selected: null, detail: null, loadingDetail: false, error: "stale", reviewDetailError: "old", searchTarget: null,
    seriesYearConflictConfirmed: true, selectedArtwork: { x: "y" }, organizerScan: null, renamePreview: { previewId: "p" }, confirmRename: true,
    pendingArtworkSave: { label: "x" }, artworkError: "e", artworkMessage: "m", lookupMessage: "", lookupMessageScope: null, ...over,
  };
  const invoke = async (command, args) => {
    calls.push({ command, args });
    const value = payloads[command];
    if (value instanceof Error) throw value;
    return typeof value === "function" ? value(args) : value;
  };
  const actions = {
    syncEpisodeAssignments: (files) => calls.push({ command: "sync", args: files }),
    assignCurrentMovie: (paths) => calls.push({ command: "assignCurrentMovie", args: paths }),
    chooseAnalyzedSeries: (root) => calls.push({ command: "chooseAnalyzedSeries", args: root }),
    applyCatalogToFiles: (root, existing, catalog) => { calls.push({ command: "applyCatalog", args: [root, catalog.length] }); return { "/e1": { title: "assigned" } }; },
    applySeasonPublication: (root, existing, seasons) => { calls.push({ command: "applySeasons", args: [root, seasons.length] }); return { 1: { title: "season record" } }; },
  };
  return { details: createProviderDetails({ invoke, host, actions }), host, calls };
}
const commands = (h) => h.calls.map((call) => call.command);

test("openMatch resets the dependent state, loads the detail and clears the artwork and rename choices", async () => {
  const value = { movie: { title: "The Matrix", originalTitle: "Matrix", year: "1999", sourceUrl: "https://tmdb/603", overview: "Hacker.", rating: 8.2 }, tagline: "Free your mind", certification: "R", runtimeMinutes: 136, genres: ["Sci-Fi"], artwork: [art("a")] };
  const h = harness({ payloads: { load_tmdb_movie_details: value } });
  const pending = h.details.openMatch(row());
  assert.deepEqual([h.host.provider, h.host.mediaKind, h.host.selected.id, h.host.detail, h.host.loadingDetail, h.host.error, h.host.seriesYearConflictConfirmed, h.host.confirmRename, h.host.pendingArtworkSave, h.host.artworkError, h.host.artworkMessage], ["tmdb", "movie", "603", null, true, "", false, false, null, "", ""]);
  await pending;
  assert.deepEqual(h.calls, [{ command: "load_tmdb_movie_details", args: { id: 603, language: "fr-FR" } }]);
  assert.deepEqual(h.host.detail, { title: "The Matrix", originalTitle: "Matrix", year: "1999", sourceUrl: "https://tmdb/603", shortDescription: "Free your mind", longDescription: "Hacker.", certification: "R", runtime: 136, genres: ["Sci-Fi"], rating: 8.2, artwork: [art("a")], seasons: [], episodes: [], attribution: "This product uses the TMDB API but is not endorsed or certified by TMDB." });
  assert.deepEqual([h.host.loadingDetail, h.host.selectedArtwork, h.host.pendingArtworkSave, h.host.renamePreview], [false, {}, null, null]);
});

test("openMatch falls back to the row's own values when the movie payload is sparse", async () => {
  const h = harness({ payloads: { load_tmdb_movie_details: { movie: {} } } });
  await h.details.openMatch(row({ year: "1999", rating: 7 }));
  const detail = h.host.detail;
  assert.deepEqual([detail.title, detail.originalTitle, detail.year, detail.sourceUrl, detail.shortDescription, detail.longDescription, detail.certification, detail.runtime, detail.genres, detail.rating, detail.artwork], ["The Matrix", null, "1999", "https://row", "", "", null, null, [], 7, []]);
});

test("openMatch reads a TMDB series with its seasons and episodes, and syncs episodes for a scanned folder", async () => {
  const value = { show: { title: "Show", originalTitle: "Orig", year: "2008", sourceUrl: "https://tmdb/tv", rating: 8.9, artwork: [art("s")] }, overview: "Teacher.", certification: "TV-MA", genres: ["Drama"], seasons: [{ number: 1 }], episodes: [{ season: 1, episode: 1 }] };
  const scan = { root: "/lib/Show", files: [{ path: "/lib/Show/e1.mkv" }] };
  const h = harness({ payloads: { load_tmdb_tv_details: value }, over: { organizerScan: scan } });
  await h.details.openMatch(row({ provider: "tmdb", kind: "tv", id: "1396", title: "Show" }));
  assert.deepEqual(h.calls[0], { command: "load_tmdb_tv_details", args: { id: 1396, language: "fr-FR" } });
  assert.deepEqual([h.host.detail.title, h.host.detail.longDescription, h.host.detail.certification, h.host.detail.runtime, h.host.detail.seasons, h.host.detail.episodes.length, h.host.detail.shortDescription], ["Show", "Teacher.", "TV-MA", null, [{ number: 1 }], 1, ""]);
  assert.deepEqual(h.calls[1], { command: "sync", args: scan.files });
  const movie = harness({ payloads: { load_tmdb_movie_details: { movie: {} } }, over: { organizerScan: scan } });
  await movie.details.openMatch(row());
  const noScan = harness({ payloads: { load_tmdb_tv_details: { show: {} } } });
  await noScan.details.openMatch(row({ provider: "tmdb", kind: "tv" }));
  assert.deepEqual([commands(movie).includes("sync"), commands(noScan).includes("sync")], [false, false]);
});

test("openMatch reads TheTVDB movies and series and Wikidata films", async () => {
  const tvdb = harness({ payloads: { load_tvdb_details: { title: "Breaking Bad", originalTitle: "BB", year: "2008", sourceUrl: "https://tvdb", shortDescription: "Short", longDescription: "Long", certification: "TV-MA", runtimeMinutes: 47, genres: ["Crime"], rating: 9.5, artwork: [art("t")], seasons: [{ number: 1 }], episodes: [], attribution: "TVDB credit", episodeOrderLabel: "Aired order" } } });
  await tvdb.details.openMatch(row({ provider: "tvdb", kind: "tv", id: "81189" }));
  assert.deepEqual(tvdb.calls[0], { command: "load_tvdb_details", args: { id: 81189, kind: "series", language: "fr-FR" } });
  assert.deepEqual([tvdb.host.detail.title, tvdb.host.detail.runtime, tvdb.host.detail.rating, tvdb.host.detail.attribution, tvdb.host.detail.episodeOrderLabel, tvdb.host.detail.longDescription], ["Breaking Bad", 47, 9.5, "TVDB credit", "Aired order", "Long"]);
  const movie = harness({ payloads: { load_tvdb_details: {} } });
  await movie.details.openMatch(row({ provider: "tvdb", kind: "movie", id: "5", description: "Row desc" }));
  assert.equal(movie.calls[0].args.kind, "movie");
  assert.deepEqual([movie.host.detail.attribution, movie.host.detail.longDescription, movie.host.detail.rating, movie.host.detail.episodeOrderLabel], ["TheTVDB metadata. Review the source access and reuse terms.", "Row desc", null, undefined]);
  const wiki = harness({ payloads: { load_public_film_details: { title: "The Matrix", year: "1999", sourceUrl: "https://wd", shortDescription: "S", longDescription: "L", certification: "R", artwork: [art("w")] } } });
  await wiki.details.openMatch(row({ provider: "wikidata", id: "Q83495" }));
  assert.deepEqual(wiki.calls[0], { command: "load_public_film_details", args: { entityId: "Q83495", language: "fr-FR" } });
  assert.deepEqual([wiki.host.detail.originalTitle, wiki.host.detail.runtime, wiki.host.detail.genres, wiki.host.detail.attribution], [null, null, [], "Metadata from Wikidata; the long description may come from the selected language’s Wikipedia."]);
});

test("openMatch reads a TVmaze series with its year range and language note, and reports a failure", async () => {
  const maze = harness({ payloads: { load_public_tv_details: { show: { title: "Breaking Bad", yearStart: "2008", yearEnd: "2013", sourceUrl: "https://maze", summary: "Teacher.", audienceRating: 9.3, artwork: [art("m")] }, seasons: [{ number: 1 }], episodes: [{ season: 1, episode: 1 }] } } });
  await maze.details.openMatch(row({ provider: "tvmaze", kind: "tv", id: "169" }));
  assert.deepEqual(maze.calls[0], { command: "load_public_tv_details", args: { showId: 169 } });
  assert.deepEqual([maze.host.detail.year, maze.host.detail.longDescription, maze.host.detail.rating, maze.host.detail.certification], ["2008–2013", "Teacher.", 9.3, null]);
  assert.match(maze.host.detail.attribution, /^TVmaze program data, as published by TVmaze\. Requested metadata language: fr-FR;/);
  const failing = harness({ payloads: { load_tmdb_movie_details: new Error("401") } });
  await failing.details.openMatch(row());
  assert.deepEqual([failing.host.error, failing.host.detail, failing.host.loadingDetail, failing.host.selectedArtwork], ["Error: 401", null, false, {}]);
});

test("applyTmdbMovieDetails merges TMDB details into an existing film draft only", async () => {
  const draft = { title: "The Matrix", certification: " ", shortDescription: "mine", plot: "my plot", genresText: "Own", ratingText: "5.0", sourceUrl: "https://own", artworkCandidates: [art("old")], posterId: "old" };
  const value = { movie: { title: "The Matrix", rating: 8.234, overview: "Hacker.", sourceUrl: "https://tmdb/603", artwork: [art("fallback")] }, tagline: "  Free your mind ", certification: "R", genres: ["Sci-Fi", 7, "Action"], artwork: [art("a"), art("a"), art("b")] };
  const h = harness({ payloads: { load_tmdb_movie_details: value }, over: { movieAssignments: { "/a": draft, "/other": { title: "keep" } } } });
  await h.details.applyTmdbMovieDetails("/a", "603");
  assert.deepEqual(h.calls[0], { command: "load_tmdb_movie_details", args: { id: 603, language: "fr-FR" } });
  const next = h.host.movieAssignments["/a"];
  assert.deepEqual([next.certification, next.shortDescription, next.plot, next.genresText, next.ratingText, next.sourceUrl, next.artworkCandidates.map((image) => image.id), next.posterId], ["R", "Free your mind", "Hacker.", "Sci-Fi, Action", "8.2", "https://tmdb/603", ["a", "b"], "a"]);
  assert.deepEqual(h.host.movieAssignments["/other"], { title: "keep" });
  assert.equal(h.host.reviewDetailError, "");
  const keep = harness({ payloads: { load_tmdb_movie_details: { movie: {} } }, over: { movieAssignments: { "/a": { ...draft, certification: "PG", posterId: "old" } } } });
  await keep.details.applyTmdbMovieDetails("/a", "603");
  const same = keep.host.movieAssignments["/a"];
  assert.deepEqual([same.certification, same.shortDescription, same.plot, same.genresText, same.ratingText, same.sourceUrl, same.artworkCandidates.length, same.posterId], ["PG", "mine", "my plot", "Own", "5.0", "https://own", 1, "old"]);
  const retained = harness({ payloads: { load_tmdb_movie_details: { movie: {}, artwork: [art("z"), art("old")] } }, over: { movieAssignments: { "/a": draft } } });
  await retained.details.applyTmdbMovieDetails("/a", "603");
  assert.equal(retained.host.movieAssignments["/a"].posterId, "old");
  const none = harness({ payloads: { load_tmdb_movie_details: { movie: { title: "x" } } } });
  await none.details.applyTmdbMovieDetails("/missing", "1");
  assert.deepEqual(none.host.movieAssignments, {});
});

const seriesValue = (over = {}) => ({
  episodes: [{ season: 1, episode: 1, title: "Pilot", airdate: "2008-01-20", summary: "S", sourceUrl: "https://e1", artwork: [{ imageUrl: "https://still/1.jpg" }] }, { season: 1, episode: 0, title: "Zero" }, { season: 1.5, episode: 2, title: "Bad" }, { season: 1, episode: 2.5, title: "Bad too" }, { season: 2, episode: 1, title: "Back" }],
  seasons: [{ number: 1, overview: "o", artwork: [{ imageUrl: "https://poster/s1.jpg" }] }, { number: 2, overview: "", artwork: [{ imageUrl: "/local/s2.jpg" }] }, { number: -1, artwork: [{ imageUrl: "https://poster/neg.jpg" }] }, { number: null }],
  artwork: [art("tv")], sourceUrl: "https://tvdb/show", certification: "  TV-MA ", ...over,
});

test("hydrateSeriesProvider loads a TheTVDB series into its assignment, with a TMDB certification fallback", async () => {
  const h = harness({ payloads: { load_tvdb_details: seriesValue({ certification: "" }), tmdb_tv_certification_for_tvdb: " TV-14 " } });
  const ok = await h.details.hydrateSeriesProvider("/lib/Show", row({ provider: "tvdb", kind: "tv", id: "81189", title: "Show", artwork: [] }));
  assert.equal(ok, true);
  assert.deepEqual(h.calls.slice(0, 2), [{ command: "load_tvdb_details", args: { id: 81189, kind: "series", language: "fr-FR" } }, { command: "tmdb_tv_certification_for_tvdb", args: { tvdbId: 81189, language: "fr-FR" } }]);
  const saved = h.host.seriesAssignments["/lib/Show"];
  assert.deepEqual([saved.certification, saved.recordId, saved.sourceUrl, saved.title, saved.posterId, saved.artworkCandidates.map((image) => image.id)], ["TV-14", "tvdb:81189", "https://row", "Show", "tv", ["tv"]]);
  assert.deepEqual(saved.episodes.map((episode) => [episode.season, episode.episode, episode.title]), [[1, 1, "Pilot"], [2, 1, "Back"]]);
  assert.deepEqual([saved.episodeAssignments, saved.seasonRecords], [{ "/e1": { title: "assigned" } }, { 1: { title: "season record" } }]);
  assert.deepEqual(h.calls.filter((call) => call.command === "applyCatalog" || call.command === "applySeasons").map((call) => call.args), [["/lib/Show", 2], ["/lib/Show", 4]]);
  const unconfigured = harness({ tmdbConfigured: false, payloads: { load_tvdb_details: seriesValue({ certification: "" }) } });
  await unconfigured.details.hydrateSeriesProvider("/lib/Show", row({ provider: "tvdb", kind: "tv", id: "1" }));
  assert.deepEqual([commands(unconfigured).includes("tmdb_tv_certification_for_tvdb"), unconfigured.host.seriesAssignments["/lib/Show"].certification], [false, ""]);
  const direct = harness({ payloads: { load_tvdb_details: seriesValue() } });
  await direct.details.hydrateSeriesProvider("/lib/Show", row({ provider: "tvdb", kind: "tv", id: "1" }));
  assert.deepEqual([commands(direct).includes("tmdb_tv_certification_for_tvdb"), direct.host.seriesAssignments["/lib/Show"].certification], [false, "TV-MA"]);
});

test("hydrateSeriesProvider reads TMDB and TVmaze series shows and ignores other providers", async () => {
  const tmdb = harness({ payloads: { load_tmdb_tv_details: { show: { artwork: [art("t")], sourceUrl: "https://tmdb/tv" }, episodes: [{ season: 1, episode: 1, title: "x" }], seasons: [], certification: "TV-PG" } } });
  assert.equal(await tmdb.details.hydrateSeriesProvider("/s", row({ provider: "tmdb", kind: "tv", id: "9" })), true);
  assert.deepEqual(tmdb.calls[0], { command: "load_tmdb_tv_details", args: { id: 9, language: "fr-FR" } });
  assert.deepEqual([tmdb.host.seriesAssignments["/s"].certification, tmdb.host.seriesAssignments["/s"].recordId], ["TV-PG", "tmdb:9"]);
  const maze = harness({ payloads: { load_public_tv_details: { show: { artwork: [art("m")], sourceUrl: "https://maze" }, episodes: [], seasons: [{ number: 1, rating: 8 }] } } });
  assert.equal(await maze.details.hydrateSeriesProvider("/s", row({ provider: "tvmaze", kind: "tv", id: "3" })), true);
  assert.deepEqual(maze.calls[0], { command: "load_public_tv_details", args: { showId: 3 } });
  for (const provider of ["wikidata", "local"]) {
    const other = harness();
    assert.equal(await other.details.hydrateSeriesProvider("/s", row({ provider })), false);
    assert.deepEqual([other.calls.length, other.host.seriesAssignments], [0, {}]);
  }
});

test("hydrateSeriesProvider keeps at most 10,000 catalog episodes", async () => {
  const episodes = Array.from({ length: 10_001 }, (_, index) => ({ season: 1, episode: index + 1, title: `E${index + 1}` }));
  const h = harness({ payloads: { load_tmdb_tv_details: { show: {}, episodes, seasons: [] } } });
  await h.details.hydrateSeriesProvider("/s", row({ provider: "tmdb", kind: "tv" }));
  const saved = h.host.seriesAssignments["/s"].episodes;
  assert.deepEqual([saved.length, saved.at(-1).episode], [10_000, 10_000]);
});

test("hydrateSeriesProvider reports whether it found episodes or season text", async () => {
  const empty = harness({ payloads: { load_tmdb_tv_details: { show: {}, episodes: [], seasons: [{ number: 1, overview: "  ", rating: 0 }] } } });
  assert.equal(await empty.details.hydrateSeriesProvider("/s", row({ provider: "tmdb", kind: "tv" })), false);
  const overview = harness({ payloads: { load_tmdb_tv_details: { show: {}, episodes: [], seasons: [{ number: 1, overview: "Plot" }] } } });
  assert.equal(await overview.details.hydrateSeriesProvider("/s", row({ provider: "tmdb", kind: "tv" })), true);
  const rating = harness({ payloads: { load_tmdb_tv_details: { show: {}, episodes: [], seasons: [{ number: 1, rating: 7.5 }] } } });
  assert.equal(await rating.details.hydrateSeriesProvider("/s", row({ provider: "tmdb", kind: "tv" })), true);
  const failing = harness({ payloads: { load_tmdb_tv_details: new Error("429") } });
  assert.equal(await failing.details.hydrateSeriesProvider("/s", row({ provider: "tmdb", kind: "tv" })), false);
  assert.deepEqual([failing.host.lookupMessage, failing.host.lookupMessageScope, failing.host.seriesAssignments], ["Could not load episode details from tmdb: Error: 429", "series folders", {}]);
});

test("hydrateSeriesProvider keeps what the user already chose and only fills what is empty", async () => {
  const existing = {
    title: "Mine", year: "2000", certification: " PG ", recordId: "tvdb:77", shortDescription: "", plot: "", genresText: "", ratingText: "", sourceUrl: "https://own", artworkCandidates: [art("own")], posterId: "own", localPosterPath: "",
    localSeasonPosterPaths: { 1: "/local/s1.jpg", 2: "https://chosen/s2.jpg", 3: "https://still/1.jpg" }, seasonRecords: {}, episodes: [], episodeAssignments: {},
  };
  const value = seriesValue({ certification: "TV-MA", sourceUrl: "https://tvdb/show", seasons: [
    { number: 1, artwork: [{ imageUrl: "https://poster/s1.jpg" }] }, { number: 2, artwork: [{ imageUrl: "https://poster/s2.jpg" }] }, { number: 3, artwork: [{ imageUrl: "https://poster/s3.jpg" }] }, { number: 4, artwork: [{ imageUrl: "https://poster/s4.jpg" }] }, { number: 5, artwork: [{ imageUrl: "/not-https.jpg" }] }, { number: -1, artwork: [{ imageUrl: "https://poster/neg.jpg" }] }, { number: 0, artwork: [{ imageUrl: "https://poster/specials.jpg" }] },
  ] });
  const h = harness({ payloads: { load_tvdb_details: value }, over: { seriesAssignments: { "/lib/Show": existing } } });
  await h.details.hydrateSeriesProvider("/lib/Show", row({ provider: "tvdb", kind: "tv", id: "77" }));
  const saved = h.host.seriesAssignments["/lib/Show"];
  assert.deepEqual(saved.localSeasonPosterPaths, { 0: "https://poster/specials.jpg", 1: "/local/s1.jpg", 2: "https://chosen/s2.jpg", 3: "https://poster/s3.jpg", 4: "https://poster/s4.jpg" });
  assert.deepEqual([saved.title, saved.certification, saved.sourceUrl, saved.posterId, saved.recordId], ["Mine", "PG", "https://own", "own", "tvdb:77"]);
  assert.deepEqual(saved.artworkCandidates.map((image) => image.id), ["tv"]);
  const local = harness({ payloads: { load_tvdb_details: seriesValue({ artwork: [] }) }, over: { seriesAssignments: { "/lib/Show": { ...existing, recordId: "local-1", sourceUrl: " ", certification: "" } } } });
  await local.details.hydrateSeriesProvider("/lib/Show", row({ provider: "tvdb", kind: "tv", id: "77" }));
  const next = local.host.seriesAssignments["/lib/Show"];
  assert.deepEqual([next.recordId, next.sourceUrl, next.certification, next.artworkCandidates.map((image) => image.id)], ["tvdb:77", "https://tvdb/show", "TV-MA", ["own"]]);
  const blank = harness({ payloads: { load_tvdb_details: seriesValue() }, over: { seriesAssignments: { "/lib/Show": { ...existing, recordId: "" } } } });
  await blank.details.hydrateSeriesProvider("/lib/Show", row({ provider: "tvdb", kind: "tv", id: "9" }));
  assert.equal(blank.host.seriesAssignments["/lib/Show"].recordId, "tvdb:9");
});

test("reviewMovieCandidate selects the film, opens the match and assigns it once detail loaded", async () => {
  const value = { movie: { title: "The Matrix" } };
  const h = harness({ payloads: { load_tmdb_movie_details: value } });
  await h.details.reviewMovieCandidate("/lib/a.mkv", row());
  assert.deepEqual(h.host.selectedMediaPaths, ["/keep", "/lib/a.mkv"]);
  assert.deepEqual(commands(h), ["load_tmdb_movie_details", "assignCurrentMovie"]);
  assert.deepEqual(h.calls[1].args, ["/lib/a.mkv"]);
  const skip = harness({ payloads: { load_tmdb_movie_details: value } });
  await skip.details.reviewMovieCandidate("/keep", row(), false);
  assert.deepEqual([skip.host.selectedMediaPaths, commands(skip)], [["/keep"], ["load_tmdb_movie_details"]]);
  const failing = harness({ payloads: { load_tmdb_movie_details: new Error("x") } });
  await failing.details.reviewMovieCandidate("/lib/a.mkv", row());
  assert.equal(commands(failing).includes("assignCurrentMovie"), false);
});

test("reviewSeriesCandidate picks the analyzed series before opening the match, and the target shortcut routes by kind", async () => {
  const value = { show: {}, episodes: [], seasons: [] };
  const h = harness({ payloads: { load_tmdb_tv_details: value } });
  await h.details.reviewSeriesCandidate("/lib/Show", row({ provider: "tmdb", kind: "tv" }));
  assert.deepEqual(commands(h), ["chooseAnalyzedSeries", "load_tmdb_tv_details"]);
  assert.equal(h.calls[0].args, "/lib/Show");
  const none = harness();
  await none.details.reviewTargetCandidate(row());
  assert.equal(none.calls.length, 0);
  const movie = harness({ payloads: { load_tmdb_movie_details: { movie: {} } }, over: { searchTarget: { kind: "movie", path: "/lib/Film.mkv", label: "Film" } } });
  await movie.details.reviewTargetCandidate(row());
  assert.deepEqual([movie.host.selectedMediaPaths, commands(movie)], [["/keep", "/lib/Film.mkv"], ["load_tmdb_movie_details", "assignCurrentMovie"]]);
  const tv = harness({ payloads: { load_tmdb_tv_details: value }, over: { searchTarget: { kind: "tv", root: "/lib/Show", label: "Show" } } });
  await tv.details.reviewTargetCandidate(row({ provider: "tmdb", kind: "tv" }));
  assert.deepEqual(commands(tv), ["chooseAnalyzedSeries", "load_tmdb_tv_details"]);
});
