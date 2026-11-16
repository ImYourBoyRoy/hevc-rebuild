// ./desktop/tests/match-lookup.test.mjs
// Characterization tests for the automatic match lookup runners extracted from MetadataWorkbench (T10.7 slice 8): looking up
// the analyzed films and series folders one by one (guards, nearby artwork check, which items are skipped, title cache,
// automatic association, per-item failures, progress and status reports, stopping early and the closing message).
// The provider calls and every dependent action are fakes. Run: pnpm test:match-lookup

import assert from "node:assert/strict";
import test from "node:test";
import { createMatchLookup } from "../src/lib/match-lookup.ts";
import { automaticMetadataMatch } from "../src/lib/metadata-match.ts";
import { candidatePreview } from "../src/lib/metadata-candidates.ts";

const art = (id) => ({ id, title: id, imageUrl: `https://img/${id}.jpg`, thumbnailUrl: "", sourceUrl: "" });
const result = (id, title, year, provider = "tmdb", kind = "movie") => ({ id, provider, kind, title, year, description: `${title} text`, rating: null, sourceUrl: `https://src/${id}`, confidence: 1, artwork: [art(`a${id}`)] });
const clue = (title, year = null) => ({ title, year, yearEnd: null, certification: null });
const file = (path) => ({ path, name: path.split("/").at(-1), parentPath: "/lib", extension: "mkv", season: null, episode: null, multipleEpisodes: false });
const group = (root, names, over = {}) => ({ root, files: names.map((name) => ({ ...file(`${root}/${name}`), name })), seasonDirectories: [{ path: `${root}/Season 1`, name: "Season 1", season: 1 }], ambiguous: false, ...over });
const ready = { status: "ready" };

function harness({ desktop = true, over = {}, clues = {}, responses = {}, ensure = async () => ({ root: "/lib" }), hydrate = async () => true, suggestions = {}, onSearch = () => {} } = {}) {
  const calls = [];
  const statuses = [];
  const matches = [];
  const host = {
    scanResult: null, organizerSource: "folder", organizerScan: { root: "/lib/scan" }, metadataSources: { tvdb: false }, tvdbCredentialStatus: { configured: false },
    movieAssignments: {}, seriesAssignments: {}, localAssetScanError: "", lookupBusy: false, lookupProgress: null, lookupMessage: "", lookupMessageScope: null,
    movieSuggestions: {}, seriesSuggestions: {}, cancelLookupRequested: false, lastLookupStopped: false, ...over,
  };
  const actions = {
    ensureLocalAssets: async (...args) => { calls.push(["ensureLocalAssets", ...args]); return ensure(...args); },
    filenameClue: async (name) => { calls.push(["filenameClue", name]); const value = clues[name]; if (value instanceof Error) throw value; return value ?? clue(name.replace(/\.[^.]+$/, "")); },
    searchPublicTitles: async (kind, title) => { calls.push(["searchPublicTitles", kind, title]); onSearch(kind, title); const value = responses[title]; if (value instanceof Error) throw value; return value ?? { results: [], warnings: [], successfulProviders: [] }; },
    rankByFilenameYear: (rows, c, kind) => { calls.push(["rank", kind]); return [...rows]; },
    autoAssignMovieMatch: (path, c, candidate) => { calls.push(["autoAssignMovieMatch", path, candidate.id]); return true; },
    autoAssignSeriesMatch: (root, c, candidate) => { calls.push(["autoAssignSeriesMatch", root, candidate.id]); return true; },
    hydrateSeriesProvider: (root, row) => { calls.push(["hydrate", root, row.id]); return Promise.resolve(hydrate(root, row)); },
    suggestionForPath: (path) => suggestions[path],
    onLookupStatus: (status) => statuses.push(status),
    onMatchSuggestion: (path, suggestion) => matches.push([path, suggestion]),
  };
  return { lookup: createMatchLookup({ isDesktop: () => desktop, host, actions }), host, calls, statuses, matches };
}
const names = (h, command) => h.calls.filter((call) => call[0] === command).map((call) => call.slice(1));
const tick = () => new Promise((resolve) => setImmediate(resolve));

