// ./desktop/tests/poster-selection.test.mjs
// Characterization tests for the poster choices extracted from MetadataWorkbench (T10.7 slice 11): choosing and toggling a series or
// season poster (local or provider), choosing a film poster, the batch apply of a local poster to several films and the film artwork
// choices. Plain-object host; the review being edited comes from injected actions.
// Run: pnpm test:poster-selection

import assert from "node:assert/strict";
import test from "node:test";
import { createPosterSelection } from "../src/lib/poster-selection.ts";

const art = (id) => ({ id, title: id, imageUrl: `https://img/${id}.jpg`, thumbnailUrl: "", sourceUrl: "" });
const file = (path) => ({ path, name: path.split("/").at(-1) });

function harness({ over = {}, root = "/lib/Show", current = null, selectedPaths = [] } = {}) {
  const host = {
    seriesAssignments: {}, movieAssignments: {}, selectedArtwork: { keep: "x" }, organizerScan: { root: "/lib", files: [file("/lib/A.mkv"), file("/lib/B.mkv"), file("/lib/C.mkv")] },
    pendingArtworkSave: { label: "pending" }, artworkMessage: "msg", pendingBatchLocalPoster: { sourcePath: "x" }, ...over,
  };
  const actions = { currentReviewedSeriesRoot: () => root, makeCurrentSeriesReview: () => current, selectedPathsInOrganizer: () => selectedPaths };
  return { poster: createPosterSelection({ host, actions }), host };
}
const review = (over = {}) => ({ title: "Show", posterId: "p", localPosterPath: "", localSeasonPosterPaths: { 1: "/s1.jpg" }, ...over });

test("updateCurrentSeriesArtwork edits the saved review, or one built from the open match, and needs a root and a review", () => {
  const saved = harness({ over: { seriesAssignments: { "/lib/Show": review(), "/other": { title: "keep" } } } });
  saved.poster.updateCurrentSeriesArtwork((r) => ({ ...r, posterId: "new" }));
  assert.deepEqual([saved.host.seriesAssignments["/lib/Show"].posterId, saved.host.seriesAssignments["/other"]], ["new", { title: "keep" }]);
  const built = harness({ current: review({ posterId: "built" }) });
  built.poster.updateCurrentSeriesArtwork((r) => ({ ...r, localPosterPath: "/x.jpg" }));
  assert.deepEqual([built.host.seriesAssignments["/lib/Show"].posterId, built.host.seriesAssignments["/lib/Show"].localPosterPath], ["built", "/x.jpg"]);
  const noRoot = harness({ root: "", current: review() });
  noRoot.poster.updateCurrentSeriesArtwork((r) => r);
  const noReview = harness();
  noReview.poster.updateCurrentSeriesArtwork((r) => r);
  assert.deepEqual([noRoot.host.seriesAssignments, noReview.host.seriesAssignments], [{}, {}]);
});

test("chooseSeriesLocalPoster toggles the local poster and clears the provider choice", () => {
  const h = harness({ over: { seriesAssignments: { "/lib/Show": review() } } });
  h.poster.chooseSeriesLocalPoster("/p.jpg");
  assert.deepEqual([h.host.seriesAssignments["/lib/Show"].posterId, h.host.seriesAssignments["/lib/Show"].localPosterPath, h.host.selectedArtwork], ["", "/p.jpg", { keep: "x", series: "" }]);
  h.poster.chooseSeriesLocalPoster("/p.jpg");
  assert.equal(h.host.seriesAssignments["/lib/Show"].localPosterPath, "");
});

test("chooseSeriesLocalSeasonPoster toggles a season's local poster", () => {
  const h = harness({ over: { seriesAssignments: { "/lib/Show": review() } } });
  h.poster.chooseSeriesLocalSeasonPoster(2, "/s2.jpg");
  assert.deepEqual(h.host.seriesAssignments["/lib/Show"].localSeasonPosterPaths, { 1: "/s1.jpg", 2: "/s2.jpg" });
  h.poster.chooseSeriesLocalSeasonPoster(2, "/s2.jpg");
  assert.deepEqual(h.host.seriesAssignments["/lib/Show"].localSeasonPosterPaths, { 1: "/s1.jpg" });
  h.poster.chooseSeriesLocalSeasonPoster(1, "/other.jpg");
  assert.deepEqual(h.host.seriesAssignments["/lib/Show"].localSeasonPosterPaths, { 1: "/other.jpg" });
});

