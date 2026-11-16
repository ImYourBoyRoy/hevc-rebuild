// ./desktop/tests/review-notes.test.mjs
// Characterization tests for the pure library-review helpers extracted from the route
// (review slice A): inherited certificates and comments, the notes shown for them, field patching,
// poster choices, review lookup by folder, and titles. Run: pnpm test:review-notes

import assert from "node:assert/strict";
import test from "node:test";
import {
  applyTvField,
  certificateNoteFor,
  commentNoteFor,
  ownCertificateFor,
  ownCommentFor,
  posterPathsForReview,
  resolvedFileComment,
  reviewPosterChoices,
  reviewedSeriesForPath,
  seasonPlotNote,
  seasonPremiereYear,
  seasonRatingNote,
  sharedCertificateFor,
  sharedCommentFor,
  tvReviewHeading,
  tvReviewPaths,
} from "../src/lib/review-notes.ts";
import { queuePathKey } from "../src/lib/queue-paths.ts";

const episode = (season, number, extra = {}) => ({ season, episode: number, title: `E${number}`, airdate: null, summary: null, sourceUrl: "", ...extra });
const review = (extra = {}) => ({
  title: "Show", year: "2020", certification: "", plot: "", genresText: "", ratingText: "", comment: "", localPosterPath: "", localSeasonPosterPaths: {},
  seasonRecords: {}, episodes: [], episodeAssignments: {}, artworkCandidates: [], ...extra,
});
const target = (level, extra = {}) => ({ level, seriesRoot: "/s", season: null, episodePath: null, ...extra });

test("seasonPremiereYear uses the earliest valid air date of the season, from catalog and assignments", () => {
  assert.equal(seasonPremiereYear(undefined, 1), "");
  assert.equal(seasonPremiereYear(review(), null), "");
  const r = review({
    episodes: [episode(1, 1, { airdate: "2019-05-01" }), episode(2, 1, { airdate: "2021-01-01" })],
    episodeAssignments: { "/a": episode(1, 2, { airdate: "2018-12-31" }), "/b": episode(1, 3, { airdate: "bad" }), "/c": episode(1, 4, { airdate: null }) },
  });
  assert.equal(seasonPremiereYear(r, 1), "2018");
  assert.equal(seasonPremiereYear(r, 2), "2021");
  assert.equal(seasonPremiereYear(r, 3), "");
});

test("certificates inherit from season to episode and fall back to the series", () => {
  const r = review({ certification: " TV-PG ", seasonRecords: { 2: { certification: " TV-14 " } }, episodeAssignments: { "/e": episode(2, 1, { certification: " R " }) } });
  assert.equal(sharedCertificateFor(target("series"), r), "TV-PG");
  assert.equal(sharedCertificateFor(target("season", { season: 2 }), r), "TV-PG");
  assert.equal(sharedCertificateFor(target("episode", { season: 2 }), r), "TV-14");
  assert.equal(sharedCertificateFor(target("episode", { season: 3 }), r), "TV-PG");
  assert.equal(sharedCertificateFor(target("episode", { season: null }), r), "TV-PG");
  assert.equal(ownCertificateFor(target("episode", { episodePath: "/e" }), r), "R");
  assert.equal(ownCertificateFor(target("season", { season: 2 }), r), "TV-14");
  assert.equal(ownCertificateFor(target("series"), r), "");
  assert.equal(ownCertificateFor(target("episode", { episodePath: "/none" }), r), "");
});

test("certificate notes say where the shown certificate comes from", () => {
  const base = review({ certification: "TV-PG" });
  assert.equal(certificateNoteFor(target("series"), base), "");
  assert.equal(certificateNoteFor(target("season", { season: 1 }), base), "Using the series certificate.");
  assert.equal(certificateNoteFor(target("season", { season: 1 }), review({ seasonRecords: { 1: { certification: "R" } } })), "Saved for this season.");
  assert.equal(certificateNoteFor(target("episode", { season: 1, episodePath: "/e" }), review({ episodeAssignments: { "/e": episode(1, 1, { certification: "R" }) } })), "Saved for this episode.");
  assert.equal(certificateNoteFor(target("season", { season: 1 }), review()), "No certificate is published.");
  assert.equal(certificateNoteFor(target("episode", { season: 1, episodePath: "/e" }), review({ certification: "G", seasonRecords: { 1: { certification: "PG" } } })), "Using this season's certificate.");
});

