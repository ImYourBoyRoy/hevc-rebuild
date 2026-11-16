// ./desktop/tests/library-cards.test.mjs
// Characterization tests for the poster-card builders extracted from the route (poster-card slice):
// media/series/season/episode cards, the animation and selection tri-states, the browse views
// (root, series, season), the breadcrumb trail and the lookup banner. The route's state is reached
// through injected getters, so every branch runs in node. Run: pnpm test:library-cards

import assert from "node:assert/strict";
import test from "node:test";
import { createLibraryCards } from "../src/lib/library-cards.ts";
import { bytes } from "../src/lib/format.ts";
import { proposedFilenameForEntry } from "../src/lib/filename-template.ts";
import { queuePathKey } from "../src/lib/queue-paths.ts";

const templates = { movie: "{title} [[({year})]]{ext}", series: "{series_title}", season: "Season {season:02}", episode: "S{season:02}E{episode:02}.{episode_title}{ext}" };

const entry = (over = {}) => ({
  path: "/lib/Film (2001).mkv", name: "Film (2001).mkv", relative_path: "Film (2001).mkv", size_bytes: 1_500_000_000, status: "Ready",
  already_processed: false, error: null, warning: null, queue_stale: null, profile: null, ...over,
});

const profile = (over = {}) => ({ video: { secondary_count: 0, ...over.video }, audio_count: 2, subtitle_count: 3, attachment_count: 1, ...over });

function harness(over = {}) {
  const state = {
    movieReviews: {}, seriesReviews: {}, suggestions: {}, previews: {}, localPosters: {}, selected: [], progress: null,
    lookup: null, browse: null, groups: [], visible: [], page: [], queued: [], tuneOverrides: {},
    prefs: { organizerTemplates: templates, fileOutputDirs: {}, rebuildOutputDir: null },
    ...over.state,
  };
  const setBrowse = [];
  const deps = {
    getQueueMovieReviews: () => state.movieReviews,
    getQueueSeriesReviews: () => state.seriesReviews,
    getQueueMatchSuggestions: () => state.suggestions,
    getQueueArtworkPreviews: () => state.previews,
    getLibraryLocalPosters: () => state.localPosters,
    getAppPreferences: () => state.prefs,
    getMetadataLookupStatus: () => state.lookup,
    getQueuedEntries: () => state.queued,
    getFileTuneOverrides: () => state.tuneOverrides,
    getSelectedPaths: () => state.selected,
    getRunProgress: () => state.progress,
    getLibraryBrowse: () => state.browse,
    setLibraryBrowse: (value) => { state.browse = value; setBrowse.push(value); },
    getLibrarySeriesGroups: () => state.groups,
    getVisibleEntries: () => state.visible,
    getCurrentPageEntries: () => state.page,
    reviewedSeriesForPath: (path) => state.seriesReviews[path] ?? state.seriesReviews[path.split("/").slice(0, -1).join("/")],
    seriesReviewStorageKey: (root) => `key:${root}`,
    settingsForEntry: () => ({ settings: true }),
    resourceForecastFor: (item) => item.resource_forecast,
    queueRunStateForPath: (path) => state.runStates?.[path],
    effectiveContentTuneForEntry: (item, review) => (review?.animated || item.animated ? "animation" : "none"),
    statusLabel: (status) => `status:${status}`,
    profileSummary: (item) => `profile:${item.name}`,
    chosenPoster: (path) => path,
    providerPosterUrl: (review) => review?.providerPoster ?? "",
    remoteArtworkUrl: (image) => image?.imageUrl ?? "",
    seriesSuggestionArtwork: (group) => state.groupArtwork?.[group.root] ?? null,
    visiblePosterUrl: (local, remote) => (local ? `local:${local}` : remote || null),
    seasonPosterSources: (review, season) => state.seasonSources?.[season] ?? { localPath: "", remotePath: "" },
    seasonArtworkPath: (review, season) => state.seasonSources?.[season]?.localPath ?? "",
    displayPosterUrl: (path) => (path ? `shown:${path}` : null),
    ...over.deps,
  };
  return { cards: createLibraryCards(deps), state, setBrowse };
}

