// ./desktop/tests/queue-persistence.test.mjs
// Characterization tests for the queue persistence controller extracted from the route (queue slice A),
// using an injected fake `invoke` as the tasklist requires. Run: pnpm test:queue-persistence

import assert from "node:assert/strict";
import test from "node:test";
import { createQueuePersistence } from "../src/lib/queue-persistence.ts";

function harness({ desktop = true, loaded = true, invoke } = {}) {
  const calls = [];
  const queue = { sources: [{ root: "/r" }], selectedPaths: ["/r/a"], runStates: {}, activeRoot: "/r", loaded, persistenceError: "" };
  const records = { tunes: { "/r/a": "animation" }, streams: { "/r/a": [1] }, movies: {}, series: {}, suggestions: {} };
  const fake = invoke ?? (async (command, args) => { calls.push({ command, args }); return null; });
  const persistence = createQueuePersistence({
    invoke: invoke ? async (command, args) => { calls.push({ command, args }); return invoke(command, args); } : fake,
    isDesktop: () => desktop,
    queue,
    getFileTuneOverrides: () => records.tunes,
    getFileStreamOverrides: () => records.streams,
    getMovieReviews: () => records.movies,
    getSeriesReviews: () => records.series,
    getMatchSuggestions: () => records.suggestions,
  });
  return { calls, queue, records, persistence };
}

test("nothing is written in browser preview or before the queue has loaded", async () => {
  for (const options of [{ desktop: false }, { loaded: false }]) {
    const { calls, persistence } = harness(options);
    assert.equal(await persistence.persistQueueSources(), false);
    await persistence.persistQueueSelection();
    await persistence.persistQueueMovieReviews({});
    await persistence.persistQueueSeriesReviews({});
    await persistence.persistQueueMetadataSuggestions();
    persistence.scheduleQueueMetadataSuggestionSave();
    assert.deepEqual(calls, []);
  }
});

test("persistQueueSources saves the whole document, clears the error and resolves true", async () => {
  const { calls, queue, persistence } = harness();
  queue.persistenceError = "old";
  assert.equal(await persistence.persistQueueSources(), true);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].command, "save_library_queue");
  assert.equal(calls[0].args.queue.version, 1);
  assert.deepEqual(calls[0].args.queue.selectedMediaPaths, ["/r/a"]);
  assert.deepEqual(calls[0].args.queue.fileTuneOverrides, { "/r/a": "animation" });
  assert.equal(queue.persistenceError, "");
});

test("persistQueueSources reports a failed save as false and keeps the message", async () => {
  const { queue, persistence } = harness({ invoke: async () => { throw new Error("disk full"); } });
  assert.equal(await persistence.persistQueueSources(), false);
  assert.equal(queue.persistenceError, "Error: disk full");
});

test("persistQueueRunStates merges the updates into the queue before saving", async () => {
  const { calls, queue, persistence } = harness();
  queue.runStates = { "/r/a": { status: "queued" } };
  assert.equal(await persistence.persistQueueRunStates({ "/r/b": { status: "failed" } }), true);
  assert.deepEqual(Object.keys(queue.runStates), ["/r/a", "/r/b"]);
  assert.deepEqual(Object.keys(calls[0].args.queue.runStates), ["/r/a", "/r/b"]);
});

test("persistQueueSelection defaults to deep copies of the live selection and overrides", async () => {
  const { calls, records, persistence } = harness();
  await persistence.persistQueueSelection();
  const sent = calls[0].args;
  assert.equal(calls[0].command, "save_library_queue_selection");
  assert.deepEqual(sent, { selectedMediaPaths: ["/r/a"], activeRoot: "/r", fileTuneOverrides: { "/r/a": "animation" }, fileStreamOverrides: { "/r/a": [1] } });
  assert.notEqual(sent.fileStreamOverrides, records.streams);
  assert.notEqual(sent.fileStreamOverrides["/r/a"], records.streams["/r/a"]);
  await persistence.persistQueueSelection(["/x"], null, {}, {});
  assert.deepEqual(calls[1].args, { selectedMediaPaths: ["/x"], activeRoot: null, fileTuneOverrides: {}, fileStreamOverrides: {} });
});

test("review saves use the native command names and argument keys", async () => {
  const { calls, persistence } = harness();
  await persistence.persistQueueMovieReviews({ "/r/a": { title: "T" } });
  await persistence.persistQueueSeriesReviews({ "/r": { title: "S" } });
  assert.deepEqual(calls, [
    { command: "save_library_queue_movie_reviews", args: { movieReviews: { "/r/a": { title: "T" } } } },
    { command: "save_library_queue_series_reviews", args: { seriesReviews: { "/r": { title: "S" } } } },
  ]);
});

test("persistQueueMetadataSuggestions saves a snapshot, not the live object", async () => {
  const { calls, records, persistence } = harness();
  records.suggestions = { "/r/a": { status: "ready" } };
  await persistence.persistQueueMetadataSuggestions();
  assert.equal(calls[0].command, "save_library_queue_metadata_suggestions");
  assert.deepEqual(calls[0].args, { matchSuggestions: { "/r/a": { status: "ready" } } });
  assert.notEqual(calls[0].args.matchSuggestions, records.suggestions);
});

test("a failed targeted save records the error without throwing and the next save still runs", async () => {
  let fail = true;
  const { calls, queue, persistence } = harness({ invoke: async () => { if (fail) throw new Error("boom"); } });
  await persistence.persistQueueMovieReviews({});
  assert.equal(queue.persistenceError, "Error: boom");
  fail = false;
  await persistence.persistQueueSeriesReviews({});
  assert.equal(queue.persistenceError, "");
  assert.equal(calls.length, 2);
});

test("writes are serialized in call order even when an earlier one is slow", async () => {
  const order = [];
  const { persistence } = harness({
    invoke: async (command) => {
      await new Promise((resolve) => setTimeout(resolve, command === "save_library_queue" ? 30 : 0));
      order.push(command);
    },
  });
  await Promise.all([persistence.persistQueueSources(), persistence.persistQueueSelection(), persistence.persistQueueMovieReviews({})]);
  assert.deepEqual(order, ["save_library_queue", "save_library_queue_selection", "save_library_queue_movie_reviews"]);
});

test("scheduleQueueMetadataSuggestionSave debounces to one save after three seconds and an explicit save cancels it", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { calls, persistence } = harness();
  persistence.scheduleQueueMetadataSuggestionSave();
  t.mock.timers.tick(2900);
  persistence.scheduleQueueMetadataSuggestionSave();
  t.mock.timers.tick(2900);
  assert.equal(calls.length, 0);
  t.mock.timers.tick(100);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(calls.length, 1);
  persistence.scheduleQueueMetadataSuggestionSave();
  await persistence.persistQueueMetadataSuggestions();
  t.mock.timers.tick(5000);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(calls.length, 2);
});
