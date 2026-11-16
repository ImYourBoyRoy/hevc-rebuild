// ./desktop/tests/organizer-rename.test.mjs
// Characterization tests for the organizer selection and rename flow extracted from MetadataWorkbench (T10.7 slice 10): the selected
// files, episode lookups, the years of a show, assigning a film, the rename request with all its safety checks, previewing it and
// applying it (path moves, assignment moves, rescan and every failure report). The bridge and the callbacks are fakes.
// Run: pnpm test:organizer-rename

import assert from "node:assert/strict";
import test from "node:test";
import { createOrganizerRename } from "../src/lib/organizer-rename.ts";

const file = (path, over = {}) => ({ path, name: path.split("/").at(-1), parentPath: "/lib", extension: "mkv", season: null, episode: null, multipleEpisodes: false, ...over });
const episode = (season, number, title = `E${number}`) => ({ season, episode: number, title, airdate: null, summary: null, sourceUrl: "", artwork: [] });
const templates = { movie: "{title}{ext}", series: "{series_title}", season: "Season {season}", episode: "S{season}E{episode}" };

function harness({ desktop = true, over = {}, payloads = {}, mismatch = null, visible = null } = {}) {
  const calls = [];
  const scan = over.organizerScan === undefined ? { root: "/lib/Show", renameRoot: "/lib", files: [file("/lib/Show/e1.mkv", { season: 1, episode: 1 }), file("/lib/Show/e2.mkv", { season: 1, episode: 2 })], seasonDirectories: [{ path: "/lib/Show/Season 1", name: "Season 1", season: 1 }], episodeGaps: [], errors: [], truncated: false } : over.organizerScan;
  const host = {
    organizerScan: scan, mediaKind: "movie", detail: null, selected: null, provider: "tmdb", episodeAssignments: {}, tvRenameScope: "series-tree", organizerSource: "folder", seriesYearConflictConfirmed: false,
    selectedArtwork: {}, renamePreview: null, renameError: "", renameMessage: "", renameBusy: false, confirmRename: false, movieAssignments: {}, selectedMediaPaths: [],
    organizerTemplates: templates, textBlacklistPatterns: ["rip*"], compactItemMode: false, visibleOrganizerFiles: scan?.files ?? [], ...over,
  };
  if (visible) host.visibleOrganizerFiles = visible;
  const invoke = async (command, args) => {
    calls.push({ command, args });
    const value = payloads[command];
    if (value instanceof Error) throw value;
    return typeof value === "function" ? value(args) : value;
  };
  const actions = {
    currentSeriesYearMismatch: () => mismatch,
    onRenameState: (...args) => calls.push({ command: "onRenameState", args }),
    onPathsRenamed: (paths) => calls.push({ command: "onPathsRenamed", args: paths }),
    onOrganizerTemplatesChange: (value) => calls.push({ command: "onOrganizerTemplatesChange", args: value }),
    onStartMetadataOnly: (paths) => calls.push({ command: "onStartMetadataOnly", args: paths }),
  };
  return { ren: createOrganizerRename({ invoke, isDesktop: () => desktop, host, actions }), host, calls };
}
const tvDetail = (over = {}) => ({ title: "Show", year: "2008", seasons: [{ yearStart: "2008", yearEnd: "2013" }], episodes: [episode(1, 1, "Pilot"), episode(1, 2, "Second")], ...over });
const tvState = (over = {}) => ({ mediaKind: "tv", detail: tvDetail(), selected: { id: "1" }, selectedMediaPaths: ["/lib/Show/e1.mkv", "/lib/Show/e2.mkv"], episodeAssignments: { "/lib/Show/e1.mkv": "1/1", "/lib/Show/e2.mkv": "1/2" }, ...over });
const throwsMessage = (fn) => { try { fn(); } catch (error) { return error.message; } return null; };

test("episodeForKey reads the season and episode from a key", () => {
  const h = harness({ over: { detail: tvDetail() } });
  assert.equal(h.ren.episodeForKey("1/2").title, "Second");
  assert.equal(h.ren.episodeForKey("9/9"), undefined);
  assert.equal(h.ren.episodeForKey(undefined), undefined);
  assert.equal(h.ren.episodeForKey(""), undefined);
  assert.equal(harness().ren.episodeForKey("1/1"), undefined);
});