const movieReview = (over = {}) => ({ title: "Reviewed Film", year: "2001", recordId: "tmdb:1", posterId: "p1", localPosterPath: "", artworkCandidates: [{ id: "p1", thumbnailUrl: "https://img/thumb.jpg", imageUrl: "https://img/full.jpg" }], ...over });

test("libraryPosterCard maps the entry, selection, byte size and stream lines", () => {
  const item = entry({ profile: profile({ video: { secondary_count: 1 } }), warning: "careful", queue_stale: "changed", resource_forecast: { level: "guarded", planned_workers: 2, planned_frame_threads: null } });
  const h = harness({ state: { selected: [item.path] } });
  const card = h.cards.libraryPosterCard(item);
  assert.deepEqual([card.path, card.filename, card.relativePath, card.selected, card.selection, card.kind, card.seasonNumber], [item.path, item.name, item.relative_path, true, "all", "media", null]);
  assert.deepEqual([card.statusLabel, card.sourceBytes, card.profileLine], ["status:Ready", bytes(1_500_000_000), "profile:Film (2001).mkv"]);
  assert.equal(card.streamLine, "2 video · 2 audio · 3 subtitles · 1 attachments");
  assert.equal(card.memoryLine, "guarded · 2 workers · — frame threads");
  assert.deepEqual([card.warning, card.stale, card.error, card.mediaPaths], ["careful", "changed", "", [item.path]]);
  const bare = harness().cards.libraryPosterCard(entry({ error: "bad" }));
  assert.deepEqual([bare.streamLine, bare.memoryLine, bare.selection, bare.error], ["Streams not available yet", "", "none", "bad"]);
});

test("libraryPosterCard title and year fall back from review to suggestion to the file stem", () => {
  const item = entry();
  const reviewed = harness({ state: { movieReviews: { [item.path]: movieReview() }, suggestions: { [item.path]: { status: "ready", title: "Suggested", year: "1999" } } } }).cards.libraryPosterCard(item);
  assert.deepEqual([reviewed.title, reviewed.year], ["Reviewed Film", "2001"]);
  const suggested = harness({ state: { suggestions: { [item.path]: { status: "ready", title: " Suggested ", year: "1999" } } } }).cards.libraryPosterCard(item);
  assert.deepEqual([suggested.title, suggested.year], ["Suggested", "1999"]);
  const searching = harness({ state: { suggestions: { [item.path]: { status: "searching", title: "Hidden", year: "1" } } } }).cards.libraryPosterCard(item);
  assert.deepEqual([searching.title, searching.year], ["Film (2001)", ""]);
  const blankTitle = harness({ state: { movieReviews: { [item.path]: movieReview({ title: "  ", year: "" }) } } }).cards.libraryPosterCard(item);
  assert.equal(blankTitle.title, "Film (2001)");
});

test("libraryPosterCard poster precedence is local preview, then reviewed art, then suggestion art", () => {
  const item = entry();
  const key = queuePathKey("/art/poster.jpg");
  const withLocal = harness({ state: { movieReviews: { [item.path]: movieReview({ localPosterPath: "/art/poster.jpg" }) }, previews: { [key]: "data:local" } } }).cards.libraryPosterCard(item);
  assert.deepEqual([withLocal.posterUrl, withLocal.localArtworkPath], ["data:local", "/art/poster.jpg"]);
  const reviewedOnly = harness({ state: { movieReviews: { [item.path]: movieReview() } } }).cards.libraryPosterCard(item);
  assert.equal(reviewedOnly.posterUrl, "https://img/thumb.jpg");
  const fullImage = harness({ state: { movieReviews: { [item.path]: movieReview({ artworkCandidates: [{ id: "p1", imageUrl: "https://img/full.jpg" }] }) } } }).cards.libraryPosterCard(item);
  assert.equal(fullImage.posterUrl, "https://img/full.jpg");
  const suggestionOnly = harness({ state: { suggestions: { [item.path]: { status: "ready", title: "S", artwork: { imageUrl: "https://s/full.jpg", thumbnailUrl: "https://s/thumb.jpg" } } } } }).cards.libraryPosterCard(item);
  assert.deepEqual([suggestionOnly.posterUrl, suggestionOnly.matchLabel], ["https://s/thumb.jpg", "Suggested"]);
  const none = harness().cards.libraryPosterCard(item);
  assert.deepEqual([none.posterUrl, none.matchState, none.matchLabel], [null, "none", "No artwork yet"]);
});