test("findMovieMatches does nothing when it cannot start", async () => {
  const files = [file("/lib/A.mkv")];
  const preview = harness({ desktop: false });
  await preview.lookup.findMovieMatches(files, "/lib");
  const busy = harness({ over: { lookupBusy: true } });
  await busy.lookup.findMovieMatches(files, "/lib");
  const empty = harness();
  await empty.lookup.findMovieMatches([], "/lib");
  const noRoot = harness({ over: { organizerScan: null } });
  await noRoot.lookup.findMovieMatches(files);
  assert.deepEqual([preview.calls.length, busy.calls.length, empty.calls.length, noRoot.calls.length, noRoot.host.lookupBusy, noRoot.statuses.length], [0, 0, 0, 0, false, 0]);
});

test("findMovieMatches picks its asset root from the argument, the analysis or the organizer scan", async () => {
  const files = [file("/lib/A.mkv")];
  const given = harness({ over: { scanResult: { root: "/x", input_kind: "folder" } } });
  await given.lookup.findMovieMatches(files, "/given");
  const analysis = harness({ over: { scanResult: { root: "C:\\media", input_kind: "folder" }, organizerSource: "analysis" } });
  await analysis.lookup.findMovieMatches(files);
  const scan = harness();
  await scan.lookup.findMovieMatches(files);
  assert.deepEqual([names(given, "ensureLocalAssets")[0][0], names(analysis, "ensureLocalAssets")[0][0], names(scan, "ensureLocalAssets")[0][0]], ["/given", "C:/media", "/lib/scan"]);
  assert.deepEqual(names(scan, "ensureLocalAssets")[0].slice(1), [files, ["/lib/scan"]]);
});

test("findMovieMatches reports a failed nearby-artwork check and stops", async () => {
  const files = [file("/lib/A.mkv"), file("/lib/B.mkv")];
  const h = harness({ ensure: async () => null, over: { localAssetScanError: "denied", cancelLookupRequested: true, lastLookupStopped: true } });
  await h.lookup.findMovieMatches(files, "/lib");
  assert.deepEqual([h.host.lookupBusy, h.host.lookupProgress, h.host.lookupMessage, h.host.lookupMessageScope, h.host.cancelLookupRequested, h.host.lastLookupStopped], [false, null, "Could not check nearby artwork: denied", "analyzed films", false, false]);
  assert.deepEqual(h.statuses, [
    { status: "searching", done: 0, total: 2, label: "films", message: "Checking nearby poster art before public searches.", sourceRoot: "/lib" },
    { status: "error", done: 0, total: 2, label: "films", message: "Could not check nearby artwork: denied", sourceRoot: "/lib" },
  ]);
  const generic = harness({ ensure: async () => null });
  await generic.lookup.findMovieMatches(files, "/lib");
  assert.equal(generic.host.lookupMessage, "Could not check nearby artwork: local asset scan failed");
  assert.equal(names(h, "filenameClue").length, 0);
});

test("findMovieMatches skips films with saved results only when the queue already analyzed them", async () => {
  const files = [file("/lib/A.mkv"), file("/lib/B.mkv"), file("/lib/C.mkv"), { path: "", name: "x.mkv" }, { path: "/lib/D.mkv", name: "" }];
  const state = { over: { movieAssignments: { "/lib/B.mkv": { recordId: "tmdb:1" } } }, suggestions: { "/lib/C.mkv": ready } };
  const queued = harness(state);
  await queued.lookup.findMovieMatches(files, "/lib");
  assert.deepEqual(names(queued, "filenameClue"), [["A.mkv"]]);
  const folder = harness(state);
  await folder.lookup.findMovieMatches(files);
  assert.deepEqual(names(folder, "filenameClue"), [["A.mkv"], ["B.mkv"], ["C.mkv"]]);
});