test("showYears gathers the four-digit years of the show and its seasons", () => {
  const years = (detail) => harness({ over: { detail } }).ren.showYears();
  assert.deepEqual(years(tvDetail()), { first: "2008", last: "2013", range: "2008-2013" });
  assert.deepEqual(years(tvDetail({ year: "2010", seasons: [{ yearStart: "2010", yearEnd: "2010" }] })), { first: "2010", last: "2010", range: "2010" });
  assert.deepEqual(years(tvDetail({ year: "Aired 2012-03-01", seasons: [{ yearStart: null, yearEnd: 5 }, { yearStart: "2009", yearEnd: "n/a" }] })), { first: "2009", last: "2012", range: "2009-2012" });
  assert.deepEqual(years(tvDetail({ year: null, seasons: [] })), { first: "", last: "", range: "" });
  assert.deepEqual(harness().ren.showYears(), { first: "", last: "", range: "" });
});

test("syncEpisodeAssignments assigns scanned episodes that the provider knows, once", () => {
  const files = [file("/a", { season: 1, episode: 1 }), file("/b", { season: 1, episode: 2, multipleEpisodes: true }), file("/c", { season: 1, episode: 9 }), file("/d", { season: null, episode: 1 }), file("/e", { season: 1, episode: 2 })];
  const h = harness({ over: { detail: tvDetail(), episodeAssignments: { "/e": "1/1" } } });
  h.ren.syncEpisodeAssignments(files);
  assert.deepEqual(h.host.episodeAssignments, { "/e": "1/1", "/a": "1/1" });
  const none = harness();
  none.ren.syncEpisodeAssignments(files);
  assert.deepEqual(none.host.episodeAssignments, {});
});

test("assignCurrentMovie copies the reviewed detail onto the chosen files", () => {
  const detail = { title: "The Matrix", year: "1999", certification: "R", shortDescription: "Free", longDescription: "Hacker", genres: ["Sci-Fi", "Action"], rating: 8.2, sourceUrl: "https://tmdb", artwork: [{ id: "p" }] };
  const h = harness({ over: { detail, selected: { id: "603" }, selectedArtwork: { movie: "p" }, renamePreview: { previewId: "x" }, renameError: "old", movieAssignments: { "/keep": { title: "k" } } } });
  h.ren.assignCurrentMovie(["/a", "/b"]);
  const expected = { title: "The Matrix", year: "1999", certification: "R", recordId: "tmdb:603", shortDescription: "Free", plot: "Hacker", genresText: "Sci-Fi, Action", ratingText: "8.2", sourceUrl: "https://tmdb", artworkCandidates: [{ id: "p" }], posterId: "p", localPosterPath: "" };
  assert.deepEqual([h.host.movieAssignments["/a"], h.host.movieAssignments["/b"], h.host.movieAssignments["/keep"]], [expected, expected, { title: "k" }]);
  assert.notEqual(h.host.movieAssignments["/a"], h.host.movieAssignments["/b"]);
  assert.notEqual(h.host.movieAssignments["/a"].artworkCandidates[0], detail.artwork[0]);
  assert.deepEqual([h.host.renamePreview, h.host.renameError], [null, ""]);
  const bare = harness({ over: { detail: { ...detail, year: null, certification: null, rating: null }, selected: null } });
  bare.ren.assignCurrentMovie(["/a"]);
  assert.deepEqual([bare.host.movieAssignments["/a"].year, bare.host.movieAssignments["/a"].certification, bare.host.movieAssignments["/a"].ratingText, bare.host.movieAssignments["/a"].recordId, bare.host.movieAssignments["/a"].posterId], ["", "", "", "tmdb:", ""]);
  for (const over of [{ detail: null }, { mediaKind: "tv" }]) {
    const blocked = harness({ over: { detail, ...over } });
    blocked.ren.assignCurrentMovie(["/a"]);
    assert.deepEqual(blocked.host.movieAssignments, {});
  }
  const empty = harness({ over: { detail } });
  empty.ren.assignCurrentMovie([]);
  assert.deepEqual(empty.host.movieAssignments, {});
});