test("libraryPosterCard match state covers searching, error, matched, suggested and the lookup banner", () => {
  const item = entry();
  const stateOf = (state) => harness({ state }).cards.libraryPosterCard(item);
  assert.deepEqual([stateOf({ suggestions: { [item.path]: { status: "searching" } } }).matchState, stateOf({ suggestions: { [item.path]: { status: "searching" } } }).matchLabel], ["matching", "Matching"]);
  assert.deepEqual([stateOf({ suggestions: { [item.path]: { status: "error" } } }).matchState, stateOf({ suggestions: { [item.path]: { status: "error" } } }).matchLabel], ["error", "Lookup failed"]);
  assert.equal(stateOf({ movieReviews: { [item.path]: movieReview() }, suggestions: { [item.path]: { status: "error" } } }).matchState, "matched");
  assert.equal(stateOf({ suggestions: { [item.path]: { status: "ready", title: "T", autoAssociated: true } } }).matchState, "matched");
  assert.equal(stateOf({ suggestions: { [item.path]: { status: "ready", title: "T" } } }).matchState, "suggested");
  assert.equal(stateOf({ suggestions: { [item.path]: { status: "ready" } } }).matchState, "none");
  const lookup = stateOf({ lookup: { status: "searching" } });
  assert.deepEqual([lookup.matchState, lookup.matchLabel], ["matching", "Matching"]);
  assert.equal(stateOf({ lookup: { status: "idle" } }).matchState, "none");
});

test("libraryPosterCard provider, animation flag, proposed name and output line", () => {
  const item = entry({ animated: true });
  const reviewed = harness({ state: { movieReviews: { [item.path]: movieReview({ recordId: "tvmaze:9" }) } } }).cards.libraryPosterCard(item);
  assert.deepEqual([reviewed.provider, reviewed.animated], ["TVMAZE", true]);
  assert.equal(harness({ state: { suggestions: { [item.path]: { status: "ready", title: "T", provider: "wikidata" } } } }).cards.libraryPosterCard(item).provider, "WIKIDATA");
  assert.deepEqual([harness().cards.libraryPosterCard(entry()).provider, harness().cards.libraryPosterCard(entry()).animated], ["", false]);
  const proposal = proposedFilenameForEntry(item, movieReview(), undefined, undefined, templates);
  const proposed = harness({ state: { movieReviews: { [item.path]: movieReview() } } }).cards.libraryPosterCard(item);
  assert.equal(proposed.proposedName, proposal.name !== item.name ? proposal.name : null);
  assert.equal(proposed.proposedNeedsReview, proposal.needsTitleReview);
  assert.deepEqual([harness().cards.libraryPosterCard(item).proposedName, harness().cards.libraryPosterCard(item).proposedNeedsReview], [null, false]);
  const custom = harness({ state: { prefs: { organizerTemplates: templates, fileOutputDirs: { [item.path]: "/out/a" }, rebuildOutputDir: "/out/default" } } }).cards.libraryPosterCard(item);
  assert.deepEqual([custom.outputLine, custom.outputIsCustom], ["Output: /out/a", true]);
  const fallback = harness({ state: { prefs: { organizerTemplates: templates, fileOutputDirs: {}, rebuildOutputDir: "/out/default" } } }).cards.libraryPosterCard(item);
  assert.deepEqual([fallback.outputLine, fallback.outputIsCustom], ["Default output: /out/default", false]);
  assert.equal(harness().cards.libraryPosterCard(item).outputLine, "Output: source folder");
});

test("libraryPosterCard run label and progress only for the matching running file", () => {
  const item = entry();
  const progress = { filePath: item.path, phase: "Encoding", filePercent: 42 };
  const h = harness({ state: { progress, runStates: { [item.path]: { status: "running", mode: "organize" } } } });
  const card = h.cards.libraryPosterCard(item);
  assert.deepEqual([card.progressLabel, card.progressPercent], ["Encoding", 42]);
  assert.match(card.runLabel, / · Rename$/);
  const other = harness({ state: { progress: { ...progress, filePath: "/other.mkv" }, runStates: { [item.path]: { status: "running", mode: "full" } } } }).cards.libraryPosterCard(item);
  assert.deepEqual([other.progressLabel, other.progressPercent], ["", null]);
  assert.match(other.runLabel, / · full$/);
  assert.equal(harness({ state: { runStates: { [item.path]: { status: "running" } } } }).cards.libraryPosterCard(item).runLabel.includes(" · "), false);
  assert.equal(harness().cards.libraryPosterCard(item).runLabel, "");
});

