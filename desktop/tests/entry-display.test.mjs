// ./desktop/tests/entry-display.test.mjs
// Characterization tests for the pure entry display helpers extracted from the route (helper slice):
// profile and media-info lines, durations, HDR10+ signals and consent text, metadata value formatting,
// status labels, stream classification and the animation hint. Run: pnpm test:entry-display

import assert from "node:assert/strict";
import test from "node:test";
import {
  clockDuration, formatMetadataValue, hasHdr10plusSignal, mediaInfoLine, metadataSuggestsAnimation, profileSummary, statusLabel,
  streamIsRealVideo, streamIsSelectable, youtubeHdr10plusConsentMessage,
} from "../src/lib/entry-display.ts";
import { bytes } from "../src/lib/format.ts";

const hdr = (over = {}) => ({ dolby_vision: false, hdr10plus: false, hdr10plus_hint: false, hdr10: false, hlg: false, ...over });
const profile = (over = {}) => ({
  video: { width: 1920, height: 1080, source_bit_depth: 10, codec: "hevc", ...over.video }, hdr: hdr(over.hdr), duration_seconds: 5400, audio_count: 2, subtitle_count: 3,
  tracks: over.tracks ?? [], ...over.rest,
});
const entry = (over = {}) => ({ path: "/lib/a.mkv", name: "a.mkv", size_bytes: 1_500_000_000, profile: profile(over), ...over.entry });

test("profileSummary shows resolution, depth and the strongest HDR label, or that the profile is missing", () => {
  assert.equal(profileSummary({ profile: null }), "Profile unavailable");
  assert.equal(profileSummary(entry()), "1920 × 1080 · 10-bit · SDR");
  const cases = [
    [{ dolby_vision: true, hdr10plus: true }, "Dolby Vision"], [{ hdr10plus: true, hdr10: true }, "HDR10+"], [{ hdr10plus_hint: true, hdr10: true }, "HDR10+ candidate"],
    [{ hdr10: true, hlg: true }, "HDR10"], [{ hlg: true }, "HLG"], [{}, "SDR"],
  ];
  for (const [flags, label] of cases) assert.equal(profileSummary(entry({ hdr: flags })), `1920 × 1080 · 10-bit · ${label}`);
});

test("hasHdr10plusSignal is true for a confirmed signal or a hint only", () => {
  assert.equal(hasHdr10plusSignal(entry({ hdr: { hdr10plus: true } })), true);
  assert.equal(hasHdr10plusSignal(entry({ hdr: { hdr10plus_hint: true } })), true);
  assert.equal(hasHdr10plusSignal(entry({ hdr: { hdr10: true } })), false);
  assert.equal(hasHdr10plusSignal({ profile: null }), false);
});

test("youtubeHdr10plusConsentMessage distinguishes a detected signal from an unconfirmed hint", () => {
  assert.match(youtubeHdr10plusConsentMessage(entry({ hdr: { hdr10plus: true } })), /^HDR10\+ was detected and hdr10plus_tool is missing/);
  assert.match(youtubeHdr10plusConsentMessage(entry({ hdr: { hdr10plus_hint: true } })), /^HDR10\+ is indicated but not confirmed from frame metadata/);
  assert.match(youtubeHdr10plusConsentMessage({ profile: null }), /^HDR10\+ is indicated but not confirmed/);
});

test("clockDuration formats minutes:seconds or h:mm:ss and clamps negatives", () => {
  assert.equal(clockDuration(0), "0:00");
  assert.equal(clockDuration(59.6), "1:00");
  assert.equal(clockDuration(125), "2:05");
  assert.equal(clockDuration(3600), "1:00:00");
  assert.equal(clockDuration(5415), "1:30:15");
  assert.equal(clockDuration(-30), "0:00");
});

test("mediaInfoLine joins the available facts and falls back to the file size without a profile", () => {
  assert.equal(mediaInfoLine({ profile: null, size_bytes: 2048 }), bytes(2048));
  const full = mediaInfoLine(entry({ hdr: { hdr10: true } }));
  assert.equal(full, `hevc · 1920×1080 · 10-bit · 1:30:00 · 2 audio · 3 subtitles · HDR10 · ${bytes(1_500_000_000)}`);
  const sparse = mediaInfoLine(entry({ video: { width: 0, height: 0, source_bit_depth: 0, codec: "" }, rest: { duration_seconds: 0 } }));
  assert.equal(sparse, `Video · 2 audio · 3 subtitles · ${bytes(1_500_000_000)}`);
  assert.match(mediaInfoLine(entry({ hdr: { dolby_vision: true, hdr10: true } })), / · Dolby Vision · /);
  assert.match(mediaInfoLine(entry({ hdr: { hdr10plus: true } })), / · HDR10\+ · /);
  assert.match(mediaInfoLine(entry({ hdr: { hlg: true } })), / · HLG · /);
});

test("formatMetadataValue prints dashes, primitives and pretty JSON, tolerating odd values", () => {
  assert.equal(formatMetadataValue(null), "—");
  assert.equal(formatMetadataValue(undefined), "—");
  assert.equal(formatMetadataValue("text"), "text");
  assert.equal(formatMetadataValue(42), "42");
  assert.equal(formatMetadataValue(false), "false");
  assert.equal(formatMetadataValue({ a: 1 }), '{\n  "a": 1\n}');
  const circular = {}; circular.self = circular;
  assert.equal(formatMetadataValue(circular), "[object Object]");
  function named() {}
  assert.equal(formatMetadataValue(named), String(named));
});

test("statusLabel rewrites the three internal statuses and passes others through", () => {
  assert.deepEqual(["Ready", "Backed up", "Tools required", "Skipped"].map(statusLabel), ["Ready", "Has backup", "Tools needed", "Skipped"]);
});

test("streamIsSelectable accepts only the five stream types", () => {
  for (const type of ["video", "audio", "subtitle", "attachment", "data"]) assert.equal(streamIsSelectable(type), true);
  assert.equal(streamIsSelectable("other"), false);
});

test("streamIsRealVideo rejects cover art, empty frames, other types and unknown indexes", () => {
  const tracks = [
    { index: 0, type: "video", width: 1920, height: 1080, attached_picture: false },
    { index: 1, type: "video", width: 600, height: 600, attached_picture: true },
    { index: 2, type: "video", width: 0, height: 0 },
    { index: 3, type: "audio" },
  ];
  const item = entry({ tracks });
  assert.deepEqual([0, 1, 2, 3, 9].map((index) => streamIsRealVideo(item, index)), [true, false, false, false, false]);
  assert.equal(streamIsRealVideo({ profile: null }, 0), false);
});

test("metadataSuggestsAnimation matches animation genres as whole words only", () => {
  for (const genres of ["Animation, Comedy", "Anime", "animated", "Cartoon", "Drama, cartoons"]) assert.equal(metadataSuggestsAnimation({ genresText: genres }), true, genres);
  for (const genres of ["Drama", "Animatronics", "", undefined]) assert.equal(metadataSuggestsAnimation({ genresText: genres }), false, String(genres));
  assert.equal(metadataSuggestsAnimation(undefined), false);
});