test("the selected files are those of the visible organizer list, and toggling or selecting clears the preview", () => {
  const files = [file("/a"), file("/b", { multipleEpisodes: true }), file("/c")];
  const h = harness({ over: { organizerScan: { root: "/lib", renameRoot: "/lib", files, seasonDirectories: [], episodeGaps: [], errors: [], truncated: false }, selectedMediaPaths: ["/a", "/zzz"], renamePreview: { previewId: "p" }, confirmRename: true }, visible: [files[0], files[2]] });
  assert.deepEqual(h.ren.selectedPathsInOrganizer(), ["/a"]);
  assert.deepEqual(harness({ over: { organizerScan: null, selectedMediaPaths: ["/a"] } }).ren.selectedPathsInOrganizer(), []);
  h.ren.toggleMedia("/c", true);
  assert.deepEqual([h.host.selectedMediaPaths, h.host.renamePreview], [["/a", "/zzz", "/c"], null]);
  h.host.renamePreview = { previewId: "p" };
  h.ren.toggleMedia("/c", true);
  assert.deepEqual(h.host.selectedMediaPaths, ["/a", "/zzz", "/c"]);
  h.ren.toggleMedia("/a", false);
  assert.deepEqual(h.host.selectedMediaPaths, ["/zzz", "/c"]);
  const all = harness({ over: { organizerScan: { root: "/lib", renameRoot: "/lib", files, seasonDirectories: [], episodeGaps: [], errors: [], truncated: false }, selectedMediaPaths: ["/zzz"], renamePreview: { previewId: "p" }, confirmRename: true } });
  all.ren.selectAllMedia(true);
  assert.deepEqual([all.host.selectedMediaPaths, all.host.renamePreview, all.host.confirmRename], [["/zzz", "/a", "/c"], null, false]);
  all.ren.selectAllMedia(false);
  assert.deepEqual(all.host.selectedMediaPaths, ["/zzz"]);
  const compact = harness({ over: { organizerScan: { root: "/lib", renameRoot: "/lib", files, seasonDirectories: [], episodeGaps: [], errors: [], truncated: false }, compactItemMode: true }, visible: [files[0]] });
  compact.ren.selectAllMedia(true);
  assert.deepEqual(compact.host.selectedMediaPaths, ["/a"]);
  const none = harness({ over: { organizerScan: null, selectedMediaPaths: ["/x"] } });
  none.ren.selectAllMedia(true);
  assert.deepEqual(none.host.selectedMediaPaths, ["/x"]);
});

test("organizer templates and metadata-only copies go to the callbacks", () => {
  const h = harness({ over: { selectedMediaPaths: ["/lib/Show/e1.mkv", "/gone"] } });
  h.ren.saveOrganizerTemplates();
  const saved = h.calls[0].args;
  assert.deepEqual(saved, templates);
  assert.notEqual(saved, h.host.organizerTemplates);
  h.ren.createMetadataOnlyCopies();
  assert.deepEqual(h.calls[1], { command: "onStartMetadataOnly", args: ["/lib/Show/e1.mkv"] });
  const empty = harness();
  empty.ren.createMetadataOnlyCopies();
  assert.equal(empty.calls.length, 0);
});

test("compactRenameReady needs compact mode, a selection and a reviewed film or episodes", () => {
  const movie = { compactItemMode: true, selectedMediaPaths: ["/lib/Show/e1.mkv"], movieAssignments: { "/lib/Show/e1.mkv": { title: "x" } } };
  assert.equal(harness({ over: movie }).ren.compactRenameReady(), true);
  assert.equal(harness({ over: { ...movie, movieAssignments: {} } }).ren.compactRenameReady(), false);
  assert.equal(harness({ over: { ...movie, compactItemMode: false } }).ren.compactRenameReady(), false);
  assert.equal(harness({ over: { ...movie, selectedMediaPaths: [] } }).ren.compactRenameReady(), false);
  const tv = tvState({ compactItemMode: true });
  assert.equal(harness({ over: tv }).ren.compactRenameReady(), true);
  assert.equal(harness({ over: { ...tv, episodeAssignments: { "/lib/Show/e1.mkv": "1/1" } } }).ren.compactRenameReady(), false);
  assert.equal(harness({ over: { ...tv, detail: null } }).ren.compactRenameReady(), false);
  assert.equal(harness({ over: { ...tv, selected: null } }).ren.compactRenameReady(), false);
});