test("animationSelection is none, some or all, using overrides for unqueued paths", () => {
  const a = entry({ path: "/a.mkv", animated: true });
  const b = entry({ path: "/b.mkv" });
  const h = harness({ state: { queued: [a, b], tuneOverrides: { "/ghost.mkv": "animation", "/plain.mkv": "none" } } });
  assert.equal(h.cards.animationSelection([]), "none");
  assert.equal(h.cards.animationSelection(["/a.mkv"]), "all");
  assert.equal(h.cards.animationSelection(["/b.mkv"]), "none");
  assert.equal(h.cards.animationSelection(["/a.mkv", "/b.mkv"]), "some");
  assert.equal(h.cards.animationSelection(["/ghost.mkv"]), "all");
  assert.equal(h.cards.animationSelection(["/plain.mkv"]), "none");
});

test("cardSelection is none, some or all of the selected paths", () => {
  const h = harness({ state: { selected: ["/a", "/b"] } });
  assert.deepEqual(["/a", "/b"].map((p) => h.cards.cardSelection([p])), ["all", "all"]);
  assert.equal(h.cards.cardSelection(["/a", "/c"]), "some");
  assert.equal(h.cards.cardSelection(["/c"]), "none");
});

test("seriesProviderLabel prefers a real record, then the suggestion provider", () => {
  const group = { root: "/lib/Show" };
  const h = harness({ state: { groupArtwork: { "/lib/Show": { provider: "tvmaze" } } } });
  assert.equal(h.cards.seriesProviderLabel(group, { recordId: "tmdb:1" }), "TMDB");
  assert.equal(h.cards.seriesProviderLabel(group, { recordId: "local-nfo" }), "TVMAZE");
  assert.equal(h.cards.seriesProviderLabel(group, undefined), "TVMAZE");
  assert.equal(harness().cards.seriesProviderLabel(group, undefined), "");
});

const showGroup = (over = {}) => ({
  root: "/lib/Show", folderName: "Show", episodePaths: ["/lib/Show/Season 1/S01E01.Pilot.mkv", "/lib/Show/Season 1/S01E02.Second.mkv", "/lib/Show/Season 2/S02E01.Return.mkv"],
  seasons: [
    { season: 1, label: "Season 1", directory: "/lib/Show/Season 1", episodePaths: ["/lib/Show/Season 1/S01E01.Pilot.mkv", "/lib/Show/Season 1/S01E02.Second.mkv"] },
    { season: 2, label: "Season 2", directory: "", episodePaths: ["/lib/Show/Season 2/S02E01.Return.mkv"] },
  ], ...over,
});

test("groupPosterCard summarises seasons and episodes and picks the poster", () => {
  const group = showGroup();
  const h = harness({ state: { selected: [group.episodePaths[0]], seriesReviews: { [group.episodePaths[0]]: { title: " The Show ", year: "2020", recordId: "tmdb:3" } }, localPosters: { [queuePathKey(group.root)]: "/lib/Show/poster.jpg" } } });
  const card = h.cards.groupPosterCard(group);
  assert.deepEqual([card.kind, card.path, card.filename, card.title, card.year, card.provider], ["series", group.root, "Show", "The Show", "2020", "TMDB"]);
  assert.deepEqual([card.summary, card.reviewLabel, card.selection, card.mediaPaths], ["2 seasons · 3 episodes", "Review series", "some", group.episodePaths]);
  assert.deepEqual([card.posterUrl, card.localArtworkPath, card.matchState, card.matchLabel, card.proposedName], ["local:/lib/Show/poster.jpg", "/lib/Show/poster.jpg", "local", "", null]);
  const single = harness().cards.groupPosterCard(showGroup({ episodePaths: [group.episodePaths[0]], seasons: [group.seasons[0]] }));
  assert.deepEqual([single.summary, single.title, single.matchState, single.posterUrl], ["1 season · 1 episode", "Show", "none", null]);
});

