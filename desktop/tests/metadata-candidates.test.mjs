// ./desktop/tests/metadata-candidates.test.mjs
// Characterization tests for the candidate and artwork helpers extracted from MetadataWorkbench (T10.7 slice 1):
// saved suggestions turned into search results and match sets, the scan fingerprint, artwork de-duplication and
// review filtering, safe URLs, artwork file targets and the provider and episode text checks. Pure functions.
// Run: pnpm test:metadata-candidates

import assert from "node:assert/strict";
import test from "node:test";
import {
  artworkFileExtension, assignedMoviePoster, candidateEvidence, candidatePreview, clueCaption, dedupeArtwork, metadataScanKey, movedPathForSource, movieArtworkTarget,
  normalizeArt, providerEpisodeTextReady, providerLabel, reviewableArtwork, safeHttpUrl, seasonArtworkTargets, seriesArtworkTargets, suggestionDetail, suggestionMatchSet,
  suggestionResult, uniqueArtwork,
} from "../src/lib/metadata-candidates.ts";
import { metadataMatchLabel } from "../src/lib/metadata-match.ts";

const art = (id, over = {}) => ({ id, title: `Poster ${id}`, imageUrl: `https://img/${id}.jpg`, thumbnailUrl: "", sourceUrl: "https://src/" + id, ...over });
const suggestion = (over = {}) => ({ status: "ready", kind: "movie", recordId: "tmdb:603", provider: "tmdb", title: "The Matrix", year: "1999", description: "A hacker.", sourceUrl: "https://tmdb/603", autoAssociated: false, artwork: art("p1"), error: "", ...over });

test("suggestionResult needs a ready suggestion with an allowed provider record id", () => {
  assert.deepEqual(suggestionResult(suggestion()), {
    id: "603", provider: "tmdb", kind: "movie", title: "The Matrix", year: "1999", description: "A hacker.", rating: null, sourceUrl: "https://tmdb/603", confidence: 0, artwork: [art("p1")],
  });
  assert.equal(suggestionResult(undefined), null);
  assert.equal(suggestionResult(suggestion({ status: "searching" })), null);
  assert.equal(suggestionResult(suggestion({ status: "error" })), null);
  assert.equal(suggestionResult(suggestion(), "tv"), null);
  assert.equal(suggestionResult(suggestion(), "movie").id, "603");
  assert.equal(suggestionResult(suggestion({ recordId: "tvdb:5" })), null);
  assert.equal(suggestionResult(suggestion({ recordId: "wikidata:Q1" })).provider, "wikidata");
  assert.equal(suggestionResult(suggestion({ kind: "tv", recordId: "tvmaze:77" })).kind, "tv");
  assert.equal(suggestionResult(suggestion({ kind: "tv", recordId: "wikidata:Q1" })), null);
  assert.equal(suggestionResult(suggestion({ recordId: "tmdb:" })), null);
  assert.equal(suggestionResult(suggestion({ recordId: "603" })), null);
  assert.equal(suggestionResult(suggestion({ recordId: undefined })), null);
});

test("suggestionResult falls back for title and source, and marks automatic associations", () => {
  const bare = suggestionResult(suggestion({ title: "", sourceUrl: "", artwork: art("p2", { title: "From art", sourceUrl: "https://art" }), autoAssociated: true }));
  assert.deepEqual([bare.title, bare.sourceUrl, bare.confidence], ["From art", "https://art", 1]);
  const none = suggestionResult(suggestion({ title: "", sourceUrl: "", artwork: null }));
  assert.deepEqual([none.title, none.sourceUrl, none.artwork], ["Untitled", "", []]);
});