test("buildRenameRequest for films checks the folder, the selection and each reviewed film", () => {
  const assignment = { title: "Heat", year: "1995", certification: "R" };
  const h = harness({ over: { selectedMediaPaths: ["/lib/Show/e1.mkv"], movieAssignments: { "/lib/Show/e1.mkv": assignment } } });
  assert.deepEqual(h.ren.buildRenameRequest(), { root: "/lib/Show", items: [{ sourcePath: "/lib/Show/e1.mkv", kind: "movie", template: "{title}{ext}", values: { title: "Heat", year: "1995", certification: "R" } }], textBlacklistPatterns: ["rip*"] });
  assert.equal(throwsMessage(() => harness({ over: { organizerScan: null } }).ren.buildRenameRequest()), "Choose a media folder first.");
  assert.equal(throwsMessage(() => harness().ren.buildRenameRequest()), "Select at least one movie file for this reviewed title.");
  assert.equal(throwsMessage(() => harness({ over: { selectedMediaPaths: ["/lib/Show/e1.mkv"] } }).ren.buildRenameRequest()), "Assign a reviewed film to e1.mkv before previewing or applying this batch.");
});

test("buildRenameRequest for series needs a reviewed series and a confirmed year", () => {
  assert.equal(throwsMessage(() => harness({ over: tvState({ detail: null }) }).ren.buildRenameRequest()), "Search for and review a TV series before planning its rename.");
  assert.equal(throwsMessage(() => harness({ over: tvState({ selected: null }) }).ren.buildRenameRequest()), "Search for and review a TV series before planning its rename.");
  const mismatch = { text: "The year 2015 does not match", clue: { title: "Show" } };
  assert.equal(throwsMessage(() => harness({ over: tvState({ tvRenameScope: "episodes-only" }), mismatch }).ren.buildRenameRequest()), "The year 2015 does not match. Confirm that this is the correct series before preparing a rename.");
  const confirmed = harness({ over: tvState({ tvRenameScope: "episodes-only", seriesYearConflictConfirmed: true }), mismatch });
  assert.equal(confirmed.ren.buildRenameRequest().items.length, 2);
});

test("a full-series rename needs a complete folder scan with every episode selected", () => {
  const base = tvState();
  const build = (over) => throwsMessage(() => harness({ over: { ...base, ...over } }).ren.buildRenameRequest());
  assert.equal(build({ organizerSource: "analysis" }), "The current library analysis may exclude files below its size filter. Scan this series folder before planning a full-series rename.");
  const scan = (over) => ({ root: "/lib/Show", renameRoot: "/lib", files: [file("/lib/Show/e1.mkv", { season: 1, episode: 1 }), file("/lib/Show/e2.mkv", { season: 1, episode: 2 })], seasonDirectories: [{ path: "/lib/Show/Season 1", name: "Season 1", season: 1 }], episodeGaps: [], errors: [], truncated: false, ...over });
  assert.equal(build({ organizerScan: scan({ truncated: true }) }), "Folder analysis reached its 10,000-file safety limit. Narrow the selected series folder before a full-series rename.");
  assert.equal(build({ organizerScan: scan({ errors: ["a", "b"] }) }), "Folder analysis found 2 unreadable entries. Resolve these before a full-series rename.");
  assert.equal(build({ selectedMediaPaths: ["/lib/Show/e1.mkv"] }), "1 supported episode file(s) are not selected. Select all supported episodes or choose “Selected episode files only.”");
  const multi = scan({ files: [file("/lib/Show/e1.mkv", { season: 1, episode: 1 }), file("/lib/Show/e2.mkv", { season: 1, episode: 2, multipleEpisodes: true })] });
  assert.equal(build({ organizerScan: multi, selectedMediaPaths: ["/lib/Show/e1.mkv"] }), null);
  const ok = harness({ over: base }).ren.buildRenameRequest();
  assert.equal(ok.root, "/lib");
  assert.deepEqual(ok.items.map((item) => [item.kind, item.sourcePath]), [["series", "/lib/Show"], ["season", "/lib/Show/Season 1"], ["episode", "/lib/Show/e1.mkv"], ["episode", "/lib/Show/e2.mkv"]]);
  assert.deepEqual(ok.items[0].values, { series_title: "Show", first_year: "2008", last_year: "2013", year_range: "2008-2013" });
  assert.deepEqual(ok.items[1].values, { season: "1" });
  assert.deepEqual(ok.items[2].values, { series_title: "Show", season: "1", episode: "1", episode_title: "Pilot" });
  assert.equal(ok.items[0].template, "{series_title}");
  const compact = harness({ over: { ...base, compactItemMode: true, organizerSource: "analysis" } }).ren.buildRenameRequest();
  assert.deepEqual([compact.root, compact.items.map((item) => item.kind)], ["/lib/Show", ["episode", "episode"]]);
});

