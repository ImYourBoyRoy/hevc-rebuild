// ./desktop/tests/library-rename.test.mjs
// Characterization tests for the library rename and organizer module extracted from the route
// (rename slice): the rename dialog helpers, applyLibraryRename (preview, confirm, apply), the
// library task shortcuts, metadata-only organizer selection, remapping every piece of queue state
// after files are renamed, and the metadata review dialog open/close. The native bridge and the
// route's actions are injected fakes. Run: pnpm test:library-rename

import assert from "node:assert/strict";
import test from "node:test";
import { createLibraryRename } from "../src/lib/library-rename.ts";

const templates = { movie: "{title} ({year}){ext}", series: "{series_title}", season: "Season {season:02}", episode: "S{season:02}E{episode:02}.{episode_title}{ext}" };
const defaults = { movie: "{title}{ext}", series: "{series_title}", season: "Season {season:02}", episode: "S{season:02}E{episode:02}{ext}" };

const entry = (path, over = {}) => ({ path, name: path.split("/").at(-1), relative_path: path.replace("/lib/", ""), size_bytes: 1, status: "Ready", already_processed: false, error: null, profile: null, ...over });
const source = (root, entries, over = {}) => ({ root, scanResult: { root, input_kind: "folder", entries }, changedFiles: [], missingFiles: [], excludedPaths: [], entryOrder: [], ...over });

function harness({ invokeImpl = {}, hostOver = {}, actionOver = {} } = {}) {
  const log = [];
  const calls = [];
  const invokes = [];
  const film = entry("/lib/Film (2001).mkv");
  const ep1 = entry("/lib/Show/Season 1/S01E01.Pilot.mkv");
  const ep2 = entry("/lib/Show/Season 1/S01E02.Second.mkv");
  const dialog = { open: false, showModal() { this.open = true; calls.push(["showModal"]); }, close() { this.open = false; calls.push(["closeDialog"]); } };
  const state = {
    renameDraft: { ...templates }, libraryRenameError: "", libraryRenameBusy: false, libraryRenameOpen: false, libraryRenameDialog: dialog,
    metadataFocusPath: "", libraryTvReview: null, metadataDialogOpen: false, metadataDialogElement: null, mode: "check", root: "/lib",
    scanResult: null, queueMovieReviews: {}, queueSeriesReviews: {}, queueMatchSuggestions: {}, fileTuneOverrides: {}, fileStreamOverrides: {}, fileSettingsOverrides: {},
    queueArtworkPreviews: { a: "x" }, queueArtworkPreviewErrors: { a: "e" }, queueArtworkPreviewBusy: { a: true },
    appPreferences: { organizerTemplates: { ...templates }, fileOutputDirs: {} }, settings: null, workflowBusy: false,
    queuedEntries: [film, ep1, ep2], selectedPaths: [film.path], sources: [source("/lib", [film, ep1, ep2])], runStates: {}, activeRoot: "/lib",
    ...hostOver,
  };
  const host = state;
  host.setNotice = (message, kind = "info") => log.push([kind, message]);
  const invoke = async (command, args) => {
    invokes.push({ command, args });
    const impl = invokeImpl[command];
    if (impl === undefined) throw new Error(`unexpected ${command}`);
    return typeof impl === "function" ? impl(args) : impl;
  };
  const actions = {
    persistPreferences: async (patch) => { calls.push(["persistPreferences", patch]); return true; },
    persistQueueSources: async () => { calls.push(["persistQueueSources"]); },
    openPage: (page) => calls.push(["openPage", page]), beginRun: async () => { calls.push(["beginRun"]); }, tick: async () => {},
    ensureQueueArtworkPreview: async (media, art) => { calls.push(["preview", media, art]); },
    seriesGroupForRoot: (root) => (root === "/lib/Show" ? { root, episodePaths: [ep1.path, ep2.path] } : undefined),
    tvReviewForEpisode: (item) => state.tvForEpisode?.(item) ?? null,
    posterPathsForReview: (group, season) => [`${group.root}/poster-${season}.jpg`],
    reviewedSeriesForPath: (path) => state.seriesReviews?.[path] ?? Object.values(state.queueSeriesReviews).find((review) => path.startsWith(review.root)),
    ...actionOver,
  };
  const lib = createLibraryRename({ invoke, host, actions, constants: { defaultOrganizerTemplates: defaults } });
  return { lib, host, log, calls, invokes, film, ep1, ep2, dialog };
}

