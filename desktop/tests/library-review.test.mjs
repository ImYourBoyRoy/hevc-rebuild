// ./desktop/tests/library-review.test.mjs
// Characterization tests for the library review controller extracted from the route (review
// slice B): creating and patching movie/series/season/episode reviews, opening review dialogs,
// poster selection and comment rules. Route state is a plain fake host. Run: pnpm test:library-review

import assert from "node:assert/strict";
import test from "node:test";
import { createLibraryReviewController } from "../src/lib/library-review.ts";
import { createReviewFactories } from "../src/lib/review-factories.ts";
import { createPosterResolvers } from "../src/lib/poster-urls.ts";
import { queuePathKey } from "../src/lib/queue-paths.ts";

const group = (extra = {}) => ({
  root: "/s/Show", folderName: "Show", episodePaths: ["/s/Show/Season 1/S01E01.Pilot.mkv", "/s/Show/Season 1/S01E02.Two.mkv"],
  seasons: [{ season: 1, directory: "/s/Show/Season 1", label: "Season 01", episodePaths: ["/s/Show/Season 1/S01E01.Pilot.mkv", "/s/Show/Season 1/S01E02.Two.mkv"] }], ...extra,
});
const entry = (path, extra = {}) => ({ path, name: path.split("/").at(-1), relative_path: path.split("/").slice(-3).join("/"), profile: null, ...extra });
const review = (extra = {}) => ({
  title: "Show", year: "2020", certification: "", recordId: "", shortDescription: "", plot: "", genresText: "", ratingText: "", sourceUrl: "", artworkCandidates: [],
  posterId: "", localPosterPath: "", localSeasonPosterPaths: {}, seasonRecords: {}, episodes: [], episodeAssignments: {}, ...extra,
});

function harness({ groups = [group()], hostOverrides = {}, entries } = {}) {
  const calls = [];
  const queuedEntries = entries ?? groups.flatMap((item) => item.episodePaths.map((path) => entry(path)));
  const host = {
    queueSeriesReviews: {}, queueMovieReviews: {}, fileTuneOverrides: {}, libraryLocalPosters: {}, libraryLocalRecords: {}, libraryBrowse: null,
    librarySeriesGroups: groups, scanResult: null, queuedEntries, libraryPosterCards: [], settings: { metadata: { comment: " Global " } },
    ensureQueueArtworkPreview: (media, art) => { calls.push(["preview", media, art]); },
    showLibraryReview: async (target) => { calls.push(["show", target]); },
    ...hostOverrides,
  };
  const factories = createReviewFactories(() => host.queuedEntries, () => host.queueSeriesReviews, () => host.libraryLocalPosters);
  const posters = createPosterResolvers(() => ({}), () => host.libraryLocalPosters, () => ({}));
  const controller = createLibraryReviewController({
    host,
    blankSeriesReview: factories.blankSeriesReview, seriesReviewStorageKey: factories.seriesReviewStorageKey, episodeDraft: factories.episodeDraft,
    chosenPoster: posters.chosenPoster, seasonArtworkPath: posters.seasonArtworkPath, displayPosterUrl: posters.displayPosterUrl,
  });
  return { controller, host, calls };
}

test("ensureSeriesReview creates a review from the folder, seeding the title and the on-disk poster", () => {
  const h = harness({ hostOverrides: { libraryLocalPosters: { [queuePathKey("/s/Show")]: "/s/Show/poster.jpg" } } });
  const key = h.controller.ensureSeriesReview(group());
  assert.equal(key, "/s/Show");
  assert.deepEqual([h.host.queueSeriesReviews[key].title, h.host.queueSeriesReviews[key].localPosterPath], ["Show", "/s/Show/poster.jpg"]);
});