test("a selected-episodes rename needs a selection and checks each episode", () => {
  const base = tvState({ tvRenameScope: "episodes-only" });
  const build = (over) => throwsMessage(() => harness({ over: { ...base, ...over } }).ren.buildRenameRequest());
  assert.equal(build({ selectedMediaPaths: [] }), "Select one or more episode files, or choose the full-series rename scope.");
  assert.equal(build({ selectedMediaPaths: ["/lib/Show/missing.mkv"] }), "Select one or more episode files, or choose the full-series rename scope.");
  assert.equal(build({ selectedMediaPaths: ["/lib/Show/missing.mkv"], visibleOrganizerFiles: [file("/lib/Show/missing.mkv")] }), "A selected episode is no longer present in the folder scan. Scan the folder again.");
  const multi = { files: [file("/lib/Show/e1.mkv", { season: 1, episode: 1, multipleEpisodes: true })], root: "/lib/Show", renameRoot: "/lib", seasonDirectories: [], episodeGaps: [], errors: [], truncated: false };
  assert.equal(build({ organizerScan: multi, selectedMediaPaths: ["/lib/Show/e1.mkv"] }), "e1.mkv contains multiple episode numbers and cannot be safely assigned to one episode filename. Handle it separately.");
  assert.equal(build({ episodeAssignments: {} }), "Choose and review a provider episode for e1.mkv before previewing or applying the rename.");
  assert.equal(build({ episodeAssignments: { "/lib/Show/e1.mkv": "5/5", "/lib/Show/e2.mkv": "1/2" } }), "Choose and review a provider episode for e1.mkv before previewing or applying the rename.");
  const ok = harness({ over: { ...base, selectedMediaPaths: ["/lib/Show/e2.mkv"] } }).ren.buildRenameRequest();
  assert.deepEqual([ok.root, ok.items.map((item) => item.values.episode_title)], ["/lib/Show", ["Second"]]);
});

