// ./desktop/tests/queue-run-state.test.mjs
// Characterization tests for the queue run-state helpers extracted from the route (slice 5).
// Run: pnpm test:queue-run-state

import assert from "node:assert/strict";
import test from "node:test";
import { makeQueueRunState, queueRunStatusLabel, runProgressStatus } from "../src/lib/queue-run-state.ts";

test("queueRunStatusLabel labels every queue status", () => {
  const expected = {
    queued: "Queued", running: "In active batch", scanning: "Scanning", renaming: "Renaming",
    remuxing: "Copying streams", rebuilding: "Rebuilding video", validating: "Validating output",
    renamed: "Renamed", completed: "Completed", failed: "Failed", skipped: "Skipped",
    interrupted: "Interrupted", needs_review: "Review before retry",
  };
  for (const [status, label] of Object.entries(expected)) assert.equal(queueRunStatusLabel(status), label, status);
});

test("runProgressStatus classifies phase text, validation first, then muxing, then encoding", () => {
  const cases = [
    ["Validating output", "rebuild", "validating"],
    ["Verified output", "rebuild", "validating"],
    ["Output installed", "rebuild", "validating"],
    ["Transaction commit", "rebuild", "validating"],
    ["Remuxing streams", "rebuild", "remuxing"],
    ["Metadata embedding", "rebuild", "remuxing"],
    ["Copying streams", "rebuild", "remuxing"],
    ["Encode video", "rebuild", "rebuilding"],
    // The engine's real phase text (found by a grep of the Rust sources, not guessed):
    ["Encoding video", "rebuild", "rebuilding"],
    ["Starting the video encode", "rebuild", "rebuilding"],
    ["Retrying the encode with fewer threads", "rebuild", "rebuilding"],
    ["Calibrating the encoder", "rebuild", "rebuilding"],
    ["Comparing picture quality with the source", "rebuild", "validating"],
    ["Validating video, streams, metadata, and HDR", "rebuild", "validating"],
    ["Analyzing streams", "rebuild", "scanning"],
    ["Calibration", "rebuild", "rebuilding"],
    ["HDR10+ metadata extraction", "rebuild", "rebuilding"],
    ["Finalizing", "rebuild", "rebuilding"],
    ["Analysis", "rebuild", "scanning"],
    ["Reading media tracks", "rebuild", "scanning"],
    ["Storage preflight", "rebuild", "scanning"],
    ["Anything", "check", "scanning"],
    ["Something unknown", "rebuild", "running"],
    ["Something unknown", null, "running"],
  ];
  for (const [phase, mode, status] of cases) assert.equal(runProgressStatus(phase, mode), status, `${phase} / ${mode}`);
});

test("runProgressStatus lets validation and encode text win over check mode", () => {
  assert.equal(runProgressStatus("Validating output", "check"), "validating");
  assert.equal(runProgressStatus("Encode", "check"), "rebuilding");
});

// Regression: the native engine reports "Encoding", "Encoding video" and "Re-encoding". The matcher
// used to look for the substring "encode", which none of them contain, so a file that was mid-encode
// showed as merely "running" (tasklist T10.17). It now matches "encod".
test("runProgressStatus recognises the native engine's Encoding phases", () => {
  for (const phase of ["Encoding", "Encoding video", "Re-encoding"]) {
    assert.equal(runProgressStatus(phase, "rebuild"), "rebuilding", phase);
  }
  assert.equal(runProgressStatus("Analyzing streams", "rebuild"), "scanning");
});

test("makeQueueRunState fills defaults, truncates the message and stamps the time", () => {
  const before = Date.now();
  const state = makeQueueRunState("completed", "rebuild");
  assert.deepEqual({ ...state, updatedAtMs: 0 }, {
    status: "completed", mode: "rebuild", message: null, outputPath: null, reportPath: null, reportStatus: null, updatedAtMs: 0,
  });
  assert.ok(state.updatedAtMs >= before && state.updatedAtMs <= Date.now());
  const full = makeQueueRunState("failed", null, { message: "m", outputPath: "/o", reportPath: "/r", reportStatus: "FAILED" });
  assert.deepEqual({ ...full, updatedAtMs: 0 }, {
    status: "failed", mode: null, message: "m", outputPath: "/o", reportPath: "/r", reportStatus: "FAILED", updatedAtMs: 0,
  });
  assert.equal(makeQueueRunState("failed", null, { message: "x".repeat(2000) }).message.length, 1024);
  assert.equal(makeQueueRunState("failed", null, { message: "" }).message, null);
});

test("makeQueueRunState truncates by code point so emoji are never split", () => {
  const message = makeQueueRunState("failed", null, { message: "😀".repeat(1100) }).message;
  assert.equal([...message].length, 1024);
  assert.ok(!message.includes("�"));
});
