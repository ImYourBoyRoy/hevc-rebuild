// ./desktop/tests/run-overrides.test.mjs
// Characterization tests for the per-file poster and metadata overrides sent to the engine,
// extracted from the route. State the route supplied is injected as getters.
// Run: pnpm test:run-overrides

import assert from "node:assert/strict";
import test from "node:test";
import { createRunOverrides } from "../src/lib/run-overrides.ts";

const entry = (path) => ({ path, name: path.split("/").at(-1) });
const episode = (season, extra = {}) => ({ season, episode: 1, title: "Pilot", airdate: null, summary: null, sourceUrl: "", ...extra });
const series = (extra = {}) => ({
  title: "Show", year: "2020", certification: "TV-PG", genresText: "", localPosterPath: "", localSeasonPosterPaths: {}, seasonRecords: {}, episodeAssignments: {}, ...extra,
});

function overrides({ movies = {}, seriesFor = () => undefined, posters = {}, comment, fileComment = () => "" } = {}) {
  return createRunOverrides({
    getMovieReviews: () => movies,
    reviewedSeriesForPath: seriesFor,
    getLocalPosters: () => posters,
    getSettingsComment: () => comment,
    resolvedFileComment: fileComment,
  });
}

test("a movie with a local poster file uses it, and a provider URL poster is never sent", () => {
  const { selectedPosterOverrides } = overrides({
    movies: { "/m/a.mkv": { localPosterPath: "/m/poster.jpg" }, "/m/b.mkv": { localPosterPath: "https://img/p.jpg" }, "/m/c.mkv": { localPosterPath: "" } },
  });
  assert.deepEqual(selectedPosterOverrides([entry("/m/a.mkv"), entry("/m/b.mkv"), entry("/m/c.mkv")]), { "/m/a.mkv": "/m/poster.jpg" });
});

test("an episode prefers its season poster file, then the on-disk season poster, over the series poster", () => {
  const review = series({ localPosterPath: "/s/poster.jpg", localSeasonPosterPaths: { 2: "/s/S2/saved.jpg" }, episodeAssignments: { "/s/S2/e1.mkv": episode(2) } });
  const { selectedPosterOverrides } = overrides({ seriesFor: () => review, posters: { "/s/S2": "/s/S2/disk.jpg", "/s/S3": "/s/S3/disk.jpg" } });
  assert.deepEqual(selectedPosterOverrides([entry("/s/S2/e1.mkv")]), { "/s/S2/e1.mkv": "/s/S2/saved.jpg" });
  const noSaved = overrides({ seriesFor: () => series({ localPosterPath: "/s/poster.jpg", episodeAssignments: { "/s/S3/e1.mkv": episode(3) } }), posters: { "/s/S3": "/s/S3/disk.jpg" } });
  assert.deepEqual(noSaved.selectedPosterOverrides([entry("/s/S3/e1.mkv")]), { "/s/S3/e1.mkv": "/s/S3/disk.jpg" });
});

test("a provider season poster blocks the series fallback, otherwise the series poster file is used", () => {
  const provider = series({ localPosterPath: "/s/poster.jpg", localSeasonPosterPaths: { 1: "https://img/s1.jpg" }, episodeAssignments: { "/s/Season 1/e1.mkv": episode(1) } });
  assert.deepEqual(overrides({ seriesFor: () => provider }).selectedPosterOverrides([entry("/s/Season 1/e1.mkv")]), {});
  const plain = series({ localPosterPath: "/s/poster.jpg" });
  assert.deepEqual(overrides({ seriesFor: () => plain }).selectedPosterOverrides([entry("/s/Season 1/e1.mkv")]), { "/s/Season 1/e1.mkv": "/s/poster.jpg" });
  const remoteOnly = series({ localPosterPath: "https://img/series.jpg" });
  assert.deepEqual(overrides({ seriesFor: () => remoteOnly }).selectedPosterOverrides([entry("/s/Season 1/e1.mkv")]), {});
  assert.deepEqual(overrides().selectedPosterOverrides([entry("/s/x.mkv")]), {});
});

test("the season comes from the assignment, matched by normalized path, or from the folder name", () => {
  const windows = series({ localSeasonPosterPaths: { 4: "C:\\S\\Season 4\\p.jpg" }, episodeAssignments: { "C:\\S\\Season 4\\e1.mkv": episode(4) } });
  assert.deepEqual(overrides({ seriesFor: () => windows }).selectedPosterOverrides([entry("c:/s/season 4/e1.mkv")]), { "c:/s/season 4/e1.mkv": "C:\\S\\Season 4\\p.jpg" });
  const folder = series({ localSeasonPosterPaths: { 7: "/s/Season_07/p.jpg" } });
  assert.deepEqual(overrides({ seriesFor: () => folder }).selectedPosterOverrides([entry("/s/season.07/e1.mkv"), entry("/s/Season 7/e1.mkv"), entry("/s/Extras/e1.mkv")]), {
    "/s/season.07/e1.mkv": "/s/Season_07/p.jpg", "/s/Season 7/e1.mkv": "/s/Season_07/p.jpg",
  });
});