test("previewNames asks the engine for the rename preview and reports a refusal", async () => {
  const preview = { previewId: "pv", root: "/lib", canApply: true, items: [], conflicts: [] };
  const h = harness({ payloads: { preview_library_rename: preview }, over: { selectedMediaPaths: ["/lib/Show/e1.mkv"], movieAssignments: { "/lib/Show/e1.mkv": { title: "Heat", year: "1995", certification: "" } }, renameError: "old", renameMessage: "msg", confirmRename: true } });
  const run = h.ren.previewNames();
  assert.deepEqual([h.host.renameBusy, h.host.renameError, h.host.renameMessage, h.host.confirmRename], [true, "", "", false]);
  await run;
  assert.equal(h.calls[0].command, "preview_library_rename");
  assert.deepEqual([h.calls[0].args.root, h.calls[0].args.items[0].sourcePath, h.host.renamePreview, h.host.renameBusy], ["/lib/Show", "/lib/Show/e1.mkv", preview, false]);
  const refused = harness({ over: { renamePreview: { previewId: "old" } } });
  await refused.ren.previewNames();
  assert.deepEqual([refused.host.renamePreview, refused.host.renameError, refused.host.renameBusy, refused.calls.length], [null, "Error: Select at least one movie file for this reviewed title.", false, 0]);
  const failing = harness({ payloads: { preview_library_rename: new Error("conflict") }, over: { selectedMediaPaths: ["/lib/Show/e1.mkv"], movieAssignments: { "/lib/Show/e1.mkv": { title: "Heat", year: "", certification: "" } } } });
  await failing.ren.previewNames();
  assert.deepEqual([failing.host.renameError, failing.host.renamePreview], ["Error: conflict", null]);
  for (const blocked of [harness({ desktop: false }), harness({ over: { organizerScan: null } })]) {
    await blocked.ren.previewNames();
    assert.deepEqual([blocked.calls.length, blocked.host.renameBusy], [0, false]);
  }
});

const applyPreview = (over = {}) => ({ previewId: "pv", root: "/lib", canApply: true, conflicts: [], items: [
  { sourcePath: "/lib/Show/e1.mkv", targetPath: "/lib/Show/Show S01E01.mkv", targetName: "Show S01E01.mkv", isDirectory: false, unchanged: false },
  { sourcePath: "/lib/Show", targetPath: "/lib/Show (2008)", targetName: "Show (2008)", isDirectory: true, unchanged: false },
  { sourcePath: "/lib/Show/e2.mkv", targetPath: "/lib/Show/e2.mkv", targetName: "e2.mkv", isDirectory: false, unchanged: true },
], ...over });

test("applyNames renames, moves the assignments, rescans the folder and reports each step", async () => {
  const rescanned = { root: "/lib/Show (2008)", renameRoot: "/lib/Show (2008)", files: [file("/lib/Show/Show S01E01.mkv", { season: 1, episode: 1 }), file("/lib/Show/fresh.mkv", { season: 1, episode: 3 }), file("/lib/Show/new2.mkv", { season: 1, episode: 2 })], seasonDirectories: [], episodeGaps: [], errors: [], truncated: false };
  const h = harness({ payloads: { apply_library_rename: null, scan_organizer_folder: rescanned }, over: { ...tvState({ tvRenameScope: "episodes-only" }), renamePreview: applyPreview(), renameError: "old", confirmRename: true } });
  const run = h.ren.applyNames();
  assert.deepEqual([h.host.renameBusy, h.host.renameError], [true, ""]);
  await run;
  const names = h.calls.map((call) => call.command);
  assert.deepEqual(names, ["onRenameState", "apply_library_rename", "onPathsRenamed", "onRenameState", "scan_organizer_folder"]);
  assert.deepEqual(h.calls[0].args, [["/lib/Show/e1.mkv"], "renaming", "Applying the reviewed filename changes."]);
  assert.deepEqual([h.calls[1].args.previewId, h.calls[1].args.confirmed, h.calls[1].args.items.length, h.calls[1].args.root], ["pv", true, 2, "/lib/Show"]);
  assert.deepEqual(h.calls[2].args, { "/lib/Show/e1.mkv": "/lib/Show/Show S01E01.mkv", "/lib/Show": "/lib/Show (2008)", "/lib/Show/e2.mkv": "/lib/Show/e2.mkv" });
  assert.deepEqual(h.calls[3].args, [["/lib/Show/Show S01E01.mkv", "/lib/Show (2008)"], "renamed", "Renamed 2 paths."]);
  assert.deepEqual(h.calls[4].args, { root: "/lib/Show (2008)" });
  assert.deepEqual(h.host.selectedMediaPaths, ["/lib/Show/Show S01E01.mkv", "/lib/Show/e2.mkv"]);
  assert.deepEqual(h.host.episodeAssignments, { "/lib/Show/Show S01E01.mkv": "1/1", "/lib/Show/e2.mkv": "1/2", "/lib/Show/new2.mkv": "1/2" });
  assert.deepEqual(h.host.organizerScan, rescanned);
  assert.deepEqual([h.host.renamePreview, h.host.confirmRename, h.host.renameBusy, h.host.renameMessage], [null, false, false, "Renamed 2 paths. Media contents were not changed."]);
});