test("ensureSeriesReview keeps an unchanged review object, but repairs missing containers and blank titles", () => {
  const complete = review();
  const h = harness({ hostOverrides: { queueSeriesReviews: { "/s/Show": complete } } });
  h.controller.ensureSeriesReview(group());
  assert.equal(h.host.queueSeriesReviews["/s/Show"], complete);
  const broken = harness({ hostOverrides: { queueSeriesReviews: { "/s/Show": { ...review({ title: "  " }), seasonRecords: undefined, episodeAssignments: undefined } } } });
  broken.controller.ensureSeriesReview(group());
  const fixed = broken.host.queueSeriesReviews["/s/Show"];
  assert.deepEqual([fixed.title, fixed.seasonRecords, fixed.episodeAssignments], ["Show", {}, {}]);
  const windows = harness({ hostOverrides: { queueSeriesReviews: { "C:\\Media\\Show": review() } } });
  assert.equal(windows.controller.ensureSeriesReview(group({ root: "c:/media/show" })), "C:\\Media\\Show");
});

test("ensureSeasonReview creates the season record from the generated label and the premiere year", () => {
  const h = harness({ hostOverrides: { queueSeriesReviews: { "/s/Show": review({ year: "2020", episodes: [{ season: 1, episode: 1, title: "x", airdate: "2021-03-04", summary: null, sourceUrl: "" }] }) } } });
  const result = h.controller.ensureSeasonReview(group(), 1);
  assert.deepEqual(result, { key: "/s/Show", seasonKey: "1" });
  assert.deepEqual(h.host.queueSeriesReviews["/s/Show"].seasonRecords[1], { title: "Season 01", year: "2021", certification: "", plot: "", genresText: "", ratingText: "" });
  const unknown = harness();
  unknown.controller.ensureSeasonReview(group(), 7);
  assert.equal(unknown.host.queueSeriesReviews["/s/Show"].seasonRecords[7].title, "Season 7");
});

test("ensureSeasonReview moves an inherited season year to the premiere year, and leaves a custom one alone", () => {
  const episodes = [{ season: 1, episode: 1, title: "x", airdate: "2021-03-04", summary: null, sourceUrl: "" }];
  const inherited = harness({ hostOverrides: { queueSeriesReviews: { "/s/Show": review({ year: "2020", episodes, seasonRecords: { 1: { title: "S", year: "2020", certification: "", plot: "", genresText: "", ratingText: "" } } }) } } });
  inherited.controller.ensureSeasonReview(group(), 1);
  assert.equal(inherited.host.queueSeriesReviews["/s/Show"].seasonRecords[1].year, "2021");
  const custom = harness({ hostOverrides: { queueSeriesReviews: { "/s/Show": review({ year: "2020", episodes, seasonRecords: { 1: { title: "S", year: "1999", certification: "", plot: "", genresText: "", ratingText: "" } } }) } } });
  custom.controller.ensureSeasonReview(group(), 1);
  assert.equal(custom.host.queueSeriesReviews["/s/Show"].seasonRecords[1].year, "1999");
});

test("ensureEpisodeReview drafts an assignment from the file name and fills it from the catalog", () => {
  const path = "/s/Show/Season 1/S01E02.Two.mkv";
  const h = harness({ hostOverrides: { queueSeriesReviews: { "/s/Show": review({ episodes: [{ season: 1, episode: 2, title: "Catalog Two", airdate: "2020-02-02", summary: "Catalog plot", sourceUrl: "https://c" }] }) } } });
  const { key, episode } = h.controller.ensureEpisodeReview(group(), path);
  assert.equal(key, "/s/Show");
  assert.deepEqual([episode.season, episode.episode, episode.title, episode.summary, episode.airdate, episode.sourceUrl], [1, 2, "Catalog Two", "Catalog plot", "2020-02-02", "https://c"]);
  assert.equal(h.host.queueSeriesReviews["/s/Show"].episodeAssignments[path], episode);
});

test("ensureEpisodeReview returns an up-to-date assignment unchanged", () => {
  const path = "/s/Show/Season 1/S01E01.Pilot.mkv";
  const existing = { season: 1, episode: 1, title: "My Title", airdate: null, summary: "mine", sourceUrl: "" };
  const h = harness({ hostOverrides: { queueSeriesReviews: { "/s/Show": review({ episodeAssignments: { [path]: existing } }) } } });
  const { episode } = h.controller.ensureEpisodeReview(group(), path);
  assert.equal(episode, existing);
  assert.equal(h.host.queueSeriesReviews["/s/Show"].episodeAssignments[path], existing);
});

