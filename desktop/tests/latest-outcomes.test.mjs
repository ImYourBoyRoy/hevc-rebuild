// ./desktop/tests/latest-outcomes.test.mjs
// Contract for which failures still count (src/lib/latest-outcomes.ts): a file is failed only when its most
// recent decisive outcome is a failure. Older failures are superseded by a later success; a read-only check,
// a skip or an interruption never clears a failure. Run: pnpm test:latest-outcomes
import assert from "node:assert/strict";
import test from "node:test";
import { latestFailureRows } from "../src/lib/latest-outcomes.ts";

const row = (input, status, message = "") => ({ input, status, message });
const report = (created, rows, name = `r${created}`) => ({ csv_path: `/logs/${name}.csv`, log_path: null, created, counts: {}, rows });

test("no reports, or only successes, mean no failures", () => {
  assert.deepEqual(latestFailureRows([]), []);
  assert.deepEqual(latestFailureRows([report(1, [row("/a.mkv", "OK")]), report(2, [row("/b.mkv", "METADATA_OK")])]), []);
});

test("repeated failures of one file count once, as its newest failure", () => {
  const reports = [1, 2, 3, 4, 5].map((n) => report(n, [row("/clips/doctor.mkv", "FAILED", `attempt ${n}`)]));
  const result = latestFailureRows(reports);
  assert.equal(result.length, 1);
  assert.equal(result[0].message, "attempt 5");
});

test("a later success supersedes an earlier failure, and a later failure supersedes an earlier success", () => {
  assert.deepEqual(latestFailureRows([report(1, [row("/a.mkv", "FAILED")]), report(2, [row("/a.mkv", "OK")])]), []);
  assert.deepEqual(latestFailureRows([report(1, [row("/a.mkv", "FAILED")]), report(2, [row("/a.mkv", "TEST_OK")])]), []);
  const back = latestFailureRows([report(1, [row("/a.mkv", "OK")]), report(2, [row("/a.mkv", "FAILED", "regressed")])]);
  assert.deepEqual(back.map((r) => r.message), ["regressed"]);
});

test("a read-only check, a skip or an interruption never clears a failure", () => {
  for (const status of ["INFO", "SKIPPED", "INTERRUPTED", "SOMETHING_NEW"]) {
    const result = latestFailureRows([report(1, [row("/a.mkv", "FAILED", "still broken")]), report(2, [row("/a.mkv", status)])]);
    assert.deepEqual(result.map((r) => r.message), ["still broken"], status);
  }
});

test("an intact integrity check clears a damaged file but not a failed rebuild", () => {
  assert.deepEqual(latestFailureRows([report(1, [row("/a.mkv", "CORRUPT")]), report(2, [row("/a.mkv", "INTEGRITY_OK")])]), []);
  assert.deepEqual(latestFailureRows([report(1, [row("/a.mkv", "UNREADABLE")]), report(2, [row("/a.mkv", "INTEGRITY_OK")])]), []);
  const kept = latestFailureRows([report(1, [row("/a.mkv", "FAILED", "rebuild failed")]), report(2, [row("/a.mkv", "INTEGRITY_OK")])]);
  assert.deepEqual(kept.map((r) => r.message), ["rebuild failed"]);
  assert.deepEqual(latestFailureRows([report(1, [row("/a.mkv", "FAILED")]), report(2, [row("/a.mkv", "INTEGRITY_OK")]), report(3, [row("/a.mkv", "OK")])]), []);
});

test("files are independent of each other", () => {
  const result = latestFailureRows([report(1, [row("/a.mkv", "FAILED", "a"), row("/b.mkv", "FAILED", "b")]), report(2, [row("/a.mkv", "OK")])]);
  assert.deepEqual(result.map((r) => r.message), ["b"]);
});

test("the order the reports are listed in does not matter, only their creation time", () => {
  const reports = [report(1, [row("/a.mkv", "FAILED", "old")]), report(2, [row("/a.mkv", "OK")]), report(3, [row("/b.mkv", "FAILED", "newest")])];
  assert.deepEqual(latestFailureRows(reports).map((r) => r.message), ["newest"]);
  assert.deepEqual(latestFailureRows([...reports].reverse()).map((r) => r.message), ["newest"]);
});

test("within one report the later row for a file wins", () => {
  assert.deepEqual(latestFailureRows([report(1, [row("/a.mkv", "FAILED"), row("/a.mkv", "OK")])]), []);
  assert.deepEqual(latestFailureRows([report(1, [row("/a.mkv", "OK"), row("/a.mkv", "FAILED", "last")])]).map((r) => r.message), ["last"]);
});

test("failure rows are returned newest first as the original objects, and rows without a path are never merged", () => {
  const first = row("/a.mkv", "FAILED", "a"), second = row("/b.mkv", "FAILED", "b"), blank1 = row("", "FAILED", "blank one"), blank2 = row("", "FAILED", "blank two");
  const result = latestFailureRows([report(1, [first, blank1]), report(2, [second, blank2])]);
  assert.deepEqual(result.map((r) => r.message), ["b", "blank two", "a", "blank one"].filter((m) => ["b", "blank two", "a", "blank one"].includes(m)));
  assert.ok(result.includes(first) && result.includes(second));
});

test("the real Nov 2-3 history (five Doctor Sleep failures and one synthetic failure) is two open failures", () => {
  const doctor = "/home/v1x0r/Desktop/AI/Hevc_rebuild/hevc-rebuild/examples/clips/Doctor_Sleep_2019_15s.mkv";
  const reports = [
    report(100, [row(doctor, "FAILED", "not verified")]), report(101, [row(doctor, "FAILED", "not verified")]), report(102, [row(doctor, "FAILED", "libx265")]),
    report(103, [row(doctor, "FAILED", "remux")]), report(104, [row(doctor, "FAILED", "cross-device")]),
    report(200, [row("/tmp/Test_Synthetic_HDR10.mkv", "FAILED", "implausibly small")]), report(150, [row("/media/other.mkv", "OK")]),
  ];
  const result = latestFailureRows(reports);
  assert.equal(result.length, 2);
  assert.deepEqual(result.map((r) => r.message), ["implausibly small", "cross-device"]);
});