test("suggestionMatchSet builds a clue and the single result, or an empty set when the record is unusable", () => {
  assert.equal(suggestionMatchSet(undefined, "x.mkv", "movie"), null);
  assert.equal(suggestionMatchSet(suggestion({ kind: "tv" }), "x.mkv", "movie"), null);
  const set = suggestionMatchSet(suggestion({ title: "  The Matrix  " }), "x.mkv", "movie");
  assert.deepEqual(set.clue, { title: "The Matrix", year: "1999", yearEnd: null, certification: null });
  assert.deepEqual([set.results.length, set.warnings, set.status, set.error], [1, [], "ready", ""]);
  const artworkTitle = suggestionMatchSet(suggestion({ title: " ", artwork: art("p", { title: " Art Title " }) }), "x.mkv", "movie");
  assert.equal(artworkTitle.clue.title, "Art Title");
  const fallback = suggestionMatchSet(suggestion({ title: "", artwork: null, recordId: "bad" }), "Some Film.mkv", "movie");
  assert.deepEqual([fallback.clue.title, fallback.results], ["Some Film", []]);
  const failed = suggestionMatchSet(suggestion({ status: "error", error: "offline" }), "x.mkv", "movie");
  assert.deepEqual([failed.status, failed.error, failed.results], ["error", "offline", []]);
});

test("suggestionDetail only accepts television suggestions", () => {
  assert.equal(suggestionDetail(suggestion()), null);
  assert.equal(suggestionDetail(suggestion({ kind: "tv", recordId: "tvdb:81189" })).id, "81189");
  assert.equal(suggestionDetail(undefined), null);
});

test("dedupeArtwork keeps the first image per url and drops images without any url", () => {
  const images = [art("a"), art("a2", { imageUrl: "https://img/a.jpg" }), art("b", { imageUrl: "", thumbnailUrl: "https://img/t.jpg" }), art("c", { imageUrl: "", thumbnailUrl: "" }), art("d")];
  assert.deepEqual(dedupeArtwork(images).map((image) => image.id), ["a", "b", "d"]);
  assert.deepEqual(dedupeArtwork([]), []);
});

test("uniqueArtwork drops duplicates, empty ids and oversized ids", () => {
  assert.deepEqual(uniqueArtwork([art("a"), art("a"), art(""), art("x".repeat(1025)), art("x".repeat(1024))]).map((image) => image.id.length), [1, 1024]);
});

test("normalizeArt passes arrays through and treats anything else as empty", () => {
  const list = [art("a")];
  assert.equal(normalizeArt(list), list);
  for (const value of [null, undefined, "x", { 0: 1 }, 5]) assert.deepEqual(normalizeArt(value), []);
});

test("metadataScanKey is a stable fingerprint of the root, paths, sizes and modified times", () => {
  const fnv = (source) => { let hash = 2166136261; for (let index = 0; index < source.length; index += 1) { hash ^= source.charCodeAt(index); hash = Math.imul(hash, 16777619); } return hash >>> 0; };
  const scan = { root: "/lib", entries: [{ path: "/lib/a.mkv", size_bytes: 10, modified_unix_ms: 5 }, { path: "/lib/b.mkv", size_bytes: 20 }] };
  const expected = fnv("/lib\n/lib/a.mkv\u000010\u00005\n/lib/b.mkv\u000020\u0000");
  assert.equal(metadataScanKey(scan), `2:${expected}`);
  assert.equal(metadataScanKey({ root: "\\lib", entries: [{ path: "\\lib\\a.mkv", size_bytes: 10, modified_unix_ms: 5 }, { path: "\\lib\\b.mkv", size_bytes: 20 }] }), metadataScanKey(scan));
  assert.notEqual(metadataScanKey({ ...scan, entries: [{ ...scan.entries[0], size_bytes: 11 }, scan.entries[1]] }), metadataScanKey(scan));
  assert.notEqual(metadataScanKey({ ...scan, entries: [{ ...scan.entries[0], modified_unix_ms: 6 }, scan.entries[1]] }), metadataScanKey(scan));
  assert.match(metadataScanKey({ root: "/x", entries: [] }), /^0:\d+$/);
});

test("candidateEvidence delegates to the match label with the result's own kind", () => {
  const result = { id: "1", provider: "tmdb", kind: "movie", title: "Heat", year: "1995", description: null, rating: null, sourceUrl: "", confidence: 0, artwork: [] };
  const clue = { title: "Heat", year: "1995", yearEnd: null, certification: null };
  assert.deepEqual(candidateEvidence(result, clue), metadataMatchLabel(result, { ...clue, kind: "movie" }));
  assert.deepEqual(candidateEvidence({ ...result, kind: "tv" }, clue), metadataMatchLabel({ ...result, kind: "tv" }, { ...clue, kind: "tv" }));
});