test("season notes prefer the season's own plot and score, then the series', then none", () => {
  assert.equal(seasonPlotNote(review({ plot: "P" }), { plot: "own" }), "Saved for this season.");
  assert.equal(seasonPlotNote(review({ plot: "P" }), { plot: " " }), "This season has no plot of its own. Showing the series description.");
  assert.equal(seasonPlotNote(review(), undefined), "No season description is published.");
  assert.equal(seasonRatingNote(review({ ratingText: "8" }), { ratingText: "9" }), "Saved for this season.");
  assert.equal(seasonRatingNote(review({ ratingText: "8" }), undefined), "This season has no score of its own. Showing the series score.");
  assert.equal(seasonRatingNote(review(), { ratingText: "" }), "No season score is published.");
});

test("shared and own comments follow settings -> series -> season -> episode precedence", () => {
  const r = review({ comment: " Series ", seasonRecords: { 1: { comment: " Season " } }, episodeAssignments: { "/e": episode(1, 1, { comment: " Ep " }) } });
  assert.equal(sharedCommentFor(target("series"), r, " Global "), "Global");
  assert.equal(sharedCommentFor(target("season", { season: 1 }), r, "Global"), "Series");
  assert.equal(sharedCommentFor(target("season", { season: 1 }), review(), " Global "), "Global");
  assert.equal(sharedCommentFor(target("episode", { season: 1 }), r, "Global"), "Season");
  assert.equal(sharedCommentFor(target("episode", { season: 2 }), r, "Global"), "Series");
  assert.equal(sharedCommentFor(target("episode", { season: null }), review(), undefined), "");
  assert.equal(ownCommentFor(target("episode", { episodePath: "/e" }), r), "Ep");
  assert.equal(ownCommentFor(target("season", { season: 1 }), r), "Season");
  assert.equal(ownCommentFor(target("series"), r), "Series");
});

test("comment notes explain which comment is in use and how to save one", () => {
  const r = review({ comment: "S", seasonRecords: { 1: { comment: "Se" } }, episodeAssignments: { "/e": episode(1, 1, { comment: "Ep" }) } });
  assert.equal(commentNoteFor(target("episode", { episodePath: "/e", season: 1 }), r), "Saved for this episode.");
  assert.equal(commentNoteFor(target("season", { season: 1 }), r), "Saved for this season. Episodes without their own comment use it.");
  assert.equal(commentNoteFor(target("series"), r), "Saved for this series. Seasons and episodes without their own comment use it.");
  assert.equal(commentNoteFor(target("episode", { episodePath: "/x", season: 1 }), r), "Using this season's comment. Edit it to save one for this episode.");
  assert.equal(commentNoteFor(target("episode", { episodePath: "/x", season: 2 }), r), "Using the series comment. Edit it to save one for this episode.");
  assert.equal(commentNoteFor(target("season", { season: 2 }), r), "Using the series comment. Edit it to save one for this season.");
  assert.equal(commentNoteFor(target("series"), review()), "Using the comment from Settings. Edit it to save one here.");
});

test("resolvedFileComment picks the episode, then season, then series comment", () => {
  const r = review({ comment: "S", seasonRecords: { 1: { comment: "Se" } } });
  assert.equal(resolvedFileComment(r, episode(1, 1, { comment: "Ep" })), "Ep");
  assert.equal(resolvedFileComment(r, episode(1, 1)), "Se");
  assert.equal(resolvedFileComment(r, episode(2, 1)), "S");
  assert.equal(resolvedFileComment(r, undefined), "S");
  assert.equal(resolvedFileComment(review(), undefined), "");
});

test("reviewedSeriesForPath walks up the folders to the nearest reviewed root, including drive and filesystem roots", () => {
  const byRoot = new Map([
    [queuePathKey("/media/Show"), { title: "Show" }], [queuePathKey("C:\\Media\\Other"), { title: "Other" }], [queuePathKey("/"), { title: "Root" }],
  ]);
  assert.equal(reviewedSeriesForPath("/media/Show/Season 1/e1.mkv", byRoot).title, "Show");
  assert.equal(reviewedSeriesForPath("/media/Show", byRoot).title, "Show");
  assert.equal(reviewedSeriesForPath("c:/media/other/s1/e.mkv", byRoot).title, "Other");
  assert.equal(reviewedSeriesForPath("/elsewhere/e.mkv", byRoot).title, "Root");
  assert.equal(reviewedSeriesForPath("/media/Show/e.mkv", new Map()), undefined);
  assert.equal(reviewedSeriesForPath("D:\\x\\e.mkv", new Map([[queuePathKey("D:\\"), { title: "Drive" }]])).title, "Drive");
  assert.equal(reviewedSeriesForPath("", byRoot), undefined);
});

const draftFor = (path) => episode(0, 1, { title: `draft:${path.split("/").at(-1)}` });

