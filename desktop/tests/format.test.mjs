// ./desktop/tests/format.test.mjs
// Characterization tests for the display formatters extracted from the route (slice 5).
// Run: pnpm test:format

import assert from "node:assert/strict";
import test from "node:test";
import { bytes, duration } from "../src/lib/format.ts";

const GiB = 1024 ** 3;

test("bytes formats binary units with no decimals below MB and one decimal from MB up", () => {
  assert.equal(bytes(0), "0 B");
  assert.equal(bytes(1023), "1023 B");
  assert.equal(bytes(1024), "1 KB");
  assert.equal(bytes(1536), "2 KB");
  assert.equal(bytes(1024 ** 2), "1.0 MB");
  assert.equal(bytes(1.5 * 1024 ** 2), "1.5 MB");
  assert.equal(bytes(GiB), "1.0 GB");
  assert.equal(bytes(1024 ** 4), "1.0 TB");
  assert.equal(bytes(2048 * 1024 ** 4), "2048.0 TB");
});

test("bytes shows an em dash for missing or non-finite values", () => {
  assert.equal(bytes(null), "—");
  assert.equal(bytes(undefined), "—");
  assert.equal(bytes(Number.NaN), "—");
  assert.equal(bytes(Infinity), "—");
});

test("duration rolls seconds into minutes and hours and drops zero parts", () => {
  assert.equal(duration(0), "0s");
  assert.equal(duration(59), "59s");
  assert.equal(duration(59.5), "1m 0s");
  assert.equal(duration(60), "1m 0s");
  assert.equal(duration(61), "1m 1s");
  assert.equal(duration(3599), "59m 59s");
  assert.equal(duration(3600), "1h");
  assert.equal(duration(3660), "1h 1m");
  assert.equal(duration(3661), "1h 1m");
  assert.equal(duration(7325), "2h 2m");
});

test("duration shows an em dash for missing, negative or non-finite values", () => {
  assert.equal(duration(undefined), "—");
  assert.equal(duration(-1), "—");
  assert.equal(duration(Number.NaN), "—");
  assert.equal(duration(Infinity), "—");
});