test("clueCaption joins the title with the year range and certification", () => {
  assert.equal(clueCaption({ title: "Heat", year: null, yearEnd: null, certification: null }), "Heat");
  assert.equal(clueCaption({ title: "Heat", year: "1995", yearEnd: null, certification: null }), "Heat · 1995");
  assert.equal(clueCaption({ title: "Show", year: "2008", yearEnd: "2013", certification: "TV-MA" }), "Show · 2008–2013 · TV-MA");
  assert.equal(clueCaption({ title: "Heat", year: null, yearEnd: "2013", certification: "R" }), "Heat · R");
});

test("safeHttpUrl returns a normalized http or https url and nothing else", () => {
  assert.equal(safeHttpUrl("https://a.example/x?y=1"), "https://a.example/x?y=1");
  assert.equal(safeHttpUrl("HTTP://A.EXAMPLE"), "http://a.example/");
  for (const value of [null, undefined, "", "javascript:alert(1)", "ftp://a.example", "file:///etc/passwd", "not a url", "data:text/html,x"]) assert.equal(safeHttpUrl(value), null, String(value));
});

test("assignedMoviePoster finds the chosen candidate", () => {
  const assignment = { posterId: "b", artworkCandidates: [art("a"), art("b")] };
  assert.equal(assignedMoviePoster(assignment).id, "b");
  assert.equal(assignedMoviePoster({ ...assignment, posterId: "zzz" }), undefined);
  assert.equal(assignedMoviePoster({ ...assignment, artworkCandidates: [] }), undefined);
});

test("artworkFileExtension keeps known image extensions and defaults to jpg", () => {
  const ext = (imageUrl) => artworkFileExtension({ imageUrl });
  assert.deepEqual(["https://x/a.PNG?v=1", "https://x/a.webp", "https://x/a.jpeg", "https://x/a.jpg"].map(ext), ["png", "webp", "jpeg", "jpg"]);
  assert.deepEqual(["https://x/a.gif", "https://x/poster", "not a url", ""].map(ext), ["jpg", "jpg", "jpg", "jpg"]);
});

test("artwork targets name the poster files beside media, series folders and season folders", () => {
  assert.deepEqual(movieArtworkTarget({ name: "Film (2001).mkv", parentPath: "/lib/Film" }), { directory: "/lib/Film", basename: "Film (2001)-poster" });
  assert.deepEqual(movieArtworkTarget({ name: "Film", parentPath: "/lib" }), { directory: "/lib", basename: "Film-poster" });
  assert.equal(movieArtworkTarget({ name: ".hidden", parentPath: "/lib" }).basename, ".hidden-poster");
  assert.deepEqual(seriesArtworkTargets("/lib/Show"), [{ directory: "/lib/Show", basename: "folder" }, { directory: "/lib/Show", basename: "poster" }]);
  assert.deepEqual(seasonArtworkTargets("/lib/Show", "/lib/Show/Season 2", 2), [
    { directory: "/lib/Show/Season 2", basename: "poster" }, { directory: "/lib/Show/Season 2", basename: "Season02" }, { directory: "/lib/Show", basename: "season2-poster" },
  ]);
  assert.deepEqual(seasonArtworkTargets("/lib/Show", "/lib/Show/Specials", 0).map((target) => target.basename), ["poster", "season-specials-poster", "season-specials-poster"]);
  assert.deepEqual(seasonArtworkTargets("/s", "/s/S12", 12).map((target) => target.basename), ["poster", "Season12", "season12-poster"]);
});

test("movedPathForSource returns the renamed path or the original", () => {
  const items = [{ sourcePath: "/a", targetPath: "/b" }];
  assert.equal(movedPathForSource("/a", items), "/b");
  assert.equal(movedPathForSource("/z", items), "/z");
  assert.equal(movedPathForSource("/a", []), "/a");
});