test("findMovieMatches explains when there is nothing left to search", async () => {
  const files = [file("/lib/B.mkv"), file("/lib/C.mkv")];
  const covered = harness({ over: { movieAssignments: { "/lib/B.mkv": { recordId: "tmdb:1" } } }, suggestions: { "/lib/C.mkv": ready } });
  await covered.lookup.findMovieMatches(files, "/lib");
  assert.deepEqual([covered.host.lookupMessage, covered.host.lookupMessageScope, covered.host.lookupBusy, covered.host.lookupProgress], ["Saved title suggestions or reviewed details are already available. Open a file to compare other sources.", "analyzed films", false, null]);
  assert.deepEqual(covered.statuses.at(-1), { status: "ready", done: 2, total: 2, label: "films", message: covered.host.lookupMessage, sourceRoot: "/lib" });
  const unnamed = harness();
  await unnamed.lookup.findMovieMatches([{ path: "", name: "" }], "/lib");
  assert.equal(unnamed.host.lookupMessage, "No movie files were available for a public search.");
});

test("findMovieMatches searches each distinct title once, associates exact matches and reports every film", async () => {
  const files = [file("/lib/Heat.1995.mkv"), file("/lib/heat.1995.copy.mkv"), file("/lib/Other.mkv")];
  const heat = result("1", "Heat", "1995");
  const clues = { "Heat.1995.mkv": clue("Heat", "1995"), "heat.1995.copy.mkv": clue("HEAT", "1995"), "Other.mkv": clue("Other") };
  const h = harness({ clues, responses: { Heat: { results: [heat], warnings: ["w"], successfulProviders: ["tmdb"] }, HEAT: { results: [heat], warnings: [], successfulProviders: [] }, Other: { results: [result("2", "Something", "2001")], warnings: [], successfulProviders: [] } } });
  await h.lookup.findMovieMatches(files, "/lib");
  assert.deepEqual(names(h, "searchPublicTitles"), [["movie", "Heat"], ["movie", "Other"]]);
  assert.deepEqual(names(h, "rank"), [["movie"], ["movie"], ["movie"]]);
  assert.deepEqual(names(h, "autoAssignMovieMatch"), [["/lib/Heat.1995.mkv", "1"], ["/lib/heat.1995.copy.mkv", "1"]]);
  assert.deepEqual(h.host.movieSuggestions["/lib/Heat.1995.mkv"], { clue: clue("Heat", "1995"), results: [heat], warnings: ["w"], status: "ready", error: "" });
  assert.equal(h.host.movieSuggestions["/lib/Other.mkv"].results[0].id, "2");
  const first = h.matches.filter(([path]) => path === "/lib/Heat.1995.mkv");
  assert.deepEqual(first[0][1], { kind: "movie", status: "searching", title: "Heat", year: "1995", provider: null, recordId: null, sourceUrl: null, description: null, autoAssociated: false, artwork: null, error: "" });
  assert.deepEqual(first[1][1], { kind: "movie", status: "ready", title: "Heat", year: "1995", provider: "tmdb", recordId: "tmdb:1", sourceUrl: "https://src/1", description: "Heat text", autoAssociated: true, artwork: candidatePreview(heat), error: "" });
  const other = h.matches.filter(([path]) => path === "/lib/Other.mkv")[1][1];
  assert.deepEqual([other.title, other.recordId, other.autoAssociated], ["Something", "tmdb:2", false]);
  assert.deepEqual(h.statuses.filter((status) => /^Checked \d+ of \d+ films\.$/.test(status.message)).map((status) => [status.done, status.message]), [[1, "Checked 1 of 3 films."], [2, "Checked 2 of 3 films."], [3, "Checked 3 of 3 films."]]);
  assert.deepEqual([h.host.lookupMessage, h.host.lookupBusy, h.host.lookupProgress, h.host.lastLookupStopped], ["Checked 3 film files. Exact title-and-year matches without a conflict were associated; review the rest.", false, null, false]);
  assert.deepEqual(h.statuses.at(-1), { status: "ready", done: 3, total: 3, label: "films", message: h.host.lookupMessage, sourceRoot: "/lib" });
  assert.equal(automaticMetadataMatch([heat], { ...clue("Heat", "1995"), kind: "movie" }), heat);
});