const movieReview = (over = {}) => ({ title: "Film", year: "2001", certification: "PG", localPosterPath: "/lib/poster.jpg", artworkCandidates: [], ...over });
const seriesReview = (over = {}) => ({ root: "/lib/Show", title: "Show", year: "2020", certification: "", localPosterPath: "/lib/Show/poster.jpg", localSeasonPosterPaths: { 1: "/lib/Show/Season 1/poster.jpg" }, episodeAssignments: {}, ...over });

test("libraryRenameEntries returns the selected queued files and libraryRenameShape classifies them", () => {
  const h = harness({ hostOver: { selectedPaths: ["/lib/Film (2001).mkv", "/lib/Show/Season 1/S01E01.Pilot.mkv"] } });
  assert.deepEqual(h.lib.libraryRenameEntries().map((item) => item.name), ["Film (2001).mkv", "S01E01.Pilot.mkv"]);
  assert.equal(h.lib.libraryRenameShape(), "episode");
  h.host.queueMovieReviews = { [h.film.path]: movieReview() };
  assert.equal(h.lib.libraryRenameShape(), "both");
  h.host.selectedPaths = [h.film.path];
  assert.equal(h.lib.libraryRenameShape(), "movie");
  h.host.selectedPaths = [];
  assert.equal(h.lib.libraryRenameShape(), "episode");
});

test("insertRenameToken edits the draft at the caret, clears the error and restores focus", async () => {
  const h = harness();
  h.host.libraryRenameError = "old";
  const field = { selectionStart: 3, selectionEnd: 3, focused: 0, caret: null, focus() { this.focused += 1; }, setSelectionRange(a) { this.caret = a; } };
  h.lib.insertRenameToken("movie", "{year}", field);
  assert.deepEqual([h.host.renameDraft.movie, h.host.libraryRenameError], ["{ti{year}tle} ({year}){ext}", ""]);
  await new Promise((resolve) => queueMicrotask(resolve));
  assert.deepEqual([field.focused, field.caret], [1, 9]);
  h.lib.insertRenameToken("episode", "{ext}", null);
  assert.equal(h.host.renameDraft.episode.endsWith("{ext}{ext}"), true);
});

test("resetRenamePattern restores the standard pattern for one kind and clears the error", () => {
  const h = harness();
  h.host.libraryRenameError = "old";
  h.lib.resetRenamePattern("movie");
  assert.deepEqual([h.host.renameDraft.movie, h.host.renameDraft.episode, h.host.libraryRenameError], [defaults.movie, templates.episode, ""]);
});

test("libraryRenameNote says whether the name changes, is suggested or stays", () => {
  const h = harness();
  h.host.queueMovieReviews = { [h.film.path]: movieReview({ title: "Reviewed", year: "2001", certification: "" }) };
  const changed = h.lib.libraryRenameNote(h.film);
  assert.deepEqual([changed.next, changed.note], ["Reviewed (2001).mkv", "New name"]);
  assert.deepEqual(h.lib.libraryRenameNote(h.film, { ...templates, movie: "Film (2001){ext}" }), { next: null, note: "Name stays the same" });
  const plain = harness();
  assert.deepEqual(plain.lib.libraryRenameNote(plain.film).note, "Name stays the same");
});

const previewOk = (items) => ({ previewId: "pv1", canApply: true, conflicts: [], items });

