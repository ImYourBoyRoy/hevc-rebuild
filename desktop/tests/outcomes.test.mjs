// ./desktop/tests/outcomes.test.mjs
// Characterization tests for the report outcome helpers extracted from the route (slice 5).
// Run: pnpm test:outcomes

import assert from "node:assert/strict";
import test from "node:test";
import { isFailure, isSuccess, outcomeLabel } from "../src/lib/outcomes.ts";

const labelled = {
  OK: "Succeeded",
  METADATA_OK: "Metadata copy succeeded",
  TEST_OK: "Short test succeeded",
  INFO: "Check finished",
  INTEGRITY_OK: "File is intact",
  CORRUPT: "File is damaged",
  UNREADABLE: "File could not be read",
  FAILED: "Failed",
  SKIPPED: "Skipped",
  INTERRUPTED: "Interrupted",
};

test("outcomeLabel maps every known status, ignoring case and surrounding space", () => {
  for (const [status, label] of Object.entries(labelled)) {
    assert.equal(outcomeLabel(status), label, status);
    assert.equal(outcomeLabel(` ${status.toLowerCase()} `), label, status.toLowerCase());
  }
});

test("outcomeLabel echoes unknown statuses and falls back for empty ones", () => {
  assert.equal(outcomeLabel("Weird_Status"), "Weird_Status");
  assert.equal(outcomeLabel(""), "Needs review");
  assert.equal(outcomeLabel(undefined), "Needs review");
  assert.equal(outcomeLabel("  "), "  ");
});

test("isSuccess is true only for the five success statuses", () => {
  for (const status of ["OK", "METADATA_OK", "TEST_OK", "INFO", "INTEGRITY_OK", " ok "]) assert.equal(isSuccess(status), true, status);
  for (const status of ["FAILED", "SKIPPED", "INTERRUPTED", "OKAY", "", undefined]) assert.equal(isSuccess(status), false, String(status));
});

test("isFailure recognises exact failure statuses and any FAIL/ERROR/CORRUPT substring", () => {
  for (const status of ["FAILED", "CORRUPT", "UNREADABLE", "ERROR", "failed", "Analysis failed", "tool error", "partly_corrupt", "FAILURE"]) {
    assert.equal(isFailure(status), true, status);
  }
  for (const status of ["OK", "SKIPPED", "INTERRUPTED", "INFO", "", undefined]) assert.equal(isFailure(status), false, String(status));
});

test("no known status is both a success and a failure", () => {
  for (const status of Object.keys(labelled)) assert.equal(isSuccess(status) && isFailure(status), false, status);
});