test("findMovieMatches records a failed film, keeps going and only reports an error when every film failed", async () => {
  const files = [file("/lib/A.mkv"), file("/lib/B.mkv")];
  const mixed = harness({ clues: { "A.mkv": new Error("engine") }, responses: { B: { results: [], warnings: [], successfulProviders: [] } } });
  await mixed.lookup.findMovieMatches(files, "/lib");
  assert.deepEqual(mixed.host.movieSuggestions["/lib/A.mkv"], { clue: { title: "A.mkv", year: null, yearEnd: null, certification: null }, results: [], warnings: [], status: "error", error: "Error: engine" });
  assert.equal(mixed.matches.find(([path, s]) => path === "/lib/A.mkv" && s.status === "error")[1].error, "Error: engine");
  assert.equal(mixed.host.lookupMessage, "Checked 2 film files; 1 lookup failed. Exact title-and-year matches without a conflict were associated; review the rest.");
  assert.equal(mixed.statuses.at(-1).status, "ready");
  const searchFails = harness({ responses: { A: new Error("429"), B: new Error("429") } });
  await searchFails.lookup.findMovieMatches(files, "/lib");
  assert.equal(searchFails.host.movieSuggestions["/lib/A.mkv"].clue.title, "A");
  assert.deepEqual([searchFails.host.lookupMessage.includes("2 lookups failed"), searchFails.statuses.at(-1).status], [true, "error"]);
  const single = harness({ responses: { A: new Error("x") } });
  await single.lookup.findMovieMatches([file("/lib/A.mkv")], "/lib");
  assert.equal(single.host.lookupMessage.startsWith("Checked 1 film file; 1 lookup failed."), true);
});

test("findMovieMatches stops when asked and says how far it got", async () => {
  const files = [file("/lib/A.mkv"), file("/lib/B.mkv"), file("/lib/C.mkv")];
  let h;
  h = harness({ onSearch: () => h.lookup.stopFilenameLookup() });
  await h.lookup.findMovieMatches(files, "/lib");
  assert.deepEqual(names(h, "filenameClue"), [["A.mkv"]]);
  assert.equal(h.host.lastLookupStopped, true);
  assert.equal(h.host.lookupMessage, "Stopped after 1 of 3 film files.");
  assert.deepEqual([h.host.cancelLookupRequested, h.host.lookupBusy, h.host.lookupProgress], [false, false, null]);
  assert.deepEqual(h.statuses.at(-1), { status: "ready", done: 1, total: 3, label: "films", message: "Stopped after 1 of 3 film files.", sourceRoot: "/lib" });
  const flagged = harness();
  flagged.lookup.stopFilenameLookup();
  assert.equal(flagged.host.cancelLookupRequested, true);
  const reset = harness({ over: { cancelLookupRequested: true, lastLookupStopped: true } });
  await reset.lookup.findMovieMatches([file("/lib/A.mkv")], "/lib");
  assert.deepEqual([reset.host.lastLookupStopped, reset.host.lookupMessage.startsWith("Checked 1 film file")], [false, true]);
});

test("findSeriesMatches does nothing when it cannot start and derives its asset root", async () => {
  const groups = [group("/lib/Show", ["S01E01.mkv"])];
  const preview = harness({ desktop: false });
  await preview.lookup.findSeriesMatches(groups, "/lib");
  const busy = harness({ over: { lookupBusy: true } });
  await busy.lookup.findSeriesMatches(groups, "/lib");
  const empty = harness();
  await empty.lookup.findSeriesMatches([], "/lib");
  assert.deepEqual([preview.calls.length, busy.calls.length, empty.calls.length], [0, 0, 0]);
  const given = harness();
  await given.lookup.findSeriesMatches(groups, "/given");
  const analysis = harness({ over: { scanResult: { root: "/media", input_kind: "folder" }, organizerSource: "analysis" } });
  await analysis.lookup.findSeriesMatches(groups);
  const scan = harness();
  await scan.lookup.findSeriesMatches(groups);
  const fallback = harness({ over: { organizerScan: null } });
  await fallback.lookup.findSeriesMatches(groups);
  assert.deepEqual([given, analysis, scan, fallback].map((h) => names(h, "ensureLocalAssets")[0][0]), ["/given", "/media", "/lib/scan", "/lib/Show"]);
  assert.deepEqual(names(given, "ensureLocalAssets")[0].slice(1), [groups[0].files, ["/lib/Show", "/lib/Show/Season 1"]]);
});