test("applyLibraryRename previews, confirms, applies, remaps the queue and reports the count", async () => {
  const h = harness({
    hostOver: { selectedPaths: ["/lib/Film (2001).mkv"], queueMovieReviews: { "/lib/Film (2001).mkv": movieReview() }, settings: { metadata: { text_blacklist: ["rip"] } }, libraryRenameOpen: true },
    invokeImpl: {
      preview_library_rename: (args) => previewOk(args.items.map((item) => ({ sourcePath: item.sourcePath, targetPath: "/lib/Film.mkv", unchanged: false }))),
      apply_library_rename: null,
    },
  });
  await h.lib.applyLibraryRename();
  assert.deepEqual(h.invokes[0], { command: "preview_library_rename", args: { root: "/lib", items: [{ sourcePath: "/lib/Film (2001).mkv", kind: "movie", template: templates.movie, values: { title: "Film", year: "2001", certification: "PG" } }], textBlacklistPatterns: ["rip"] } });
  assert.deepEqual(h.invokes[1].args.previewId, "pv1");
  assert.equal(h.invokes[1].args.confirmed, true);
  assert.deepEqual([h.host.libraryRenameOpen, h.host.libraryRenameBusy, h.host.libraryRenameError], [false, false, ""]);
  assert.deepEqual(h.log, [["success", "Renamed 1 file. The video itself was not changed."]]);
  assert.deepEqual(h.host.selectedPaths, ["/lib/Film.mkv"]);
  assert.equal(h.host.sources[0].scanResult.entries[0].name, "Film.mkv");
});

test("applyLibraryRename persists changed templates first and pluralizes the notice", async () => {
  const h = harness({
    hostOver: { selectedPaths: ["/lib/Film (2001).mkv", "/lib/Show/Season 1/S01E01.Pilot.mkv"], queueMovieReviews: { "/lib/Film (2001).mkv": movieReview() }, renameDraft: { ...templates, movie: "{title}{ext}" },
      queueSeriesReviews: { "/lib/Show": seriesReview({ episodeAssignments: { "/lib/Show/Season 1/S01E01.Pilot.mkv": { season: 1, episode: 1, title: "Pilot" } } }) } },
    invokeImpl: { preview_library_rename: (args) => previewOk(args.items.map((item) => ({ sourcePath: item.sourcePath, targetPath: `${item.sourcePath}.new`, unchanged: item.kind === "movie" }))), apply_library_rename: null },
  });
  await h.lib.applyLibraryRename();
  assert.deepEqual(h.calls.find((call) => call[0] === "persistPreferences"), ["persistPreferences", { organizerTemplates: { ...templates, movie: "{title}{ext}" } }]);
  assert.deepEqual(h.invokes[0].args.items.map((item) => item.kind), ["movie", "episode"]);
  assert.deepEqual(h.invokes[0].args.items[1].values, { series_title: "Show", season: "1", episode: "1", episode_title: "Pilot" });
  assert.deepEqual(h.log.at(-1), ["success", "Renamed 1 file. The video itself was not changed."]);
});

test("applyLibraryRename refuses an empty selection, several folders and files without confirmed titles", async () => {
  const none = harness({ hostOver: { selectedPaths: [] } });
  await none.lib.applyLibraryRename();
  assert.equal(none.host.libraryRenameError, "Select the files to rename.");
  const second = entry("/other/B.mkv");
  const two = harness({ hostOver: { selectedPaths: ["/lib/Film (2001).mkv", "/other/B.mkv"], queuedEntries: [entry("/lib/Film (2001).mkv"), second], sources: [source("/lib", [entry("/lib/Film (2001).mkv")]), source("/other", [second])] } });
  await two.lib.applyLibraryRename();
  assert.equal(two.host.libraryRenameError, "Rename one folder at a time.");
  const untitled = harness({ hostOver: { selectedPaths: ["/lib/Film (2001).mkv"] } });
  await untitled.lib.applyLibraryRename();
  assert.equal(untitled.host.libraryRenameError, "Confirm a title on each selected file before renaming. A file that already has the right name stays as it is.");
  assert.equal(untitled.invokes.length, 0);
  const busy = harness({ hostOver: { libraryRenameBusy: true, selectedPaths: [] } });
  await busy.lib.applyLibraryRename();
  assert.equal(busy.host.libraryRenameError, "");
});

