// ./desktop/tests/queue-document.test.mjs
// Characterization tests for the pure queue document and list-edit helpers extracted from the route
// (queue slice A of the +page.svelte segregation). Run: pnpm test:queue-document

import assert from "node:assert/strict";
import test from "node:test";
import {
  buildQueueDocument,
  normalizeQueueSource,
  pruneAfterSourceRemoval,
  removeEntryFromSources,
  reorderSourceEntries,
  reorderSources,
  restoreExcludedFiles,
  scanResultWithQueueStatus,
  uniquePosterChoices,
  withoutQueuePath,
} from "../src/lib/queue-document.ts";

const entry = (path) => ({ path, name: path.split("/").at(-1), size_bytes: 1 });
const source = (root, paths = [], extra = {}) => ({
  root, status: "ready", error: null, updatedAtMs: 1,
  scanResult: paths.length ? { root, entries: paths.map(entry), candidate_count: paths.length } : null,
  changedFiles: [], missingFiles: [], excludedPaths: [], entryOrder: [], ...extra,
});

test("uniquePosterChoices drops empty, oversized and duplicate ids and keeps the first of each", () => {
  const a = { id: "a", n: 1 }, a2 = { id: "a", n: 2 };
  assert.deepEqual(uniquePosterChoices([a, { id: "" }, a2, { id: "x".repeat(1025) }, { id: "b" }]), [a, { id: "b" }]);
  assert.deepEqual(uniquePosterChoices([{ id: "x".repeat(1024) }]).length, 1);
  assert.deepEqual(uniquePosterChoices([]), []);
});

test("scanResultWithQueueStatus marks missing and changed files and leaves others unmarked", () => {
  assert.equal(scanResultWithQueueStatus(source("/r")), null);
  const result = scanResultWithQueueStatus(source("/r", ["/r/a", "/r/b", "/r/c"], { missingFiles: ["/r/a"], changedFiles: ["/r/b", "/r/a"] }));
  assert.deepEqual(result.entries.map((item) => item.queue_stale), ["missing", "changed", undefined]);
  assert.equal(result.candidate_count, 3);
});

test("normalizeQueueSource fills missing list fields only", () => {
  const normalized = normalizeQueueSource({ root: "/r", status: "ready" });
  assert.deepEqual([normalized.changedFiles, normalized.missingFiles, normalized.excludedPaths, normalized.entryOrder], [[], [], [], []]);
  const kept = normalizeQueueSource({ root: "/r", changedFiles: ["/r/a"], excludedPaths: ["/r/b"] });
  assert.deepEqual([kept.changedFiles, kept.excludedPaths], [["/r/a"], ["/r/b"]]);
});

test("buildQueueDocument copies state, de-duplicates poster choices and stamps the time", () => {
  const dup = [{ id: "p" }, { id: "p" }];
  const parts = {
    sources: [source("/r")], selectedPaths: ["/r/a"], fileTuneOverrides: { "/r/a": "animation" }, fileStreamOverrides: { "/r/a": [1] },
    movieReviews: { "/r/a": { title: "T", artworkCandidates: dup } }, seriesReviews: { "/r": { title: "S", artworkCandidates: dup } },
    matchSuggestions: { "/r/a": { status: "ready" } }, runStates: { "/r/a": { status: "failed" } }, activeRoot: "/r",
  };
  const document = buildQueueDocument(parts, 1234);
  assert.equal(document.version, 1);
  assert.equal(document.updatedAtMs, 1234);
  assert.equal(document.activeRoot, "/r");
  assert.deepEqual(document.movieReviews["/r/a"].artworkCandidates, [{ id: "p" }]);
  assert.deepEqual(document.seriesReviews["/r"].artworkCandidates, [{ id: "p" }]);
  assert.notEqual(document.selectedMediaPaths, parts.selectedPaths);
  assert.notEqual(document.runStates, parts.runStates);
  assert.equal(document.sources, parts.sources);
  assert.deepEqual(parts.movieReviews["/r/a"].artworkCandidates, dup);
});

test("reorderSources moves a source and reports no-ops as null", () => {
  const sources = [source("/a"), source("/b"), source("/c")];
  assert.deepEqual(reorderSources(sources, "/c", "/a").map((item) => item.root), ["/c", "/a", "/b"]);
  assert.deepEqual(reorderSources(sources, "/a", "/b").map((item) => item.root), ["/b", "/a", "/c"]);
  assert.equal(reorderSources(sources, "/a", "/a"), null);
  assert.equal(reorderSources(sources, "/a", "/missing"), null);
  assert.equal(reorderSources(sources, "/missing", "/a"), null);
});

test("reorderSourceEntries reorders one source, records entryOrder and candidate_count, and ignores others", () => {
  const sources = [source("/a", ["/a/1", "/a/2", "/a/3"]), source("/b", ["/b/1", "/b/2"])];
  const next = reorderSourceEntries(sources, "/a", "/a/3", "/a/1");
  assert.deepEqual(next[0].scanResult.entries.map((item) => item.path), ["/a/3", "/a/1", "/a/2"]);
  assert.deepEqual(next[0].entryOrder, ["/a/3", "/a/1", "/a/2"]);
  assert.equal(next[0].scanResult.candidate_count, 3);
  assert.equal(next[1], sources[1]);
  const unchanged = reorderSourceEntries(sources, "/a", "/a/1", "/a/missing");
  assert.deepEqual(unchanged[0].scanResult.entries.map((item) => item.path), ["/a/1", "/a/2", "/a/3"]);
  assert.deepEqual(reorderSourceEntries([source("/c")], "/c", "/c/1", "/c/2")[0].scanResult, null);
});