test("findSeriesMatches reports a failed nearby-artwork check", async () => {
  const groups = [group("/lib/Show", ["S01E01.mkv"])];
  const h = harness({ ensure: async () => null, over: { localAssetScanError: "denied" } });
  await h.lookup.findSeriesMatches(groups, "/lib");
  assert.deepEqual([h.host.lookupMessage, h.host.lookupMessageScope, h.host.lookupBusy, h.host.lookupProgress], ["Could not check nearby artwork: denied", "series folders", false, null]);
  assert.deepEqual(h.statuses.at(-1), { status: "error", done: 0, total: 1, label: "series", message: "Could not check nearby artwork: denied", sourceRoot: "/lib" });
});

test("findSeriesMatches skips ambiguous or empty groups and queue series that already have everything", async () => {
  const complete = { recordId: "tvdb:1", artworkCandidates: [art("p")], localSeasonPosterPaths: { 1: "https://poster/s1.jpg" }, certification: "TV-MA" };
  const groups = [group("/lib/A", ["S01E01.mkv"]), group("/lib/B", ["S01E01.mkv"], { ambiguous: true }), group("/lib/C", []), group("/lib/D", ["S01E01.mkv"])];
  const state = { over: { seriesAssignments: { "/lib/A": complete, "/lib/D": { ...complete, certification: " " } } } };
  const queued = harness(state);
  await queued.lookup.findSeriesMatches(groups, "/lib");
  assert.deepEqual(names(queued, "filenameClue"), [["D"]]);
  const folder = harness(state);
  await folder.lookup.findSeriesMatches(groups);
  assert.deepEqual(names(folder, "filenameClue"), [["A"], ["D"]]);
  for (const [label, patch] of [["local record", { recordId: "local-1" }], ["no artwork", { artworkCandidates: [{ imageUrl: "/x", thumbnailUrl: "" }] }], ["local season poster", { localSeasonPosterPaths: { 1: "/s1.jpg" } }]]) {
    const h = harness({ over: { seriesAssignments: { "/lib/A": { ...complete, ...patch } } } });
    await h.lookup.findSeriesMatches([groups[0]], "/lib");
    assert.equal(names(h, "filenameClue").length, 1, label);
  }
  // Every numbered season needs a provider poster, not just one of them.
  const twoSeasons = group("/lib/A", ["S01E01.mkv"], { seasonDirectories: [{ path: "/lib/A/Season 1", name: "Season 1", season: 1 }, { path: "/lib/A/Season 2", name: "Season 2", season: 2 }] });
  const partial = harness({ over: { seriesAssignments: { "/lib/A": { ...complete, localSeasonPosterPaths: { 1: "https://poster/s1.jpg" } } } } });
  await partial.lookup.findSeriesMatches([twoSeasons], "/lib");
  assert.equal(names(partial, "filenameClue").length, 1);
  const noSeasons = harness({ over: { seriesAssignments: { "/lib/A": { ...complete, localSeasonPosterPaths: { 3: "https://p/3.jpg" } } } } });
  await noSeasons.lookup.findSeriesMatches([group("/lib/A", ["S01E01.mkv"], { seasonDirectories: [] })], "/lib");
  assert.equal(names(noSeasons, "filenameClue").length, 0);
});

