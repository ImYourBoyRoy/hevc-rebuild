// ./desktop/tests/report-view.test.mjs
// Pure-function contract for report presentation helpers (no DOM, no IPC).
// Run: pnpm test:report-view

import assert from "node:assert/strict";
import test from "node:test";
import { optimizeSummary } from "../src/lib/report-view.ts";

test("copy rows say the video stream was kept and show the reason", () => {
  const summary = optimizeSummary({
    input: "a.mkv", status: "OK", message: "",
    upload_mode: "optimize", optimize_action: "copy",
    optimize_reason: "healthy 1440p HEVC", video_payload_identical: "True",
  });
  assert.deepEqual(summary, {
    label: "Video copied untouched",
    detail: "healthy 1440p HEVC",
    verified: "Packets verified identical",
  });
});

test("encode rows report the sampled SSIM with four decimals", () => {
  const summary = optimizeSummary({
    input: "b.mkv", status: "OK", message: "",
    upload_mode: "optimize", optimize_action: "encode",
    optimize_reason: "oversized", quality_ssim_min: "0.99123",
  });
  assert.equal(summary?.label, "Re-encoded smaller");
  assert.equal(summary?.verified, "Sampled SSIM 0.9912");
});

test("encode rows without a quality value do not invent one", () => {
  const summary = optimizeSummary({
    input: "c.mkv", status: "OK", message: "",
    upload_mode: "optimize", optimize_action: "encode",
  });
  assert.equal(summary?.verified, undefined);
});

test("upscale rows and legacy reports have no optimize summary", () => {
  assert.equal(optimizeSummary({ input: "d.mkv", status: "OK", message: "", upload_mode: "upscale" }), null);
  assert.equal(optimizeSummary({ input: "e.mkv", status: "OK", message: "" }), null);
});

test("an unknown action is surfaced rather than hidden", () => {
  const summary = optimizeSummary({
    input: "f.mkv", status: "OK", message: "",
    optimize_action: "surprise",
  });
  assert.equal(summary?.label, "Optimize: surprise");
});