test("series-level fields are written straight onto the review", () => {
  const next = applyTvField(review(), target("series"), "title", "New", draftFor);
  assert.equal(next.title, "New");
  assert.equal(applyTvField(review(), target("series"), "plot", "P", draftFor).plot, "P");
});

test("season-level fields write to the season record and drop values equal to the inherited one", () => {
  const r = review({ plot: "Series plot", ratingText: "8", certification: "TV-PG", year: "2020" });
  const own = applyTvField(r, target("season", { season: 2 }), "plot", "Season plot", draftFor);
  assert.deepEqual(own.seasonRecords[2], { title: "Season 2", year: "2020", certification: "TV-PG", plot: "Season plot", genresText: "", ratingText: "" });
  const inherited = applyTvField(r, target("season", { season: 2 }), "plot", " Series plot ", draftFor);
  assert.equal(inherited.seasonRecords[2].plot, "");
  assert.equal(applyTvField(r, target("season", { season: 2 }), "ratingText", "8", draftFor).seasonRecords[2].ratingText, "");
  assert.equal(applyTvField(r, target("season", { season: 2 }), "certification", "TV-PG", draftFor).seasonRecords[2].certification, "");
  assert.equal(applyTvField(r, target("season", { season: 2 }), "title", "Series plot", draftFor).seasonRecords[2].title, "Series plot");
});

test("episode fields start from the draft, validate numbers and keep inherited certificates empty", () => {
  const t = target("episode", { season: 1, episodePath: "/s/e1.mkv" });
  const r = review({ certification: "TV-PG" });
  const first = applyTvField(r, t, "title", "Pilot", draftFor);
  assert.deepEqual(first.episodeAssignments["/s/e1.mkv"], { season: 0, episode: 1, title: "Pilot", airdate: null, summary: null, sourceUrl: "" });
  assert.equal(applyTvField(r, t, "plot", "Plot", draftFor).episodeAssignments["/s/e1.mkv"].summary, "Plot");
  assert.equal(applyTvField(r, t, "airdate", "2020-01-01", draftFor).episodeAssignments["/s/e1.mkv"].airdate, "2020-01-01");
  assert.equal(applyTvField(r, t, "airdate", "", draftFor).episodeAssignments["/s/e1.mkv"].airdate, null);
  assert.equal(applyTvField(r, t, "comment", "C", draftFor).episodeAssignments["/s/e1.mkv"].comment, "C");
  assert.equal(applyTvField(r, t, "certification", "TV-PG", draftFor).episodeAssignments["/s/e1.mkv"].certification, "");
  assert.equal(applyTvField(r, t, "certification", "R", draftFor).episodeAssignments["/s/e1.mkv"].certification, "R");
  assert.equal(applyTvField(r, t, "season", "3", draftFor).episodeAssignments["/s/e1.mkv"].season, 3);
  assert.equal(applyTvField(r, t, "season", "0", draftFor).episodeAssignments["/s/e1.mkv"].season, 0);
  for (const bad of ["x", "-1", "1.5"]) assert.equal(applyTvField(r, t, "season", bad, draftFor), r, `season ${bad}`);
  // Pinned quirk: an empty season box counts as 0 (Number("") === 0), so clearing it moves the episode to season 0.
  assert.equal(applyTvField(r, t, "season", "", draftFor).episodeAssignments["/s/e1.mkv"].season, 0);
  assert.equal(applyTvField(r, t, "episode", "4", draftFor).episodeAssignments["/s/e1.mkv"].episode, 4);
  for (const bad of ["0", "x", "-2"]) assert.equal(applyTvField(r, t, "episode", bad, draftFor), r, `episode ${bad}`);
});

test("an existing episode assignment is patched instead of the draft", () => {
  const t = target("episode", { season: 1, episodePath: "/s/e1.mkv" });
  const r = review({ episodeAssignments: { "/s/e1.mkv": episode(1, 5, { title: "Kept" }) } });
  const next = applyTvField(r, t, "plot", "P", draftFor);
  assert.deepEqual([next.episodeAssignments["/s/e1.mkv"].title, next.episodeAssignments["/s/e1.mkv"].episode], ["Kept", 5]);
});

const group = (extra = {}) => ({ root: "/s", folderName: "S", episodePaths: ["/s/S1/e1", "/s/S2/e1"], seasons: [{ season: 1, directory: "/s/S1", episodePaths: ["/s/S1/e1"], label: "Season 1" }, { season: 2, directory: "/s/S2", episodePaths: ["/s/S2/e1"], label: "Season 2" }], ...extra });
const art = (name, role, path) => ({ name, role, path });