test("a movie review turns its non-blank fields into metadata, splits genres and omits the shared comment", () => {
  const movie = { title: " Film ", year: "2001", certification: "", shortDescription: "Short", plot: "Plot", genresText: "Drama; Comedy, ,Thriller", ratingText: "7.1", sourceUrl: "https://s", comment: "Shared" };
  const { selectedMetadataOverrides } = overrides({ movies: { "/m/a.mkv": movie }, comment: " Shared " });
  assert.deepEqual(selectedMetadataOverrides([entry("/m/a.mkv")]), {
    "/m/a.mkv": { title: "Film", year: "2001", short_description: "Short", plot: "Plot", genres: ["Drama", "Comedy", "Thriller"], rating: "7.1", source_url: "https://s" },
  });
  const own = overrides({ movies: { "/m/a.mkv": { ...movie, comment: "Mine" } }, comment: "Shared" });
  assert.equal(own.selectedMetadataOverrides([entry("/m/a.mkv")])["/m/a.mkv"].comment, "Mine");
  const empty = overrides({ movies: { "/m/a.mkv": { title: "", year: "", certification: "", shortDescription: "", plot: "", genresText: "", ratingText: "", sourceUrl: "" } } });
  assert.deepEqual(empty.selectedMetadataOverrides([entry("/m/a.mkv")]), {});
});

test("genres are capped at 50 entries", () => {
  const movie = { title: "", genresText: Array.from({ length: 60 }, (_, index) => `g${index}`).join(",") };
  const result = overrides({ movies: { "/m/a.mkv": movie } }).selectedMetadataOverrides([entry("/m/a.mkv")]);
  assert.equal(result["/m/a.mkv"].genres.length, 50);
});

test("an episode gets show, season, episode, certificate precedence, bounded text and its own comment", () => {
  const review = series({
    genresText: "Comedy", seasonRecords: { 1: { certification: " TV-14 " } },
    episodeAssignments: { "/s/e1.mkv": episode(1, { title: "The Pilot", summary: "Plot text", sourceUrl: "https://ep", airdate: "2020-01-01", certification: "" }) },
  });
  const { selectedMetadataOverrides } = overrides({ seriesFor: () => review, comment: "Shared", fileComment: () => "Episode note" });
  assert.deepEqual(selectedMetadataOverrides([entry("/s/e1.mkv")]), {
    "/s/e1.mkv": {
      title: "The Pilot", show_title: "Show", year: "2020", certification: "TV-14", short_description: "Plot text", plot: "Plot text", genres: ["Comedy"],
      source_url: "https://ep", season: "1", episode: "1", airdate: "2020-01-01", comment: "Episode note",
    },
  });
  const sharedComment = overrides({ seriesFor: () => review, comment: "Shared", fileComment: () => "Shared" });
  assert.equal("comment" in sharedComment.selectedMetadataOverrides([entry("/s/e1.mkv")])["/s/e1.mkv"], false);
  const own = overrides({ seriesFor: () => series({ episodeAssignments: { "/s/e1.mkv": episode(1, { certification: "R" }) }, seasonRecords: { 1: { certification: "PG" } } }) });
  assert.equal(own.selectedMetadataOverrides([entry("/s/e1.mkv")])["/s/e1.mkv"].certification, "R");
});

test("an unassigned episode falls back to its file name and the series certificate", () => {
  const { selectedMetadataOverrides } = overrides({ seriesFor: (path) => (path.startsWith("/s/") ? series({ certification: "TV-G" }) : undefined) });
  assert.deepEqual(selectedMetadataOverrides([entry("/s/Show.S01E02.mkv")]), {
    "/s/Show.S01E02.mkv": { title: "Show.S01E02", show_title: "Show", year: "2020", certification: "TV-G" },
  });
  assert.deepEqual(selectedMetadataOverrides([entry("/x/none.mkv")]), {});
});

test("an https-only source URL is kept and others are dropped by the episode text rules", () => {
  const review = series({ episodeAssignments: { "/s/e1.mkv": episode(1, { sourceUrl: "http://insecure", summary: "S" }) } });
  const { selectedMetadataOverrides } = overrides({ seriesFor: () => review });
  assert.equal("source_url" in selectedMetadataOverrides([entry("/s/e1.mkv")])["/s/e1.mkv"], false);
});