test("patchSeriesReview updates a saved review, starts from a blank one when the group exists, and ignores unknown series", () => {
  const h = harness();
  h.controller.patchSeriesReview("/s/Show", (current) => ({ ...current, plot: "P" }));
  assert.equal(h.host.queueSeriesReviews["/s/Show"].plot, "P");
  h.controller.patchSeriesReview("/s/Show", (current) => ({ ...current, plot: "Q" }));
  assert.equal(h.host.queueSeriesReviews["/s/Show"].plot, "Q");
  const none = harness({ groups: [] });
  none.controller.patchSeriesReview("/nowhere", (current) => ({ ...current, plot: "P" }));
  assert.deepEqual(none.host.queueSeriesReviews, {});
});

test("patchTvField edits through the shared field rules", () => {
  const h = harness({ hostOverrides: { queueSeriesReviews: { "/s/Show": review({ certification: "TV-PG" }) } } });
  h.controller.patchTvField({ level: "series", seriesRoot: "/s/Show", season: null, episodePath: null }, "title", "New");
  assert.equal(h.host.queueSeriesReviews["/s/Show"].title, "New");
  h.controller.patchTvField({ level: "season", seriesRoot: "/s/Show", season: 1, episodePath: null }, "certification", "TV-PG");
  assert.equal(h.host.queueSeriesReviews["/s/Show"].seasonRecords[1].certification, "");
  const path = "/s/Show/Season 1/S01E01.Pilot.mkv";
  h.controller.patchTvField({ level: "episode", seriesRoot: "/s/Show", season: 1, episodePath: path }, "title", "Pilot!");
  assert.equal(h.host.queueSeriesReviews["/s/Show"].episodeAssignments[path].title, "Pilot!");
});

test("movie comments equal to the Settings comment are stored empty, and unknown movies are ignored", () => {
  const h = harness({ hostOverrides: { queueMovieReviews: { "/m/a.mkv": { title: "A", comment: "old" } } } });
  h.controller.setMovieComment("/m/a.mkv", " Global ");
  assert.equal(h.host.queueMovieReviews["/m/a.mkv"].comment, "");
  h.controller.setMovieComment("/m/a.mkv", "Mine");
  assert.equal(h.host.queueMovieReviews["/m/a.mkv"].comment, "Mine");
  h.controller.setMovieComment("/m/none.mkv", "X");
  assert.deepEqual(Object.keys(h.host.queueMovieReviews), ["/m/a.mkv"]);
});

test("review comments equal to what would be inherited are stored empty", () => {
  const h = harness({ hostOverrides: { queueSeriesReviews: { "/s/Show": review({ comment: "Series note" }) } } });
  const season = { level: "season", seriesRoot: "/s/Show", season: 1, episodePath: null };
  h.controller.setReviewComment(season, "Series note");
  assert.equal(h.host.queueSeriesReviews["/s/Show"].seasonRecords[1].comment, "");
  h.controller.setReviewComment(season, "Season own");
  assert.equal(h.host.queueSeriesReviews["/s/Show"].seasonRecords[1].comment, "Season own");
  h.controller.setReviewComment({ level: "series", seriesRoot: "/s/Show", season: null, episodePath: null }, "Global");
  assert.equal(h.host.queueSeriesReviews["/s/Show"].comment, "");
  const none = harness();
  none.controller.setReviewComment(season, "x");
  assert.deepEqual(none.host.queueSeriesReviews, {});
  assert.equal(h.controller.sharedCommentFor({ level: "series", seriesRoot: "/s/Show", season: null, episodePath: null }, review()), "Global");
});