test("poster paths prefer the season's art plus the series art, hiding banners and logos when posters exist", () => {
  const records = {
    [queuePathKey("/s")]: { artwork: [art("poster.jpg", "poster", "/s/poster.jpg"), art("fanart.jpg", "fanart", "/s/fanart.jpg")] },
    [queuePathKey("/s/S1")]: { artwork: [art("poster.jpg", "poster", "/s/S1/poster.jpg"), art("poster2.jpg", "poster", "/s/poster.jpg")] },
  };
  assert.deepEqual(posterPathsForReview(group(), null, records), ["/s/poster.jpg"]);
  assert.deepEqual(posterPathsForReview(group(), 1, records), ["/s/S1/poster.jpg", "/s/poster.jpg"]);
  assert.deepEqual(posterPathsForReview(group(), 2, records), ["/s/poster.jpg"]);
  const onlyBanners = { [queuePathKey("/s")]: { artwork: [art("banner.jpg", "banner", "/s/banner.jpg")] } };
  assert.deepEqual(posterPathsForReview(group(), null, onlyBanners), ["/s/banner.jpg"]);
  assert.deepEqual(posterPathsForReview(group(), null, {}), []);
});

test("poster choices list local files, then new provider images, and keep a selected provider URL visible", () => {
  const records = { [queuePathKey("/s")]: { artwork: [art("poster.jpg", "poster", "/s/poster.jpg")] } };
  const display = (path) => (path === "/s/poster.jpg" ? "asset://p" : null);
  const r = review({ artworkCandidates: [{ imageUrl: "https://a/1.jpg", thumbnailUrl: "" }, { imageUrl: "", thumbnailUrl: "https://a/2.jpg" }, { imageUrl: "http://x", thumbnailUrl: "" }] });
  assert.deepEqual(reviewPosterChoices(group(), null, r, records, display), [
    { path: "/s/poster.jpg", url: "asset://p" }, { path: "https://a/1.jpg", url: "https://a/1.jpg" }, { path: "https://a/2.jpg", url: "https://a/2.jpg" },
  ]);
  const picked = review({ localPosterPath: "https://elsewhere/p.jpg" });
  assert.deepEqual(reviewPosterChoices(group(), null, picked, records, display)[0], { path: "https://elsewhere/p.jpg", url: "https://elsewhere/p.jpg" });
  const seasonPick = review({ localSeasonPosterPaths: { 2: "https://elsewhere/s2.jpg" } });
  assert.equal(reviewPosterChoices(group(), 2, seasonPick, {}, display)[0].path, "https://elsewhere/s2.jpg");
  assert.deepEqual(reviewPosterChoices(group(), null, undefined, {}, display), []);
  assert.equal(reviewPosterChoices(group(), null, review(), records, () => null)[0].url, "");
});

test("review paths cover the episode, the season, or the whole series", () => {
  assert.deepEqual(tvReviewPaths(target("episode", { episodePath: "/s/S1/e1" }), group()), ["/s/S1/e1"]);
  assert.deepEqual(tvReviewPaths(target("season", { season: 2 }), group()), ["/s/S2/e1"]);
  assert.deepEqual(tvReviewPaths(target("season", { season: 9 }), group()), []);
  assert.deepEqual(tvReviewPaths(target("series"), group()), ["/s/S1/e1", "/s/S2/e1"]);
  assert.deepEqual(tvReviewPaths(target("series"), undefined), []);
  assert.deepEqual(tvReviewPaths(target("episode", { episodePath: null }), group()), ["/s/S1/e1", "/s/S2/e1"]);
});

test("review headings use the saved title and fall back to generated names", () => {
  const r = review({ title: "Saved Show", seasonRecords: { 1: { title: "Opening" } }, episodeAssignments: { "/e": episode(1, 1, { title: " Pilot " }) } });
  assert.equal(tvReviewHeading(target("series"), r, draftFor), "Saved Show");
  assert.equal(tvReviewHeading(target("season", { season: 1 }), r, draftFor), "Opening");
  assert.equal(tvReviewHeading(target("season", { season: 4 }), r, draftFor), "Season 4");
  assert.equal(tvReviewHeading(target("episode", { episodePath: "/e" }), r, draftFor), "Pilot");
  assert.equal(tvReviewHeading(target("episode", { episodePath: "/x/new.mkv" }), r, draftFor), "draft:new.mkv");
  assert.equal(tvReviewHeading(target("episode", { episodePath: "/x/new.mkv" }), undefined, draftFor), "draft:new.mkv");
  assert.equal(tvReviewHeading(target("series"), undefined, draftFor), "");
  assert.equal(tvReviewHeading(target("season", { season: null }), r, draftFor), "");
});