test("applyLibraryRename finds an episode assignment by an equivalent path and skips incomplete ones", async () => {
  const h = harness({
    hostOver: { selectedPaths: ["/lib/Show/Season 1/S01E01.Pilot.mkv", "/lib/Show/Season 1/S01E02.Second.mkv"], queueSeriesReviews: { "/lib/Show": seriesReview({ episodeAssignments: { "/lib/Show/Season 1/S01E01.Pilot.mkv": { season: 1, episode: 1, title: "Pilot" }, "/lib/Show/Season 1/S01E02.Second.mkv": { season: 1, episode: 2, title: "  " } } }) } },
    invokeImpl: { preview_library_rename: (args) => previewOk(args.items.map((item) => ({ sourcePath: item.sourcePath, targetPath: item.sourcePath, unchanged: true }))), apply_library_rename: null },
  });
  await h.lib.applyLibraryRename();
  assert.equal(h.invokes[0].args.items.length, 1);
  assert.deepEqual(h.log.at(-1), ["success", "Renamed 0 files. The video itself was not changed."]);
});

test("applyLibraryRename shows the first conflict, a default message and any thrown error, and always clears busy", async () => {
  const base = { selectedPaths: ["/lib/Film (2001).mkv"], queueMovieReviews: { "/lib/Film (2001).mkv": movieReview() } };
  const conflict = harness({ hostOver: base, invokeImpl: { preview_library_rename: { previewId: "p", canApply: false, conflicts: [{ reason: "Target exists" }], items: [] } } });
  await conflict.lib.applyLibraryRename();
  assert.deepEqual([conflict.host.libraryRenameError, conflict.host.libraryRenameBusy], ["Target exists", false]);
  const same = harness({ hostOver: base, invokeImpl: { preview_library_rename: { previewId: "p", canApply: false, conflicts: [], items: [] } } });
  await same.lib.applyLibraryRename();
  assert.equal(same.host.libraryRenameError, "These files already use the proposed names.");
  const failed = harness({ hostOver: base, invokeImpl: { preview_library_rename: () => { throw new Error("backend down"); } } });
  await failed.lib.applyLibraryRename();
  assert.equal(failed.host.libraryRenameError, "backend down");
  const text = harness({ hostOver: base, invokeImpl: { preview_library_rename: () => { throw "plain failure"; } } });
  await text.lib.applyLibraryRename();
  assert.deepEqual([text.host.libraryRenameError, text.host.libraryRenameBusy], ["plain failure", false]);
});

test("chooseLibraryTask opens the rename dialog with the saved templates, or starts another task", async () => {
  const h = harness({ hostOver: { libraryRenameError: "old", renameDraft: { ...templates, movie: "x" } } });
  h.host.appPreferences.organizerTemplates = { ...templates, movie: "saved" };
  h.lib.chooseLibraryTask("organize");
  assert.deepEqual([h.host.libraryRenameError, h.host.renameDraft.movie, h.host.libraryRenameOpen], ["", "saved", true]);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(h.calls.some((call) => call[0] === "showModal"), true);
  h.dialog.open = true;
  const before = h.calls.filter((call) => call[0] === "showModal").length;
  h.lib.chooseLibraryTask("organize");
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(h.calls.filter((call) => call[0] === "showModal").length, before);
  for (const [task, expected] of [["metadata", "metadata"], ["integrity", "integrity"], ["rebuild", "full"]]) {
    const run = harness();
    run.lib.chooseLibraryTask(task);
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(run.host.mode, expected);
    assert.deepEqual(run.calls.map((call) => call[0]), ["openPage", "beginRun"]);
  }
  const busy = harness({ hostOver: { workflowBusy: true } });
  busy.lib.chooseLibraryTask("organize");
  busy.lib.chooseLibraryTask("rebuild");
  assert.deepEqual([busy.host.libraryRenameOpen, busy.host.mode, busy.calls.length], [false, "check", 0]);
});

test("a task shortcut does not start the run if the workspace became busy meanwhile", async () => {
  const h = harness();
  h.lib.chooseLibraryTask("rebuild");
  h.host.workflowBusy = true;
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(h.calls.map((call) => call[0]), ["openPage"]);
});