test("selecting a poster stores it for the season or series, and previews only local files", () => {
  const h = harness({ hostOverrides: { queueSeriesReviews: { "/s/Show": review({ localPosterPath: "/old", posterId: "p" }) } } });
  h.controller.selectReviewPoster({ level: "season", seriesRoot: "/s/Show", season: 1, episodePath: null }, "/s/Show/Season 1/poster.jpg");
  assert.deepEqual(h.host.queueSeriesReviews["/s/Show"].localSeasonPosterPaths, { 1: "/s/Show/Season 1/poster.jpg" });
  assert.deepEqual(h.calls[0], ["preview", "/s/Show/Season 1/S01E01.Pilot.mkv", "/s/Show/Season 1/poster.jpg"]);
  h.controller.selectReviewPoster({ level: "series", seriesRoot: "/s/Show", season: null, episodePath: null }, "https://img/p.jpg");
  assert.deepEqual([h.host.queueSeriesReviews["/s/Show"].localPosterPath, h.host.queueSeriesReviews["/s/Show"].posterId], ["https://img/p.jpg", ""]);
  assert.equal(h.calls.length, 1);
  h.controller.selectReviewPoster({ level: "episode", seriesRoot: "/s/Show", season: 1, episodePath: "/s/Show/Season 1/S01E02.Two.mkv" }, "/x.jpg");
  assert.deepEqual(h.calls.at(-1), ["preview", "/s/Show/Season 1/S01E02.Two.mkv", "/x.jpg"]);
  assert.equal(h.host.queueSeriesReviews["/s/Show"].localSeasonPosterPaths[1], "/x.jpg");
});

test("animated flags are set per path without touching others", () => {
  const h = harness({ hostOverrides: { fileTuneOverrides: { "/a": "none", "/keep": "animation" } } });
  h.controller.setPathsAnimated(["/a", "/b"], true);
  assert.deepEqual(h.host.fileTuneOverrides, { "/a": "animation", "/b": "animation", "/keep": "animation" });
  h.controller.setPathsAnimated(["/b"], false);
  assert.equal(h.host.fileTuneOverrides["/b"], "none");
});

test("seriesGroupForRoot matches by normalized root and returns nothing for unknown folders", () => {
  const h = harness({ groups: [group({ root: "C:\\Media\\Show" })] });
  assert.equal(h.controller.seriesGroupForRoot("c:/media/show").folderName, "Show");
  assert.equal(h.controller.seriesGroupForRoot("/other"), undefined);
});

test("an episode review target needs a group and a season/episode file name, and prepares the assignment", () => {
  const path = "/s/Show/Season 1/S01E01.Pilot.mkv";
  const h = harness();
  assert.deepEqual(h.controller.tvReviewForEpisode(entry(path)), { level: "episode", seriesRoot: "/s/Show", season: 1, episodePath: path });
  assert.ok(h.host.queueSeriesReviews["/s/Show"].episodeAssignments[path]);
  assert.equal(h.controller.tvReviewForEpisode(entry("/s/Show/Season 1/not-an-episode.mkv")), null);
  assert.equal(harness({ groups: [] }).controller.tvReviewForEpisode(entry(path)), null);
});

test("opening a series review prepares it, previews its poster files and shows the dialog for its sample episode", () => {
  const records = { [queuePathKey("/s/Show")]: { artwork: [{ name: "poster.jpg", role: "poster", path: "/s/Show/poster.jpg" }] } };
  const h = harness({ hostOverrides: { libraryLocalRecords: records } });
  h.controller.openSeriesReview("/s/Show");
  assert.deepEqual(h.calls, [
    ["preview", "/s/Show/Season 1/S01E01.Pilot.mkv", "/s/Show/poster.jpg"],
    ["show", { level: "series", seriesRoot: "/s/Show", season: null, episodePath: "/s/Show/Season 1/S01E01.Pilot.mkv" }],
  ]);
  assert.ok(h.host.queueSeriesReviews["/s/Show"]);
  const none = harness({ groups: [] });
  none.controller.openSeriesReview("/s/Show");
  assert.deepEqual(none.calls, []);
});

test("opening a season review uses that season's first episode and its artwork", () => {
  const records = { [queuePathKey("/s/Show/Season 1")]: { artwork: [{ name: "poster.jpg", role: "poster", path: "/s/Show/Season 1/poster.jpg" }] } };
  const h = harness({ hostOverrides: { libraryLocalRecords: records } });
  h.controller.openSeasonReview("/s/Show", 1);
  assert.deepEqual(h.calls.at(-1), ["show", { level: "season", seriesRoot: "/s/Show", season: 1, episodePath: "/s/Show/Season 1/S01E01.Pilot.mkv" }]);
  assert.deepEqual(h.calls[0], ["preview", "/s/Show/Season 1/S01E01.Pilot.mkv", "/s/Show/Season 1/poster.jpg"]);
  assert.ok(h.host.queueSeriesReviews["/s/Show"].seasonRecords[1]);
});