test("findSeriesMatches explains when no series needs a search", async () => {
  const complete = { recordId: "tvdb:1", artworkCandidates: [art("p")], localSeasonPosterPaths: { 1: "https://poster/s1.jpg" }, certification: "TV-MA" };
  const saved = harness({ over: { seriesAssignments: { "/lib/A": complete } } });
  await saved.lookup.findSeriesMatches([group("/lib/A", ["S01E01.mkv"])], "/lib");
  assert.deepEqual([saved.host.lookupMessage, saved.host.lookupMessageScope], ["Saved title suggestions or reviewed show details are already available. Open a series to compare other sources.", "series folders"]);
  assert.deepEqual(saved.statuses.at(-1), { status: "ready", done: 1, total: 1, label: "series", message: saved.host.lookupMessage, sourceRoot: "/lib" });
  const none = harness();
  await none.lookup.findSeriesMatches([group("/lib/B", ["S01E01.mkv"], { ambiguous: true })], "/lib");
  assert.equal(none.host.lookupMessage, "No show-specific series folders were recognized in this analysis.");
  const viaSuggestions = harness({ suggestions: { "/lib/C/S01E01.mkv": ready } });
  await viaSuggestions.lookup.findSeriesMatches([group("/lib/C", ["S01E01.mkv"], { ambiguous: true })], "/lib");
  assert.equal(viaSuggestions.host.lookupMessage, "Saved title suggestions or reviewed show details are already available. Open a series to compare other sources.");
});

test("findSeriesMatches looks up each folder, associates an exact match, loads provider details and reports every file", async () => {
  const show = result("81189", "Breaking Bad", "2008", "tmdb", "tv");
  const g = group("/lib/Breaking Bad", ["S01E01.mkv", "S01E02.mkv"]);
  const h = harness({ clues: { "Breaking Bad": clue("Breaking Bad", "2008") }, responses: { "Breaking Bad": { results: [show], warnings: ["w"], successfulProviders: ["tmdb"] } } });
  await h.lookup.findSeriesMatches([g], "/lib");
  assert.deepEqual([names(h, "searchPublicTitles"), names(h, "rank"), names(h, "autoAssignSeriesMatch"), names(h, "hydrate")], [[["tv", "Breaking Bad"]], [["tv"]], [["/lib/Breaking Bad", "81189"]], [["/lib/Breaking Bad", "81189"]]]);
  assert.deepEqual(h.host.seriesSuggestions["/lib/Breaking Bad"], { clue: clue("Breaking Bad", "2008"), results: [show], warnings: ["w"], status: "ready", error: "" });
  const reports = h.matches.filter(([, s]) => s.status === "ready");
  assert.deepEqual(reports.map(([path]) => path), g.files.map((f) => f.path));
  assert.deepEqual(reports[0][1], { kind: "tv", status: "ready", title: "Breaking Bad", year: "2008", provider: "tmdb", recordId: "tmdb:81189", sourceUrl: "https://src/81189", description: "Breaking Bad text", autoAssociated: true, artwork: candidatePreview(show), error: "" });
  assert.equal(h.matches.filter(([, s]) => s.status === "searching").length, 2);
  assert.deepEqual([h.host.lookupMessage, h.host.lookupMessageScope, h.statuses.at(-1).label], ["Checked 1 series folder. Exact title-and-year matches without a conflict were associated; review the rest.", "series folders", "series"]);
  assert.deepEqual(h.statuses.filter((s) => s.message.startsWith("Checked ")).map((s) => s.message).slice(0, 1), ["Checked 1 of 1 series."]);
});