test("groupPosterCard falls back to the review's remote poster or the suggestion art", () => {
  const group = showGroup();
  const remote = harness({ state: { seriesReviews: { [group.episodePaths[0]]: { title: "S", year: "", providerPoster: "https://p/poster.jpg", localPosterPath: "https://p/x.jpg" } } } }).cards.groupPosterCard(group);
  assert.deepEqual([remote.posterUrl, remote.localArtworkPath, remote.matchState], ["https://p/poster.jpg", "https://p/poster.jpg", "local"]);
  const suggestion = harness({ state: { groupArtwork: { [group.root]: { imageUrl: "https://s/art.jpg" } } } }).cards.groupPosterCard(group);
  assert.equal(suggestion.posterUrl, "https://s/art.jpg");
});

test("seasonPosterCard names the season and tracks whether its poster is local, awaited or absent", () => {
  const group = showGroup();
  const withLocal = harness({ state: { seasonSources: { 1: { localPath: "/lib/Show/Season 1/poster.jpg", remotePath: "" } } } }).cards.seasonPosterCard(group, group.seasons[0]);
  assert.deepEqual([withLocal.kind, withLocal.path, withLocal.title, withLocal.seasonNumber, withLocal.matchState, withLocal.posterUrl, withLocal.summary, withLocal.reviewLabel], ["season", "/lib/Show/Season 1", "Season 1", 1, "local", "local:/lib/Show/Season 1/poster.jpg", "2 episodes", "Review season"]);
  const waiting = harness({ state: { groupArtwork: { [group.root]: { imageUrl: "x" } } } }).cards.seasonPosterCard(group, group.seasons[1]);
  assert.deepEqual([waiting.path, waiting.matchState, waiting.summary], ["/lib/Show::season:2", "matched", "1 episode"]);
  const absent = harness().cards.seasonPosterCard(group, group.seasons[1]);
  assert.equal(absent.matchState, "none");
  const reviewWaiting = harness({ state: { seriesReviews: { [group.seasons[1].episodePaths[0]]: { title: "S", recordId: "tmdb:1" } } } }).cards.seasonPosterCard(group, group.seasons[1]);
  assert.equal(reviewWaiting.matchState, "matched");
});

test("episodePosterCard uses the episode title and the season poster over the card poster", () => {
  const item = entry({ path: "/lib/Show/Season 1/S01E01.Pilot.mkv", name: "S01E01.Pilot.mkv" });
  const withSeasonPoster = harness().cards.episodePosterCard(item, "/lib/Show/Season 1/poster.jpg");
  assert.deepEqual([withSeasonPoster.title, withSeasonPoster.year, withSeasonPoster.posterUrl, withSeasonPoster.localArtworkPath, withSeasonPoster.matchState, withSeasonPoster.matchLabel], ["Pilot", "", "shown:/lib/Show/Season 1/poster.jpg", "/lib/Show/Season 1/poster.jpg", "local", ""]);
  const plain = harness().cards.episodePosterCard(item, "");
  assert.deepEqual([plain.title, plain.posterUrl, plain.matchState], ["Pilot", null, "none"]);
  const notEpisode = harness().cards.episodePosterCard(entry(), "");
  assert.equal(notEpisode.title, "Film (2001)");
  const matched = harness({ state: { movieReviews: { [item.path]: movieReview() } } }).cards.episodePosterCard(item, "");
  assert.deepEqual([matched.matchState, matched.matchLabel], ["matched", "Matched"]);
  const suggested = harness({ state: { suggestions: { [item.path]: { status: "ready", title: "T", artwork: { thumbnailUrl: "https://s/t.jpg" } } } } }).cards.episodePosterCard(item, "");
  assert.deepEqual([suggested.matchState, suggested.matchLabel, suggested.posterUrl], ["suggested", "Suggested", "https://s/t.jpg"]);
});

test("buildLibraryPosterCards lists series cards then ungrouped movies from the current page", () => {
  const group = showGroup();
  const movie = entry();
  const grouped = entry({ path: group.episodePaths[0], name: "S01E01.Pilot.mkv" });
  const h = harness({ state: { groups: [group], page: [movie, grouped], visible: [movie, grouped] } });
  const cards = h.cards.buildLibraryPosterCards();
  assert.deepEqual(cards.map((card) => [card.kind, card.path]), [["series", group.root], ["media", movie.path]]);
});