test("removeEntryFromSources hides the file by normalized key and records it as excluded", () => {
  const sources = [source("C:\\Media", ["C:\\Media\\A.mkv", "C:\\Media\\B.mkv"]), source("/other", ["/other/x"])];
  const { sources: next, removed } = removeEntryFromSources(sources, "c:/media/a.mkv");
  assert.equal(removed, true);
  assert.deepEqual(next[0].scanResult.entries.map((item) => item.path), ["C:\\Media\\B.mkv"]);
  assert.deepEqual(next[0].excludedPaths, ["c:/media/a.mkv"]);
  assert.deepEqual(next[0].entryOrder, ["C:\\Media\\B.mkv"]);
  assert.equal(next[0].scanResult.candidate_count, 1);
  assert.equal(next[1], sources[1]);
  const again = removeEntryFromSources(next, "c:/media/a.mkv");
  assert.equal(again.removed, false);
  assert.equal(removeEntryFromSources([source("/s", ["/s/a"], { excludedPaths: ["/s/old"] })], "/s/a").sources[0].excludedPaths.join(), "/s/old,/s/a");
  assert.equal(removeEntryFromSources([source("/s")], "/s/a").removed, false);
});

test("restoreExcludedFiles clears exclusions on the named source only", () => {
  const sources = [source("/a", ["/a/1"], { excludedPaths: ["/a/2"] }), source("/b", ["/b/1"], { excludedPaths: ["/b/2"] })];
  const next = restoreExcludedFiles(sources, "/a");
  assert.deepEqual(next[0].excludedPaths, []);
  assert.equal(next[1], sources[1]);
});

const parts = () => ({
  selectedPaths: ["/r/a", "/r/b"], fileTuneOverrides: { "/r/a": "animation", "/r/b": "none" }, fileStreamOverrides: { "/r/a": [1], "/r/b": [2] },
  runStates: { "/r/a": { status: "failed" }, "/r/b": { status: "completed" } }, matchSuggestions: { "/r/a": {}, "/r/b": {} }, movieReviews: { "/r/a": {}, "/r/b": {} },
});

test("withoutQueuePath matches Windows paths by normalized key", () => {
  const next = withoutQueuePath("c:/media/a.mkv", {
    selectedPaths: ["C:\\Media\\A.mkv", "C:\\Media\\B.mkv"], fileTuneOverrides: { "C:\\Media\\A.mkv": "none" }, fileStreamOverrides: {},
    runStates: {}, matchSuggestions: {}, movieReviews: { "C:\\Media\\A.mkv": {}, "C:\\Media\\B.mkv": {} },
  });
  assert.deepEqual(next.selectedPaths, ["C:\\Media\\B.mkv"]);
  assert.deepEqual(next.fileTuneOverrides, {});
  assert.deepEqual(Object.keys(next.movieReviews), ["C:\\Media\\B.mkv"]);
});

test("withoutQueuePath is case-sensitive on POSIX paths and drops exact matches everywhere", () => {
  const next = withoutQueuePath("/r/a", parts());
  assert.deepEqual(next.selectedPaths, ["/r/b"]);
  for (const key of ["fileTuneOverrides", "fileStreamOverrides", "runStates", "matchSuggestions", "movieReviews"]) assert.deepEqual(Object.keys(next[key]), ["/r/b"], key);
  assert.deepEqual(Object.keys(next).sort(), ["fileStreamOverrides", "fileTuneOverrides", "matchSuggestions", "movieReviews", "runStates", "selectedPaths"]);
});

test("pruneAfterSourceRemoval keeps only records for files still in the queue", () => {
  const remaining = [source("/keep", ["/keep/a"])];
  const next = pruneAfterSourceRemoval("/gone", remaining, {
    ...parts(), selectedPaths: ["/keep/a", "/gone/x"], fileTuneOverrides: { "/keep/a": "none", "/gone/x": "none" }, fileStreamOverrides: { "/gone/x": [1] },
    runStates: { "/keep/a": { status: "failed" }, "/gone/x": { status: "failed" } }, matchSuggestions: { "/gone/x": {} }, movieReviews: { "/keep/a": {}, "/gone/x": {} },
    seriesReviews: {},
  });
  assert.deepEqual(next.selectedPaths, ["/keep/a"]);
  assert.deepEqual(Object.keys(next.fileTuneOverrides), ["/keep/a"]);
  assert.deepEqual(Object.keys(next.fileStreamOverrides), []);
  assert.deepEqual(Object.keys(next.runStates), ["/keep/a"]);
  assert.deepEqual(Object.keys(next.matchSuggestions), []);
  assert.deepEqual(Object.keys(next.movieReviews), ["/keep/a"]);
});

test("pruneAfterSourceRemoval drops series reviews that belonged only to the removed source", () => {
  const review = (...paths) => ({ episodeAssignments: Object.fromEntries(paths.map((path) => [path, {}])) });
  const remaining = [source("/keep", ["/keep/Show/e1.mkv", "/gone/Show2/e1.mkv"])];
  const next = pruneAfterSourceRemoval("/gone", remaining, {
    ...parts(),
    seriesReviews: {
      "/gone/Show1": review("/gone/Show1/e1.mkv"),
      "/gone/Show2": review(),
      "/keep/Show": review("/keep/Show/e1.mkv"),
      "/elsewhere/Show": review("/elsewhere/Show/e1.mkv"),
    },
  });
  assert.deepEqual(Object.keys(next.seriesReviews).sort(), ["/elsewhere/Show", "/gone/Show2", "/keep/Show"]);
});