test("choosing a provider series or season poster clears the local one and the pending save", () => {
  const h = harness({ over: { seriesAssignments: { "/lib/Show": review({ localPosterPath: "/old.jpg" }) } } });
  h.poster.chooseSeriesProviderPoster("p2");
  assert.deepEqual([h.host.seriesAssignments["/lib/Show"].posterId, h.host.seriesAssignments["/lib/Show"].localPosterPath, h.host.selectedArtwork, h.host.pendingArtworkSave, h.host.artworkMessage], ["p2", "", { keep: "x", series: "p2" }, null, ""]);
  h.host.pendingArtworkSave = { label: "again" }; h.host.artworkMessage = "again";
  h.poster.chooseSeriesProviderSeasonPoster(1, "sp1");
  assert.deepEqual([h.host.seriesAssignments["/lib/Show"].localSeasonPosterPaths, h.host.selectedArtwork["season:1"], h.host.pendingArtworkSave, h.host.artworkMessage], [{}, "sp1", null, ""]);
});

test("chooseMovieLocalPoster toggles a film's local poster and drops a pending batch", () => {
  const h = harness({ over: { movieAssignments: { "/lib/A.mkv": { title: "A", posterId: "p", localPosterPath: "" } } } });
  h.poster.chooseMovieLocalPoster("/lib/A.mkv", "/a.jpg");
  assert.deepEqual([h.host.movieAssignments["/lib/A.mkv"].posterId, h.host.movieAssignments["/lib/A.mkv"].localPosterPath, h.host.pendingBatchLocalPoster], ["", "/a.jpg", null]);
  h.poster.chooseMovieLocalPoster("/lib/A.mkv", "/a.jpg");
  assert.equal(h.host.movieAssignments["/lib/A.mkv"].localPosterPath, "");
  const none = harness();
  none.poster.chooseMovieLocalPoster("/lib/Z.mkv", "/z.jpg");
  assert.deepEqual([none.host.movieAssignments, none.host.pendingBatchLocalPoster], [{}, { sourcePath: "x" }]);
});

const films = { "/lib/A.mkv": { title: "A", posterId: "pa", localPosterPath: "" }, "/lib/B.mkv": { title: "B", posterId: "pb", localPosterPath: "" } };

test("batchLocalPosterTargets lists the selected scanned films that have a reviewed assignment", () => {
  const h = harness({ over: { movieAssignments: films }, selectedPaths: ["/lib/A.mkv", "/lib/A.mkv", "/lib/B.mkv", "/lib/C.mkv", "/lib/gone.mkv"] });
  assert.deepEqual(h.poster.batchLocalPosterTargets(), ["/lib/A.mkv", "/lib/B.mkv"]);
  assert.deepEqual(harness({ over: { organizerScan: null, movieAssignments: films }, selectedPaths: ["/lib/A.mkv"] }).poster.batchLocalPosterTargets(), []);
});

test("previewBatchLocalPoster applies directly for one film and stages a batch for several", () => {
  const one = harness({ over: { movieAssignments: films }, selectedPaths: ["/lib/A.mkv"] });
  one.poster.previewBatchLocalPoster("/lib/A.mkv", "/a.jpg");
  assert.deepEqual([one.host.movieAssignments["/lib/A.mkv"].localPosterPath, one.host.pendingBatchLocalPoster], ["/a.jpg", null]);
  const many = harness({ over: { movieAssignments: films }, selectedPaths: ["/lib/A.mkv", "/lib/B.mkv"] });
  many.poster.previewBatchLocalPoster("/lib/A.mkv", "/a.jpg");
  assert.deepEqual(many.host.pendingBatchLocalPoster, { sourcePath: "/lib/A.mkv", artworkPath: "/a.jpg", mediaPaths: ["/lib/A.mkv", "/lib/B.mkv"] });
  assert.equal(many.host.movieAssignments["/lib/A.mkv"].localPosterPath, "");
});