test("buildLibraryPosterCards drills into a series and a season", () => {
  const group = showGroup();
  const episodes = group.episodePaths.map((path) => entry({ path, name: path.split("/").at(-1) }));
  const h = harness({ state: { groups: [group], visible: episodes, page: episodes, browse: { seriesRoot: group.root, season: null }, seasonSources: { 1: { localPath: "/lib/Show/Season 1/poster.jpg", remotePath: "" } } } });
  assert.deepEqual(h.cards.buildLibraryPosterCards().map((card) => [card.kind, card.title]), [["season", "Season 1"], ["season", "Season 2"]]);
  h.state.browse = { seriesRoot: group.root, season: 1 };
  const seasonCards = h.cards.buildLibraryPosterCards();
  assert.deepEqual(seasonCards.map((card) => [card.kind, card.title, card.posterUrl]), [["media", "Pilot", "shown:/lib/Show/Season 1/poster.jpg"], ["media", "Second", "shown:/lib/Show/Season 1/poster.jpg"]]);
  h.state.browse = { seriesRoot: group.root, season: 9 };
  assert.deepEqual(h.cards.buildLibraryPosterCards(), []);
  h.state.browse = { seriesRoot: "/missing", season: null };
  assert.deepEqual(h.cards.buildLibraryPosterCards(), []);
});

test("buildLibraryCrumbs is empty at the root and links back up the series and season trail", () => {
  const group = showGroup();
  const h = harness({ state: { groups: [group], seriesReviews: { [group.episodePaths[0]]: { title: "The Show" } } } });
  assert.deepEqual(h.cards.buildLibraryCrumbs(), []);
  h.state.browse = { seriesRoot: group.root, season: null };
  let crumbs = h.cards.buildLibraryCrumbs();
  assert.deepEqual(crumbs.map((crumb) => [crumb.label, crumb.onSelect === null]), [["All media", false], ["The Show", true]]);
  crumbs[0].onSelect();
  assert.equal(h.state.browse, null);
  h.state.browse = { seriesRoot: group.root, season: 2 };
  crumbs = h.cards.buildLibraryCrumbs();
  assert.deepEqual(crumbs.map((crumb) => [crumb.label, crumb.onSelect === null]), [["All media", false], ["The Show", false], ["Season 2", true]]);
  crumbs[1].onSelect();
  assert.deepEqual(h.state.browse, { seriesRoot: group.root, season: null });
});

test("buildLibraryCrumbs labels the series and season from saved titles with fallbacks", () => {
  const group = showGroup();
  const saved = harness({ state: { groups: [group], browse: { seriesRoot: group.root, season: 1 }, seriesReviews: { [group.episodePaths[0]]: { title: " " } } } });
  saved.state.seriesReviews["key:/lib/Show"] = { seasonRecords: { 1: { title: "Saved Season" } } };
  assert.deepEqual(saved.cards.buildLibraryCrumbs().map((crumb) => crumb.label), ["All media", "Show", "Saved Season"]);
  const labelled = harness({ state: { groups: [group], browse: { seriesRoot: group.root, season: 2 } } });
  assert.equal(labelled.cards.buildLibraryCrumbs()[2].label, "Season 2");
  const unknown = harness({ state: { groups: [group], browse: { seriesRoot: group.root, season: 7 } } });
  assert.equal(unknown.cards.buildLibraryCrumbs()[2].label, "Season 7");
  const missing = harness({ state: { groups: [], browse: { seriesRoot: "/gone", season: null } } });
  assert.equal(missing.cards.buildLibraryCrumbs()[1].label, "Series");
});

test("libraryLookupMessage shows progress or the error and is otherwise empty", () => {
  const h = harness();
  assert.equal(h.cards.libraryLookupMessage(), "");
  h.state.lookup = { status: "searching", done: 3, total: 10 };
  assert.equal(h.cards.libraryLookupMessage(), "Matching titles and posters · 3 of 10");
  h.state.lookup = { status: "error", message: "offline" };
  assert.equal(h.cards.libraryLookupMessage(), "offline");
  h.state.lookup = { status: "done" };
  assert.equal(h.cards.libraryLookupMessage(), "");
});