test("openLibraryReview and reviewCurrentBrowse choose the review level from the card or the browse position", () => {
  const cards = [{ path: "/s/Show", kind: "series" }, { path: "/s/Show/Season 1", kind: "season", seasonNumber: 1 }, { path: "/m/a.mkv", kind: "movie" }];
  const h = harness({ hostOverrides: { libraryPosterCards: cards, libraryBrowse: { seriesRoot: "/s/Show", season: null } } });
  h.controller.openLibraryReview("/s/Show");
  assert.equal(h.calls.at(-1)[1].level, "series");
  h.controller.openLibraryReview("/s/Show/Season 1");
  assert.equal(h.calls.at(-1)[1].level, "season");
  const before = h.calls.length;
  h.controller.openLibraryReview("/m/a.mkv");
  h.controller.openLibraryReview("/unknown");
  assert.equal(h.calls.length, before);
  h.controller.reviewCurrentBrowse();
  assert.equal(h.calls.at(-1)[1].level, "series");
  h.host.libraryBrowse = { seriesRoot: "/s/Show", season: 1 };
  h.controller.reviewCurrentBrowse();
  assert.deepEqual([h.calls.at(-1)[1].level, h.calls.at(-1)[1].season], ["season", 1]);
  h.host.libraryBrowse = null;
  const count = h.calls.length;
  h.controller.reviewCurrentBrowse();
  assert.equal(h.calls.length, count);
});

test("the active review poster prefers a chosen local poster, the on-disk poster, then the provider poster", () => {
  const seriesTarget = { level: "series", seriesRoot: "/s/Show", season: null, episodePath: null };
  const withProvider = review({ artworkCandidates: [{ imageUrl: "https://p/1.jpg", thumbnailUrl: "" }] });
  const h = harness({ hostOverrides: { libraryLocalPosters: { [queuePathKey("/s/Show")]: "/s/Show/disk.jpg" } } });
  assert.equal(h.controller.activeReviewPosterPath(seriesTarget, withProvider), "/s/Show/disk.jpg");
  assert.equal(h.controller.activeReviewPosterPath(seriesTarget, review({ localPosterPath: "https://p/chosen.jpg" })), "https://p/chosen.jpg");
  assert.equal(harness().controller.activeReviewPosterPath(seriesTarget, withProvider), "https://p/1.jpg");
  const seasonTarget = { level: "season", seriesRoot: "/s/Show", season: 1, episodePath: null };
  assert.equal(h.controller.activeReviewPosterPath(seasonTarget, withProvider), "https://p/1.jpg");
  const seasonDisk = harness({ hostOverrides: { libraryLocalPosters: { [queuePathKey("/s/Show/Season 1")]: "/s/Show/Season 1/disk.jpg" } } });
  assert.equal(seasonDisk.controller.activeReviewPosterPath(seasonTarget, withProvider), "/s/Show/Season 1/disk.jpg");
});

test("headings, paths and poster choices are answered from the saved review and the group", () => {
  const h = harness({ hostOverrides: { queueSeriesReviews: { "/s/Show": review({ title: "Saved" }) } } });
  assert.equal(h.controller.tvReviewHeading({ level: "series", seriesRoot: "/s/Show", season: null, episodePath: null }), "Saved");
  assert.deepEqual(h.controller.tvReviewPaths({ level: "season", seriesRoot: "/s/Show", season: 1, episodePath: null }).length, 2);
  assert.deepEqual(h.controller.reviewPosterChoices(group(), null, undefined), []);
  assert.deepEqual(h.controller.posterPathsForReview(group(), null), []);
  assert.equal(h.controller.reviewedSeriesForPath("/s/Show/Season 1/e.mkv").title, "Saved");
  assert.equal(h.controller.reviewedSeriesForPath("/elsewhere/e.mkv"), undefined);
});