test("startMetadataOnlyFromOrganizer selects queued files for a metadata-only run", () => {
  const h = harness();
  h.lib.startMetadataOnlyFromOrganizer([h.film.path, h.film.path, h.ep1.path]);
  assert.deepEqual([h.host.selectedPaths, h.host.mode], [[h.film.path, h.ep1.path], "metadata"]);
  assert.deepEqual(h.calls, [["openPage", "run"]]);
  assert.deepEqual(h.log, [["success", "2 files selected for metadata-only MKV output. Review the destination and collision check before starting."]]);
  const one = harness();
  one.lib.startMetadataOnlyFromOrganizer([one.film.path]);
  assert.match(one.log[0][1], /^1 file selected/);
});

test("startMetadataOnlyFromOrganizer refuses unqueued files and a busy workspace", () => {
  const h = harness();
  h.lib.startMetadataOnlyFromOrganizer([h.film.path, "/elsewhere/new.mkv"]);
  assert.deepEqual(h.log, [["error", "Add these files to the Media Library queue and finish their scan before creating MKV copies."]]);
  assert.deepEqual([h.host.mode, h.host.selectedPaths], ["check", [h.film.path]]);
  const busy = harness({ hostOver: { workflowBusy: true } });
  busy.lib.startMetadataOnlyFromOrganizer([busy.film.path]);
  assert.deepEqual([busy.log.length, busy.calls.length], [0, 0]);
});

test("handleQueuePathsRenamed does nothing for an empty move set", () => {
  const h = harness();
  h.lib.handleQueuePathsRenamed({});
  assert.deepEqual(h.calls, []);
});

test("handleQueuePathsRenamed remaps sources, entries, selection and per-file overrides", () => {
  const h = harness({ hostOver: { fileTuneOverrides: { "/lib/Film (2001).mkv": "animation", "/lib/keep.mkv": "none" }, fileStreamOverrides: { "/lib/Film (2001).mkv": [1] }, fileSettingsOverrides: { "/lib/Film (2001).mkv": { crf: 20 } } } });
  h.host.sources = [source("/lib", [h.film, h.ep1], { changedFiles: ["/lib/Film (2001).mkv", "/lib/other.mkv"], missingFiles: ["/lib/Film (2001).mkv"], excludedPaths: ["/lib/Film (2001).mkv"], entryOrder: ["/lib/Film (2001).mkv", h.ep1.path] })];
  h.host.selectedPaths = ["/lib/Film (2001).mkv", h.ep1.path, "/lib/Film (2001).mkv"];
  h.lib.handleQueuePathsRenamed({ "/lib/Film (2001).mkv": "/lib/Film.mkv" });
  const moved = h.host.sources[0];
  assert.deepEqual(moved.scanResult.entries.map((item) => [item.path, item.name, item.relative_path]), [["/lib/Film.mkv", "Film.mkv", "Film.mkv"], [h.ep1.path, "S01E01.Pilot.mkv", "Show/Season 1/S01E01.Pilot.mkv"]]);
  assert.equal(moved.scanResult.entries[0].queue_stale, undefined);
  assert.deepEqual([moved.changedFiles, moved.missingFiles, moved.excludedPaths, moved.entryOrder], [["/lib/other.mkv"], [], ["/lib/Film.mkv"], ["/lib/Film.mkv", h.ep1.path]]);
  assert.deepEqual(h.host.selectedPaths, ["/lib/Film.mkv", h.ep1.path]);
  assert.deepEqual(h.host.fileTuneOverrides, { "/lib/keep.mkv": "none", "/lib/Film.mkv": "animation" });
  assert.deepEqual([h.host.fileStreamOverrides, h.host.fileSettingsOverrides], [{ "/lib/Film.mkv": [1] }, { "/lib/Film.mkv": { crf: 20 } }]);
});