test("findSeriesMatches prefers an exact TheTVDB match for details and falls back to the automatic match when it loads nothing", async () => {
  const tmdb = result("1", "Breaking Bad", "2008", "tmdb", "tv");
  const tvdb = result("81189", "Breaking Bad", "2008", "tvdb", "tv");
  const g = group("/lib/Breaking Bad", ["S01E01.mkv"]);
  const responses = { "Breaking Bad": { results: [tmdb, tvdb], warnings: [], successfulProviders: [] } };
  const clues = { "Breaking Bad": clue("Breaking Bad", "2008") };
  const preferred = harness({ clues, responses, over: { metadataSources: { tvdb: true }, tvdbCredentialStatus: { configured: true } } });
  await preferred.lookup.findSeriesMatches([g], "/lib");
  await tick();
  assert.equal(names(preferred, "hydrate")[0][1], "81189");
  const fallback = harness({ clues, responses, hydrate: async (root, row) => row.id !== "81189", over: { metadataSources: { tvdb: true }, tvdbCredentialStatus: { configured: true } } });
  await fallback.lookup.findSeriesMatches([g], "/lib");
  await tick(); await tick();
  const hydrated = names(fallback, "hydrate").map((call) => call[1]);
  const auto = automaticMetadataMatch([tmdb, tvdb], { ...clue("Breaking Bad", "2008"), kind: "tv" });
  assert.deepEqual(hydrated, ["81189", auto.id]);
  const plain = harness({ clues, responses });
  await plain.lookup.findSeriesMatches([g], "/lib");
  assert.equal(names(plain, "hydrate")[0][1], automaticMetadataMatch([tmdb, tvdb], { ...clue("Breaking Bad", "2008"), kind: "tv" }).id);
  const unconfigured = harness({ clues, responses, over: { metadataSources: { tvdb: true }, tvdbCredentialStatus: { configured: false } } });
  await unconfigured.lookup.findSeriesMatches([g], "/lib");
  assert.equal(names(unconfigured, "hydrate")[0][1], auto.id);
  const noMatch = harness({ responses: { Show: { results: [result("9", "Other", "2001", "tmdb", "tv")], warnings: [], successfulProviders: [] } } });
  await noMatch.lookup.findSeriesMatches([group("/lib/Show", ["S01E01.mkv"])], "/lib");
  assert.deepEqual([names(noMatch, "autoAssignSeriesMatch").length, names(noMatch, "hydrate").length, noMatch.matches.at(-1)[1].recordId], [0, 0, "tmdb:9"]);
});

test("findSeriesMatches records failures per folder, keeps going and reports an error only when all failed", async () => {
  const groups = [group("/lib/A", ["S01E01.mkv"]), group("/lib/B", ["S01E01.mkv"])];
  const mixed = harness({ clues: { A: new Error("engine") }, responses: { B: { results: [], warnings: [], successfulProviders: [] } } });
  await mixed.lookup.findSeriesMatches(groups, "/lib");
  assert.deepEqual(mixed.host.seriesSuggestions["/lib/A"], { clue: { title: "A", year: null, yearEnd: null, certification: null }, results: [], warnings: [], status: "error", error: "Error: engine" });
  assert.equal(mixed.matches.find(([path, s]) => path === "/lib/A/S01E01.mkv" && s.status === "error")[1].error, "Error: engine");
  assert.equal(mixed.host.lookupMessage, "Checked 2 series folders; 1 lookup failed. Exact title-and-year matches without a conflict were associated; review the rest.");
  assert.equal(mixed.statuses.at(-1).status, "ready");
  const all = harness({ responses: { A: new Error("429"), B: new Error("429") } });
  await all.lookup.findSeriesMatches(groups, "/lib");
  assert.equal(all.statuses.at(-1).status, "error");
  assert.equal(all.host.seriesSuggestions["/lib/A"].clue.title, "A");
});

test("findSeriesMatches stops when asked and resets its flags", async () => {
  const groups = [group("/lib/A", ["S01E01.mkv"]), group("/lib/B", ["S01E01.mkv"]), group("/lib/C", ["S01E01.mkv"])];
  let h;
  h = harness({ onSearch: () => h.lookup.stopFilenameLookup() });
  await h.lookup.findSeriesMatches(groups, "/lib");
  assert.deepEqual(names(h, "filenameClue"), [["A"]]);
  assert.equal(h.host.lastLookupStopped, true);
  assert.equal(h.host.lookupMessage, "Stopped after 1 of 3 series folders.");
  assert.deepEqual([h.host.cancelLookupRequested, h.host.lookupBusy, h.host.lookupProgress], [false, false, null]);
  const reset = harness({ over: { cancelLookupRequested: true, lastLookupStopped: true } });
  await reset.lookup.findSeriesMatches([groups[0]], "/lib");
  assert.deepEqual([reset.host.lastLookupStopped, reset.host.lookupMessage.startsWith("Checked 1 series folder")], [false, true]);
});