test("applyBatchLocalPoster gives every still-valid film the local poster, or drops a stale batch", () => {
  const pending = { sourcePath: "/lib/A.mkv", artworkPath: "/a.jpg", mediaPaths: ["/lib/A.mkv", "/lib/B.mkv", "/lib/C.mkv", "/lib/gone.mkv"] };
  const h = harness({ over: { movieAssignments: films, pendingBatchLocalPoster: pending } });
  h.poster.applyBatchLocalPoster();
  assert.deepEqual([h.host.movieAssignments["/lib/A.mkv"].localPosterPath, h.host.movieAssignments["/lib/B.mkv"].localPosterPath, h.host.movieAssignments["/lib/A.mkv"].posterId, "/lib/C.mkv" in h.host.movieAssignments, h.host.pendingBatchLocalPoster], ["/a.jpg", "/a.jpg", "", false, null]);
  const staleSource = harness({ over: { movieAssignments: films, pendingBatchLocalPoster: { ...pending, sourcePath: "/lib/gone.mkv" } } });
  staleSource.poster.applyBatchLocalPoster();
  assert.deepEqual([staleSource.host.movieAssignments["/lib/A.mkv"].localPosterPath, staleSource.host.pendingBatchLocalPoster], ["", null]);
  const nobody = harness({ over: { pendingBatchLocalPoster: { ...pending, mediaPaths: ["/lib/gone.mkv"] } } });
  nobody.poster.applyBatchLocalPoster();
  assert.equal(nobody.host.pendingBatchLocalPoster, null);
  const none = harness({ over: { pendingBatchLocalPoster: null, movieAssignments: films } });
  none.poster.applyBatchLocalPoster();
  assert.deepEqual(none.host.movieAssignments, films);
  const noScan = harness({ over: { organizerScan: null, pendingBatchLocalPoster: pending, movieAssignments: films } });
  noScan.poster.applyBatchLocalPoster();
  assert.deepEqual([noScan.host.pendingBatchLocalPoster === pending, noScan.host.movieAssignments], [true, films]);
});

test("selectMovieArtwork and chooseMovieProviderPoster edit a reviewed film only", () => {
  const h = harness({ over: { movieAssignments: { "/lib/A.mkv": { title: "A", artworkCandidates: [art("old")], posterId: "old", localPosterPath: "/x.jpg" } } } });
  h.poster.selectMovieArtwork("/lib/A.mkv", "new", art("new"));
  assert.deepEqual([h.host.movieAssignments["/lib/A.mkv"].artworkCandidates.map((image) => image.id), h.host.movieAssignments["/lib/A.mkv"].posterId, h.host.movieAssignments["/lib/A.mkv"].localPosterPath], [["new", "old"], "new", ""]);
  h.poster.selectMovieArtwork("/lib/A.mkv", "old", art("new"));
  assert.deepEqual(h.host.movieAssignments["/lib/A.mkv"].artworkCandidates.map((image) => image.id), ["new", "old"]);
  h.poster.selectMovieArtwork("/lib/A.mkv", "old");
  assert.equal(h.host.movieAssignments["/lib/A.mkv"].posterId, "old");
  h.host.movieAssignments["/lib/A.mkv"].localPosterPath = "/y.jpg";
  h.poster.chooseMovieProviderPoster("/lib/A.mkv");
  assert.equal(h.host.movieAssignments["/lib/A.mkv"].localPosterPath, "");
  const none = harness();
  none.poster.selectMovieArtwork("/lib/Z.mkv", "p", art("p"));
  none.poster.chooseMovieProviderPoster("/lib/Z.mkv");
  assert.deepEqual(none.host.movieAssignments, {});
});