test("handleQueuePathsRenamed remaps a renamed folder for roots, reviews, posters and run states", () => {
  const h = harness({
    hostOver: {
      queueMovieReviews: { "/lib/Film (2001).mkv": movieReview({ localPosterPath: "/lib/Film (2001).jpg" }), "/lib/blank.mkv": movieReview({ localPosterPath: "" }) },
      queueSeriesReviews: { "/lib/Show": seriesReview({ episodeAssignments: { [`/lib/Show/Season 1/S01E01.Pilot.mkv`]: { season: 1, episode: 1, title: "Pilot" } } }) },
      runStates: { "/lib/Show/Season 1/S01E01.Pilot.mkv": { status: "completed", mode: "full", message: "done", reportPath: "/r.csv", reportStatus: "ok" }, "/lib/Show/Season 1/S01E02.Second.mkv": { status: "queued", mode: "test", message: "waiting" }, "/lib/other.mkv": { status: "failed", mode: "full" } },
      queueMatchSuggestions: { "/lib/Show/Season 1/S01E01.Pilot.mkv": { status: "ready", title: "Pilot" } }, metadataFocusPath: "/lib/Show/Season 1/S01E01.Pilot.mkv", activeRoot: "/lib", root: "/lib", mode: "check",
      appPreferences: { organizerTemplates: { ...templates }, fileOutputDirs: { "/lib/Show/Season 1/S01E01.Pilot.mkv": "/out/a", "/lib/other.mkv": "/out/b" } },
    },
  });
  // Only the longest matching move applies to a path; moves do not chain.
  h.lib.handleQueuePathsRenamed({ "/lib/Show": "/lib/The Show", "/lib": "/media" });
  assert.deepEqual([h.host.root, h.host.activeRoot, h.host.metadataFocusPath], ["/media", "/media", "/lib/The Show/Season 1/S01E01.Pilot.mkv"]);
  assert.deepEqual(Object.keys(h.host.queueSeriesReviews), ["/lib/The Show"]);
  const review = h.host.queueSeriesReviews["/lib/The Show"];
  assert.deepEqual([review.localPosterPath, review.localSeasonPosterPaths[1], Object.keys(review.episodeAssignments)], ["/lib/The Show/poster.jpg", "/lib/The Show/Season 1/poster.jpg", ["/lib/The Show/Season 1/S01E01.Pilot.mkv"]]);
  assert.deepEqual(Object.keys(h.host.queueMovieReviews).sort(), ["/media/Film (2001).mkv", "/media/blank.mkv"]);
  assert.deepEqual([h.host.queueMovieReviews["/media/Film (2001).mkv"].localPosterPath, h.host.queueMovieReviews["/media/blank.mkv"].localPosterPath], ["/media/Film (2001).jpg", ""]);
  const states = h.host.runStates;
  assert.deepEqual(Object.keys(states).sort(), ["/lib/The Show/Season 1/S01E01.Pilot.mkv", "/lib/The Show/Season 1/S01E02.Second.mkv", "/media/other.mkv"]);
  assert.equal(states["/lib/The Show/Season 1/S01E01.Pilot.mkv"].status, "needs_review");
  assert.equal(states["/lib/The Show/Season 1/S01E01.Pilot.mkv"].message, "The media path changed after this run. Review the report and output before retrying.");
  assert.deepEqual([states["/lib/The Show/Season 1/S01E02.Second.mkv"].status, states["/lib/The Show/Season 1/S01E02.Second.mkv"].message], ["queued", "waiting"]);
  assert.deepEqual(Object.keys(h.host.queueMatchSuggestions), ["/lib/The Show/Season 1/S01E01.Pilot.mkv"]);
  assert.deepEqual([h.host.queueArtworkPreviews, h.host.queueArtworkPreviewErrors, h.host.queueArtworkPreviewBusy], [{}, {}, {}]);
});

test("handleQueuePathsRenamed refreshes the visible scan, saves the root and output folders and the queue", () => {
  const h = harness({ hostOver: { scanResult: { root: "/lib", entries: [] }, appPreferences: { organizerTemplates: { ...templates }, fileOutputDirs: { "/lib/Film (2001).mkv": "/out/f" } } } });
  h.lib.handleQueuePathsRenamed({ "/lib/Film (2001).mkv": "/lib/Film.mkv" });
  assert.deepEqual(h.host.scanResult.entries.map((item) => item.path), ["/lib/Film.mkv", "/lib/Show/Season 1/S01E01.Pilot.mkv", "/lib/Show/Season 1/S01E02.Second.mkv"]);
  assert.deepEqual(h.calls.find((call) => call[0] === "persistPreferences"), ["persistPreferences", { root: "/lib", fileOutputDirs: { "/lib/Film.mkv": "/out/f" } }]);
  assert.equal(h.calls.some((call) => call[0] === "persistQueueSources"), true);
  const none = harness({ hostOver: { scanResult: null, activeRoot: null, root: "", metadataFocusPath: "" } });
  none.lib.handleQueuePathsRenamed({ "/lib/Film (2001).mkv": "/lib/Film.mkv" });
  assert.deepEqual([none.host.scanResult, none.host.activeRoot, none.host.root, none.host.metadataFocusPath], [null, null, "", ""]);
});