test("applyNames moves film assignments, and reports singular renames", async () => {
  const preview = applyPreview({ items: [{ sourcePath: "/lib/Show/e1.mkv", targetPath: "/lib/Show/Heat.mkv", targetName: "Heat.mkv", isDirectory: false, unchanged: false }, { sourcePath: "/lib/Show/e2.mkv", targetPath: "/lib/Show/e2.mkv", targetName: "e2.mkv", isDirectory: false, unchanged: true }] });
  const rescanned = { root: "/lib/Show", renameRoot: "/lib/Show", files: [], seasonDirectories: [], episodeGaps: [], errors: [], truncated: false };
  const h = harness({ payloads: { apply_library_rename: null, scan_organizer_folder: rescanned }, over: { selectedMediaPaths: ["/lib/Show/e1.mkv"], movieAssignments: { "/lib/Show/e1.mkv": { title: "Heat", year: "1995", certification: "" }, "/lib/Show/e2.mkv": { title: "Same" }, "/other": { title: "keep" } }, renamePreview: preview } });
  await h.ren.applyNames();
  assert.deepEqual(Object.keys(h.host.movieAssignments).sort(), ["/lib/Show/Heat.mkv", "/lib/Show/e2.mkv", "/other"]);
  assert.equal(h.host.renameMessage, "Renamed 1 path. Media contents were not changed.");
  assert.deepEqual(h.calls.find((call) => call.command === "scan_organizer_folder").args, { root: "/lib/Show" });
  assert.equal(h.calls.some((call) => call.command === "sync"), false);
});

test("applyNames reports a rename that did not happen, and one whose follow-up refresh failed", async () => {
  const failing = harness({ payloads: { apply_library_rename: new Error("exists") }, over: { ...tvState({ tvRenameScope: "episodes-only" }), renamePreview: applyPreview() } });
  await failing.ren.applyNames();
  assert.deepEqual(failing.calls.filter((call) => call.command === "onRenameState").map((call) => call.args), [[["/lib/Show/e1.mkv"], "renaming", "Applying the reviewed filename changes."], [["/lib/Show/e1.mkv"], "failed", "Rename did not complete: Error: exists"]]);
  assert.deepEqual([failing.host.renameError, failing.host.renamePreview, failing.host.renameBusy, failing.calls.some((call) => call.command === "onPathsRenamed")], ["Error: exists", null, false, false]);
  const refresh = harness({ payloads: { apply_library_rename: null, scan_organizer_folder: new Error("scan failed") }, over: { ...tvState({ tvRenameScope: "episodes-only" }), renamePreview: applyPreview() } });
  await refresh.ren.applyNames();
  const states = refresh.calls.filter((call) => call.command === "onRenameState").map((call) => call.args);
  assert.deepEqual(states.at(-1), [["/lib/Show/Show S01E01.mkv"], "renamed", "Files were renamed, but the follow-up refresh failed: Error: scan failed"]);
  assert.deepEqual([refresh.host.renameError, refresh.host.renamePreview, refresh.host.renameBusy], ["Error: scan failed", null, false]);
  const refused = harness({ over: { ...tvState({ detail: null }), renamePreview: applyPreview() } });
  await refused.ren.applyNames();
  assert.deepEqual(refused.calls.filter((call) => call.command === "onRenameState").map((call) => call.args[1]), ["renaming", "failed"]);
  for (const over of [{ renamePreview: null }, { renamePreview: applyPreview({ canApply: false }) }, { organizerScan: null, renamePreview: applyPreview() }]) {
    const blocked = harness({ over });
    await blocked.ren.applyNames();
    assert.deepEqual([blocked.calls.length, blocked.host.renameBusy], [0, false]);
  }
  const preview = harness({ desktop: false, over: { renamePreview: applyPreview() } });
  await preview.ren.applyNames();
  assert.equal(preview.calls.length, 0);
});