test("providerLabel names each provider", () => {
  assert.deepEqual(["tmdb", "wikidata", "tvmaze", "tvdb", "local"].map(providerLabel), ["TMDB", "Wikidata", "TVmaze", "TheTVDB", "Earlier review"]);
});

test("reviewableArtwork de-duplicates, and for Wikidata keeps only images naming every title word", () => {
  assert.deepEqual(reviewableArtwork("tmdb", "Heat", [art("a"), art("a")]).map((image) => image.id), ["a"]);
  assert.deepEqual(reviewableArtwork("tmdb", "Heat", "nope"), []);
  const images = [art("1", { title: "File:The Matrix poster.jpg" }), art("2", { title: "File : Matrix Reloaded.jpg" }), art("3", { title: "Random book cover.jpg" })];
  assert.deepEqual(reviewableArtwork("wikidata", "The Matrix", images).map((image) => image.id), ["1", "2"]);
  assert.deepEqual(reviewableArtwork("wikidata", "The Matrix Reloaded", images).map((image) => image.id), ["2"]);
  assert.deepEqual(reviewableArtwork("wikidata", "The Film", images), []);
  assert.deepEqual(reviewableArtwork("wikidata", "Up", images), []);
  assert.deepEqual(reviewableArtwork("wikidata", "Matrix", images).map((image) => image.id), ["1", "2"]);
});

test("candidatePreview is the first reviewable image of a result", () => {
  assert.equal(candidatePreview({ provider: "tmdb", title: "Heat", artwork: [art("a"), art("b")] }).id, "a");
  assert.equal(candidatePreview({ provider: "tmdb", title: "Heat", artwork: [] }), undefined);
  assert.equal(candidatePreview({ provider: "wikidata", title: "Heat", artwork: [art("a", { title: "Other.jpg" })] }), undefined);
});

test("providerEpisodeTextReady checks that saved episodes carry the provider's text and every season has a record", () => {
  const group = { files: [{ path: "/e1", season: 1, episode: 1, multipleEpisodes: false }, { path: "/e2", season: 1, episode: 2, multipleEpisodes: false }], seasonDirectories: [{ season: 1 }, { season: 0 }] };
  const review = (over = {}) => ({
    episodes: [{ season: 1, episode: 1, title: "Pilot", summary: "Intro", airdate: "2020-01-01" }, { season: 1, episode: 2, title: "Two", summary: "", airdate: null }],
    episodeAssignments: { "/e1": { title: "Pilot", summary: "Intro", airdate: "2020-01-01" }, "/e2": { title: "Two", summary: "", airdate: "" } },
    seasonRecords: { 1: "rec" }, ...over,
  });
  assert.equal(providerEpisodeTextReady(review(), group), true);
  assert.equal(providerEpisodeTextReady(review({ episodes: [] }), group), false);
  assert.equal(providerEpisodeTextReady(review({ episodeAssignments: { "/e2": review().episodeAssignments["/e2"] } }), group), false);
  assert.equal(providerEpisodeTextReady(review({ episodeAssignments: { ...review().episodeAssignments, "/e1": { title: "Pilot", summary: " ", airdate: "2020-01-01" } } }), group), false);
  assert.equal(providerEpisodeTextReady(review({ episodeAssignments: { ...review().episodeAssignments, "/e1": { title: "Pilot", summary: "Intro", airdate: "" } } }), group), false);
  assert.equal(providerEpisodeTextReady(review({ episodeAssignments: { ...review().episodeAssignments, "/e1": { title: " ", summary: "Intro", airdate: "2020-01-01" } } }), group), false);
  assert.equal(providerEpisodeTextReady(review({ seasonRecords: {} }), group), false);
  const skipped = { files: [{ path: "/x", season: null, episode: 1, multipleEpisodes: false }, { path: "/y", season: 1, episode: 0, multipleEpisodes: false }, { path: "/z", season: 1, episode: 9, multipleEpisodes: false }, { path: "/m", season: 1, episode: 1, multipleEpisodes: true }], seasonDirectories: [{ season: 0 }] };
  assert.equal(providerEpisodeTextReady(review({ episodeAssignments: {} }), skipped), true);
});