test("handleQueuePathsRenamed keeps a single-file source's relative path as the file name", () => {
  const single = { root: "/lib/Film (2001).mkv", scanResult: { root: "/lib/Film (2001).mkv", input_kind: "file", entries: [entry("/lib/Film (2001).mkv")] }, changedFiles: [], missingFiles: [], excludedPaths: undefined, entryOrder: undefined };
  const h = harness({ hostOver: { sources: [single], activeRoot: single.root } });
  h.lib.handleQueuePathsRenamed({ "/lib/Film (2001).mkv": "/lib/Film.mkv" });
  assert.deepEqual([h.host.sources[0].root, h.host.sources[0].scanResult.entries[0].relative_path, h.host.sources[0].excludedPaths, h.host.sources[0].entryOrder], ["/lib/Film.mkv", "Film.mkv", [], []]);
});

test("openMetadataForEntry selects the file, opens its review and loads season posters for an episode", async () => {
  const review = { level: "episode", season: 1, seriesRoot: "/lib/Show", episodePath: "/lib/Show/Season 1/S01E01.Pilot.mkv" };
  const h = harness({ hostOver: { selectedPaths: ["/lib/other.mkv"] } });
  h.host.tvForEpisode = () => review;
  const dialog = { open: false, showModal() { this.open = true; h.calls.push(["showModalMeta"]); } };
  h.host.metadataDialogElement = dialog;
  await h.lib.openMetadataForEntry(h.ep1);
  assert.deepEqual([h.host.selectedPaths, h.host.metadataFocusPath, h.host.libraryTvReview, h.host.metadataDialogOpen], [["/lib/other.mkv", h.ep1.path], h.ep1.path, review, true]);
  assert.deepEqual(h.calls.filter((call) => call[0] === "preview"), [["preview", h.ep1.path, "/lib/Show/poster-1.jpg"]]);
  assert.equal(dialog.open, true);
  h.calls.length = 0;
  await h.lib.openMetadataForEntry(h.ep1);
  assert.equal(h.calls.some((call) => call[0] === "showModalMeta"), false, "an already open dialog is not reopened");
  const film = harness();
  await film.lib.openMetadataForEntry(film.film);
  assert.deepEqual([film.host.libraryTvReview, film.host.metadataDialogOpen, film.calls.filter((call) => call[0] === "preview").length], [null, true, 0]);
  const noGroup = harness();
  noGroup.host.tvForEpisode = () => ({ level: "episode", season: 1, seriesRoot: "/unknown", episodePath: "x" });
  await noGroup.lib.openMetadataForEntry(noGroup.ep1);
  assert.equal(noGroup.calls.filter((call) => call[0] === "preview").length, 0);
});

test("closeMetadataDialog closes an open dialog, otherwise resets the review flags", () => {
  const h = harness({ hostOver: { metadataDialogOpen: true, libraryTvReview: { level: "series" } } });
  const dialog = { open: true, close() { this.open = false; h.calls.push(["closeMeta"]); } };
  h.host.metadataDialogElement = dialog;
  h.lib.closeMetadataDialog();
  assert.deepEqual([h.calls, h.host.metadataDialogOpen], [[["closeMeta"]], true]);
  h.host.metadataDialogElement = null;
  h.lib.closeMetadataDialog();
  assert.deepEqual([h.host.metadataDialogOpen, h.host.libraryTvReview], [false, null]);
});

test("openFullMetadataOrganizer closes the review dialog and opens the organizer page", () => {
  const h = harness({ hostOver: { metadataDialogOpen: true } });
  h.lib.openFullMetadataOrganizer();
  assert.deepEqual([h.host.metadataDialogOpen, h.calls.at(-1)], [false, ["openPage", "metadata"]]);
});
