// ./desktop/tests/review-factories.test.mjs
// Characterization tests for the library review factories extracted from the route
// (slice 3 of the +page.svelte segregation): blank review shapes, review key lookup,
// container-tag reading and episode drafts. Run: pnpm test:review-factories

import assert from "node:assert/strict";
import test from "node:test";
import { blankSeasonReview, containerTags, createReviewFactories } from "../src/lib/review-factories.ts";
import { queuePathKey } from "../src/lib/queue-paths.ts";

const entry = (name, path, tags) => ({
  path,
  name,
  profile: tags === undefined ? null : { source_metadata: { format: { tags } } },
});

function factories({ entries = [], reviews = {}, posters = {} } = {}) {
  return createReviewFactories(() => entries, () => reviews, () => posters);
}

test("blankSeasonReview inherits year, certification and genres from the series only", () => {
  assert.deepEqual(blankSeasonReview("Season 2", undefined), {
    title: "Season 2", year: "", certification: "", plot: "", genresText: "", ratingText: "",
  });
  assert.deepEqual(blankSeasonReview("Season 2", { year: "2019", certification: "TV-MA", genresText: "Drama", plot: "ignored", ratingText: "9" }), {
    title: "Season 2", year: "2019", certification: "TV-MA", plot: "", genresText: "Drama", ratingText: "",
  });
});

test("blankSeriesReview seeds the title from the folder and the poster from known library posters", () => {
  const group = { folderName: "Show Name", root: "/media/Show Name" };
  const { blankSeriesReview } = factories({ posters: { [queuePathKey(group.root)]: "/media/Show Name/poster.jpg" } });
  assert.deepEqual(blankSeriesReview(group), {
    title: "Show Name", year: "", certification: "", recordId: "", shortDescription: "", plot: "",
    genresText: "", ratingText: "", sourceUrl: "", artworkCandidates: [], posterId: "",
    localPosterPath: "/media/Show Name/poster.jpg", localSeasonPosterPaths: {}, seasonRecords: {},
    episodes: [], episodeAssignments: {},
  });
  assert.equal(factories().blankSeriesReview(group).localPosterPath, "");
});

test("blankSeriesReview returns independent containers on every call", () => {
  const { blankSeriesReview } = factories();
  const group = { folderName: "S", root: "/s" };
  const a = blankSeriesReview(group);
  const b = blankSeriesReview(group);
  a.episodes.push(1);
  a.episodeAssignments.x = 1;
  assert.deepEqual(b.episodes, []);
  assert.deepEqual(b.episodeAssignments, {});
});

test("seriesReviewStorageKey reuses an existing key that normalizes to the same root", () => {
  const { seriesReviewStorageKey } = factories({ reviews: { "C:\\Media\\Show": {}, "/other": {} } });
  assert.equal(seriesReviewStorageKey("c:/media/show"), "C:\\Media\\Show");
  assert.equal(seriesReviewStorageKey("/brand/new"), "/brand/new");
  assert.equal(factories().seriesReviewStorageKey("/x"), "/x");
});

test("seriesReviewStorageKey reads reviews lazily", () => {
  let reviews = {};
  const { seriesReviewStorageKey } = createReviewFactories(() => [], () => reviews, () => ({}));
  assert.equal(seriesReviewStorageKey("/a/b"), "/a/b");
  reviews = { "/a/b/": {} };
  assert.equal(seriesReviewStorageKey("/a/b"), "/a/b/");
});

test("containerTags lower-cases keys, trims values and drops blanks and non-strings", () => {
  assert.deepEqual(containerTags(undefined), {});
  assert.deepEqual(containerTags(entry("a.mkv", "/a.mkv")), {});
  assert.deepEqual(containerTags({ profile: { source_metadata: { format: "oops" } } }), {});
  assert.deepEqual(containerTags({ profile: { source_metadata: { format: { tags: "oops" } } } }), {});
  assert.deepEqual(
    containerTags(entry("a.mkv", "/a.mkv", { TITLE: " Pilot ", Synopsis: "Plot", empty: "  ", number: 7 })),
    { title: "Pilot", synopsis: "Plot" },
  );
});

test("episodeFileFacts takes title, plot and an air date only when dates differ across the series folder", () => {
  const path = "/media/Show/Season 1/S01E01.mkv";
  const entries = [
    entry("S01E01.mkv", path, { title: "Pilot", showtitle: "Show", description: "A plot", date_released: "2020-01-01" }),
    entry("S01E02.mkv", "/media/Show/Season 1/S01E02.mkv", { date_released: "2020-01-08" }),
  ];
  const { episodeFileFacts } = factories({ entries });
  assert.deepEqual(episodeFileFacts(path), { title: "Pilot", plot: "A plot", airdate: "2020-01-01" });
  const sameDate = factories({ entries: [entries[0], entry("S01E02.mkv", "/media/Show/Season 1/S01E02.mkv", { date_released: "2020-01-01" })] });
  assert.equal(sameDate.episodeFileFacts(path).airdate, null);
});

test("episodeFileFacts ignores a title equal to the show title and prefers synopsis over description", () => {
  const path = "/m/Show/S1/a.mkv";
  const { episodeFileFacts } = factories({ entries: [entry("a.mkv", path, { title: "Show", showtitle: "Show", synopsis: "Syn", description: "Desc" })] });
  assert.deepEqual(episodeFileFacts(path), { title: "", plot: "Syn", airdate: null });
  assert.deepEqual(factories().episodeFileFacts("/unknown.mkv"), { title: "", plot: "", airdate: null });
});

test("episodeDraft combines the file name identity with embedded tags and safe defaults", () => {
  const path = "/media/Show/Season 2/S02E05.The.Title.mkv";
  const { episodeDraft } = factories({ entries: [entry("S02E05.The.Title.mkv", path, { synopsis: "Plot" })] });
  assert.deepEqual(episodeDraft(path), { season: 2, episode: 5, title: "The Title", airdate: null, summary: "Plot", sourceUrl: "" });
  const tagged = factories({ entries: [entry("S02E05.The.Title.mkv", path, { title: "Tag Title" })] });
  assert.equal(tagged.episodeDraft(path).title, "Tag Title");
});

test("episodeDraft falls back to the file name, then to a generic title, and clamps episode 0", () => {
  const plain = "/media/Show/random file.mkv";
  const { episodeDraft } = factories({ entries: [entry("random file.mkv", plain)] });
  assert.deepEqual(episodeDraft(plain), { season: 0, episode: 1, title: "random file.mkv", airdate: null, summary: "", sourceUrl: "" });
  assert.equal(factories().episodeDraft("/gone.mkv").title, "Episode");
  const zero = "/m/S01E00.Special.mkv";
  assert.equal(factories({ entries: [entry("S01E00.Special.mkv", zero)] }).episodeDraft(zero).episode, 1);
});
